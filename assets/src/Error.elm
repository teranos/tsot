module Error exposing
    ( Anchor
    , Context
    , DragOffset
    , Error
    , Severity(..)
    , ViewConfig
    , clickAnchorDecoder
    , decode
    , dragOffsetDecoder
    , key
    , styles
    , view
    , viewLogLine
    )

{-| The Error primitive — single source of truth for rendering failures
across every layer of the dev tool (Elm decode failures, JS catches,
FFI envelope `errors` field, Rust panic envelopes). Stands alongside
`Card.elm` as a first-class primitive per `ERROR.md`.

The contract from `CLAUDE.md` + `ERROR.md`:

  - **One type, one render.** Every layer that emits a failure
    produces an `Error`. Every surface that displays a failure goes
    through `Error.view`. No bespoke "show this red" divs.
  - **Overlay at the originating surface.** The render is a terminal-
    style block anchored at the surface where the failure happened —
    `context.surface` + `context.region`. NOT a side drawer.
  - **Dark red background, monospace, severity ribbon, structured
    content.** See `ERROR.md` § Visual contract for the full spec.

The Error type carries all the context the developer needs to act on
the failure without opening browser devtools:

  - `id`: stable identity across re-renders + persistence.
  - `severity`: Info / Warn / Error / Panic.
  - `context`: which surface + region surfaced this — used for
    placement.
  - `title`: one-line summary the developer reads first.
  - `why`: the cause chain — for a decode failure the field path +
    expected vs got; for an FFI failure the call + arg; for a Lua
    error the cards/ filename + line.
  - `trace`: the structured `TraceEvent` chain leading up to the
    failure (drained from the OBSERVABILITY bus at emission time).
  - `raw`: optional sample of the failing payload — useful for decode
    failures where the developer wants to see what came in.
  - `at`: ISO-8601 or `Δms` timestamp.

-}

import Html exposing (Html, button, div, node, span, text)
import Html.Attributes as A exposing (class)
import Html.Events as E
import Json.Decode as D


type alias Error =
    { id : String
    , severity : Severity
    , context : Context
    , title : String
    , why : String
    , trace : List String
    , raw : Maybe String
    , at : String
    }


type Severity
    = Info
    | Warn
    | LevelError
    | Panic


type alias Context =
    { surface : String
    , region : Maybe String
    , anchor : Maybe Anchor
    }


{-| Pixel position the overlay anchors at, captured from
`MouseEvent.clientX/clientY` at the moment the failing action fired.
Developer mental model from ERROR.md § Visual contract: *"I click on
something, it doesn't work, error right there where my cursor is."*
The Error opens AT this position rather than at the surface's
bounding box, so the read is local to the interaction.

`Nothing` for port-triggered failures (async payload decode failures
where no cursor was involved) — those fall back to surface-bounding-
box anchoring.
-}
type alias Anchor =
    { x : Float
    , y : Float
    }


{-| Stable identity for keyed render. Same role as `Card.key`. The
same Error in the LOG mirror and at the contextual overlay maps to
the SAME DOM node — a re-render doesn't destroy + reconstruct it.
-}
key : Error -> String
key e =
    e.id



-- DECODER ------------------------------------------------------------


{-| Decode the typed Error from any cross-boundary envelope. The wire
shape is the same on the JS side (port payloads, FFI envelope's
`errors` field, panic envelopes) and (forthcoming Slice 4) on the
Rust side via serde — bytes-compatible.
-}
decode : D.Decoder Error
decode =
    D.succeed makeError
        |> required "id" D.string
        |> required "severity" decodeSeverity
        |> required "context" decodeContext
        |> required "title" D.string
        |> required "why" D.string
        |> optionalListField "trace" D.string
        |> optionalStringField "raw"
        |> required "at" D.string


required : String -> D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
required field aDec fDec =
    D.map2 (\f a -> f a) fDec (D.field field aDec)


optionalListField : String -> D.Decoder a -> D.Decoder (List a -> b) -> D.Decoder b
optionalListField field aDec fDec =
    D.map2 (\f a -> f a)
        fDec
        (D.oneOf [ D.field field (D.list aDec), D.succeed [] ])


optionalStringField : String -> D.Decoder (Maybe String -> b) -> D.Decoder b
optionalStringField field fDec =
    D.map2 (\f a -> f a)
        fDec
        (D.oneOf [ D.field field (D.map Just D.string), D.succeed Nothing ])


makeError :
    String
    -> Severity
    -> Context
    -> String
    -> String
    -> List String
    -> Maybe String
    -> String
    -> Error
makeError id_ sev ctx title_ why_ trace_ raw_ at_ =
    { id = id_
    , severity = sev
    , context = ctx
    , title = title_
    , why = why_
    , trace = trace_
    , raw = raw_
    , at = at_
    }


decodeSeverity : D.Decoder Severity
decodeSeverity =
    D.string
        |> D.andThen
            (\s ->
                case String.toLower s of
                    "info" ->
                        D.succeed Info

                    "warn" ->
                        D.succeed Warn

                    "warning" ->
                        D.succeed Warn

                    "error" ->
                        D.succeed LevelError

                    "panic" ->
                        D.succeed Panic

                    other ->
                        D.fail ("Error.decodeSeverity: unknown severity " ++ other)
            )


decodeContext : D.Decoder Context
decodeContext =
    D.map3 Context
        (D.field "surface" D.string)
        (D.oneOf
            [ D.field "region" (D.map Just D.string)
            , D.succeed Nothing
            ]
        )
        (D.oneOf
            [ D.field "anchor" (D.map Just decodeAnchor)
            , D.succeed Nothing
            ]
        )


decodeAnchor : D.Decoder Anchor
decodeAnchor =
    D.map2 Anchor
        (D.field "x" D.float)
        (D.field "y" D.float)


{-| Decoder you can attach to a DOM event (typically `click`) via
`Html.Events.on "click" clickAnchorDecoder` so the Msg constructor
receives the cursor position. The shape mirrors `MouseEvent`'s
`clientX`/`clientY`. Call site:

    onClickCapturingAnchor : (Anchor -> msg) -> Html.Attribute msg
    onClickCapturingAnchor toMsg =
        Html.Events.on "click" (D.map toMsg Error.clickAnchorDecoder)

-}
clickAnchorDecoder : D.Decoder Anchor
clickAnchorDecoder =
    D.map2 Anchor
        (D.field "clientX" D.float)
        (D.field "clientY" D.float)


{-| Cursor position WITHIN the event target — the grab-offset for
drag. `MouseEvent.offsetX/Y` reports the click coordinates relative
to the target element's padding-edge. For a titlebar click, that's
the cursor's position inside the titlebar, which (since the titlebar
sits at the top of the box) is approximately the cursor's position
inside the box.

Used by `ErrorDragStarted` to capture the grab-offset directly from
the DOM event — avoids having to know where the box actually
rendered (which depends on the corner-flip logic).
-}
type alias DragOffset =
    { offsetX : Float
    , offsetY : Float
    }


dragOffsetDecoder : D.Decoder DragOffset
dragOffsetDecoder =
    D.map2 DragOffset
        (D.field "offsetX" D.float)
        (D.field "offsetY" D.float)



-- VIEW ---------------------------------------------------------------


{-| Render config. Mirrors `Card.Config`'s polymorphic-msg pattern so
the parent wires its concrete Msg in:

  - `onDismiss errorId` — close button click
  - `onDragStart errorId anchor` — titlebar mousedown captures the
    cursor position so the parent can compute the drag offset
  - `position` — when present, overrides `error.context.anchor`
    (used to render the dragged position). When `Nothing`, the
    renderer falls back to `error.context.anchor` (the cursor where
    the failure occurred).
  - `viewport` — used to decide which CORNER of the box the cursor
    anchors to. Classic-OS context-menu behavior: open down-right
    normally; flip to down-left when click is near the right edge,
    up-right when near the bottom edge, up-left in the bottom-right
    corner. The box itself is never shifted away from the cursor —
    only the choice of which corner the cursor sits on.
-}
type alias ViewConfig msg =
    { onDismiss : String -> msg
    , onDragStart : String -> DragOffset -> msg
    , position : Maybe Anchor
    , viewport : { w : Float, h : Float }
    , -- Optional build watermark. Per developer mental model the
      -- error window should carry build/commit info as a tiny
      -- minifooter so it's immediately obvious WHICH build
      -- produced the failure. `Nothing` = no footer (build info
      -- not yet loaded). String is rendered verbatim.
      buildLabel : Maybe String
    }


{-| Canonical render — a classic-OS-style window overlay anchored at
the failing interaction. Per `ERROR.md` § Visual contract:

  - Titlebar at top, severity-tinted, with the failure label.
  - Square close button (×) pinned to the upper-right corner.
  - Titlebar is the drag handle (mousedown starts drag).
  - Body: `error: <title>` → `why: <reason>` → `trace: <chain>`.
  - Anchor: spawns AT the cursor where the failure originated. No
    clamping; off-screen overflow recovered via drag.

`onDismiss` wires the close button. `onDragStart` wires the titlebar
mousedown for drag. Per ERROR.md Slice 6: dismissal is a state
transition on the same DOM element, not destroy + reconstruct.
-}
view : ViewConfig msg -> Error -> Html msg
view cfg e =
    let
        positionAttrs =
            case cfg.position of
                Just dragged ->
                    -- User dragged the box; use the exact position
                    -- they put it at. No flip — respect their choice.
                    [ A.class "tsot-error-anchored"
                    , A.style "left" (String.fromFloat dragged.x ++ "px")
                    , A.style "top" (String.fromFloat dragged.y ++ "px")
                    ]

                Nothing ->
                    case e.context.anchor of
                        Just cursor ->
                            cornerAnchorAttrs cursor cfg.viewport

                        Nothing ->
                            []
    in
    div
        ([ class "tsot-error"
         , class ("tsot-error--" ++ severityClass e.severity)
         , A.attribute "data-error-id" e.id
         , A.attribute "data-surface" e.context.surface
         , -- Clicks inside the overlay must NOT bubble to the global
           -- `Browser.Events.onClick` cursor capture — otherwise
           -- clicking inside the overlay would re-anchor the next
           -- error to the click point inside this overlay. The empty
           -- string id is a no-op in the parent's update (no live
           -- Error has id "").
           E.stopPropagationOn "click" (D.succeed ( cfg.onDismiss "", True ))
         ]
            ++ (case e.context.region of
                    Just r ->
                        [ A.attribute "data-region" r ]

                    Nothing ->
                        []
               )
            ++ positionAttrs
        )
        [ viewTitlebar cfg e
        , div [ class "tsot-error-body" ]
            (viewField "error:" e.title [ class "tsot-error-title" ]
                :: viewField "why:" e.why [ class "tsot-error-why" ]
                :: viewTrace e.trace
                ++ viewRaw e.raw
            )
        , viewBuildFooter cfg.buildLabel
        ]


{-| Tiny build/commit minifooter so the developer immediately knows
which build produced the failure. Per the developer mental model:
errors are sacred, and "what build was running" is a sacred bit of
context every error carries with it.
-}
viewBuildFooter : Maybe String -> Html msg
viewBuildFooter maybeLabel =
    case maybeLabel of
        Just label ->
            div [ class "tsot-error-buildfooter" ] [ text label ]

        Nothing ->
            text ""


{-| Decide which corner of the box the cursor sits on so the box
opens INTO the viewport. Classic-OS context-menu behavior:

  - Down-right (normal): cursor at the box's top-LEFT corner —
    emit `left = cursor.x + 8; top = cursor.y + 8`.
  - Down-left (cursor near right edge): cursor at top-RIGHT corner —
    emit `right = viewport.w - cursor.x + 8; top = cursor.y + 8`.
  - Up-right (cursor near bottom edge): cursor at bottom-LEFT corner —
    emit `left = cursor.x + 8; bottom = viewport.h - cursor.y + 8`.
  - Up-left (bottom-right corner of viewport): cursor at bottom-RIGHT
    corner — emit `right = ...; bottom = ...`.

`approxBoxW`/`approxBoxH` estimate the rendered size for the flip
decision. The box itself sizes via `.tsot-error` CSS (`width: 28rem`,
content-driven height). These constants match closely enough that
the flip fires when the box WOULD overflow.

The cursor is always on a corner of the box; the box never shifts
AWAY from the cursor. Only the direction it opens changes.
-}
cornerAnchorAttrs :
    Anchor
    -> { w : Float, h : Float }
    -> List (Html.Attribute msg)
cornerAnchorAttrs cursor viewport =
    let
        approxBoxW =
            -- .tsot-error width is 28rem at 16px root = 448px.
            448

        approxBoxH =
            -- Conservative minimum so we flip when there's not
            -- enough room for even a minimal box below the cursor.
            -- Real boxes can be taller; user can drag if needed.
            120

        offset =
            8

        flipHorizontal =
            cursor.x + offset + approxBoxW > viewport.w

        flipVertical =
            cursor.y + offset + approxBoxH > viewport.h

        horizontalStyle =
            if flipHorizontal then
                A.style "right"
                    (String.fromFloat (viewport.w - cursor.x + offset) ++ "px")

            else
                A.style "left" (String.fromFloat (cursor.x + offset) ++ "px")

        verticalStyle =
            if flipVertical then
                A.style "bottom"
                    (String.fromFloat (viewport.h - cursor.y + offset) ++ "px")

            else
                A.style "top" (String.fromFloat (cursor.y + offset) ++ "px")
    in
    [ A.class "tsot-error-anchored"
    , horizontalStyle
    , verticalStyle
    ]


{-| Classic-OS titlebar — severity-tinted bar across the top with the
window label, a square × close button pinned to the upper-right
corner, and the whole bar acting as the drag handle (mousedown
starts drag). Close-button mousedown stops propagation so it
doesn't also trigger a drag.
-}
viewTitlebar : ViewConfig msg -> Error -> Html msg
viewTitlebar cfg e =
    div
        [ class "tsot-error-titlebar"
        , E.on "mousedown" (D.map (cfg.onDragStart e.id) dragOffsetDecoder)
        ]
        [ span [ class "tsot-error-titlebar-label" ]
            [ text (severityLabel e.severity ++ " — " ++ e.context.surface) ]
        , button
            [ class "tsot-error-titlebar-close"
            , A.attribute "aria-label" "close"
            , E.onClick (cfg.onDismiss e.id)
            , -- Stop mousedown bubbling so the close button doesn't
              -- ALSO start a drag on the titlebar behind it.
              E.stopPropagationOn "mousedown" (D.succeed ( cfg.onDismiss "", True ))
            ]
            [ text "\u{00D7}" ]
        ]




viewField : String -> String -> List (Html.Attribute msg) -> Html msg
viewField label value valueAttrs =
    div [ class "tsot-error-field" ]
        [ span [ class "tsot-error-label" ] [ text label ]
        , span ([ class "tsot-error-value" ] ++ valueAttrs) [ text value ]
        ]


viewTrace : List String -> List (Html msg)
viewTrace trace =
    if List.isEmpty trace then
        []

    else
        [ div [ class "tsot-error-field tsot-error-field--trace" ]
            [ span [ class "tsot-error-label" ] [ text "trace:" ]
            , div [ class "tsot-error-trace" ]
                (List.map
                    (\line ->
                        div [ class "tsot-error-trace-line" ] [ text line ]
                    )
                    trace
                )
            ]
        ]


viewRaw : Maybe String -> List (Html msg)
viewRaw raw =
    case raw of
        Just sample ->
            [ div [ class "tsot-error-field tsot-error-field--raw" ]
                [ span [ class "tsot-error-label" ] [ text "raw:" ]
                , Html.pre [ class "tsot-error-raw" ] [ text sample ]
                ]
            ]

        Nothing ->
            []






{-| Stripped historical mirror for the LOG drawer. Same content but
inline, no overlay positioning, no dismiss affordance. Used for
recovering past errors in a session timeline. NOT for live attention
— that's `view`.
-}
viewLogLine : Error -> Html msg
viewLogLine e =
    div [ class "tsot-error-log-line" ]
        [ span [ class ("tsot-error-tag tsot-error--" ++ severityClass e.severity) ]
            [ text (severityLabel e.severity) ]
        , text " "
        , span [ class "tsot-error-log-surface" ] [ text e.context.surface ]
        , text " — "
        , span [ class "tsot-error-log-title" ] [ text e.title ]
        , text " · "
        , span [ class "tsot-error-log-why" ] [ text e.why ]
        ]


severityClass : Severity -> String
severityClass s =
    case s of
        Info ->
            "info"

        Warn ->
            "warn"

        LevelError ->
            "error"

        Panic ->
            "panic"


severityLabel : Severity -> String
severityLabel s =
    case s of
        Info ->
            "info"

        Warn ->
            "warn"

        LevelError ->
            "error"

        Panic ->
            "PANIC"



-- STYLES -------------------------------------------------------------


{-| All Error-internal CSS — the visual contract of the primitive.
Returned as a `<style>` element that Main mounts once at the top of
the page (same pattern as `Card.styles`). The CSS travels with the
module; new components that adopt the Error primitive don't need to
copy any styling.
-}
styles : Html msg
styles =
    node "style" [] [ text errorCss ]


errorCss : String
errorCss =
    """
    /* --- Error overlay -------------------------------------------- */
    /* Classic-OS-style window: titlebar + body. Anchor position is
       caller-driven (position:absolute by default; the cursor-
       anchored case overrides to position:fixed via inline style). */
    .tsot-error {
      position: absolute;
      z-index: 1000;
      min-width: 18rem;
      width: 28rem;
      max-width: min(32rem, calc(100vw - 1rem));
      background: #2a0c0c;
      border: 1px solid #4a1414;
      border-radius: 4px;
      box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
      color: #ddd;
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.75rem;
      line-height: 1.4;
      display: flex;
      flex-direction: column;
      overflow: hidden;
    }

    /* Classic-OS titlebar — severity-tinted band across the top.
       `position: relative` so the close-button's absolute corner
       positioning anchors here. Cursor `move` signals the drag
       affordance (titlebar is the drag handle). */
    .tsot-error-titlebar {
      position: relative;
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.2rem 1.6rem 0.2rem 0.55rem;
      border-bottom: 1px solid #4a1414;
      user-select: none;
      cursor: move;
    }
    /* Cursor-anchored variant — position:fixed. The actual edge
       (left vs right, top vs bottom) is set by Elm inline so the
       cursor sits on whichever box corner has room. Classic-OS
       context-menu behavior. */
    .tsot-error-anchored {
      position: fixed;
    }

    .tsot-error--info .tsot-error-titlebar { background: linear-gradient(#1a1a3a, #0e0e22); }
    .tsot-error--warn .tsot-error-titlebar { background: linear-gradient(#3a2a0a, #1f1605); }
    .tsot-error--error .tsot-error-titlebar { background: linear-gradient(#3a0a0a, #1f0505); }
    .tsot-error--panic .tsot-error-titlebar { background: linear-gradient(#3a0a3a, #1f051f); }

    .tsot-error-titlebar-label {
      font-weight: bold;
      font-size: 0.7rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
    }
    .tsot-error--info .tsot-error-titlebar-label { color: #aaf; }
    .tsot-error--warn .tsot-error-titlebar-label { color: #fc6; }
    .tsot-error--error .tsot-error-titlebar-label { color: #f88; }
    .tsot-error--panic .tsot-error-titlebar-label { color: #f8f; }

    /* Square close button pinned to the upper-right corner of the
       titlebar — classic-OS style, flush with the window frame. */
    .tsot-error-titlebar-close {
      position: absolute;
      top: 3px;
      right: 3px;
      width: 1.1rem;
      height: 1.1rem;
      padding: 0;
      line-height: 1;
      font-size: 0.85rem;
      font-family: inherit;
      color: #ddd;
      background: #4a1414;
      border: 1px solid #6a1c1c;
      border-radius: 2px;
      cursor: pointer;
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
    .tsot-error-titlebar-close:hover { background: #6a1c1c; color: #fff; }
    .tsot-error-titlebar-close:active { background: #2a0c0c; }

    .tsot-error-body {
      padding: 0.5rem 0.7rem;
      display: flex;
      flex-direction: column;
      gap: 0.35rem;
      overflow-x: auto;
      max-height: 50vh;
      overflow-y: auto;
    }
    .tsot-error-field {
      display: flex;
      gap: 0.5rem;
      align-items: baseline;
    }
    .tsot-error-field--trace,
    .tsot-error-field--raw {
      align-items: flex-start;
      flex-direction: column;
      gap: 0.2rem;
    }
    .tsot-error-label {
      color: #888;
      font-weight: bold;
      min-width: 3.5rem;
      flex: 0 0 auto;
    }
    /* Value text wraps at WORD boundaries (no `word-break:break-word`,
       which forced one-char-per-line at the cursor-anchored width). */
    .tsot-error-value {
      color: #ddd;
      white-space: pre-wrap;
      overflow-wrap: break-word;
      flex: 1 1 auto;
      min-width: 0;
    }
    .tsot-error--info .tsot-error-title { color: #aaf; }
    .tsot-error--warn .tsot-error-title { color: #fc6; }
    .tsot-error--error .tsot-error-title { color: #f88; }
    .tsot-error--panic .tsot-error-title { color: #f8f; }

    .tsot-error-why { color: #ddd; }

    /* Build/commit watermark — tiny muted footer at the bottom
       edge so every error carries the build context that produced
       it. Per ERROR.md developer mental model. */
    .tsot-error-buildfooter {
      padding: 0.15rem 0.7rem 0.25rem;
      color: #555;
      font-size: 0.6rem;
      line-height: 1.2;
      border-top: 1px solid #2a0c0c;
      background: #1a0606;
      user-select: text;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .tsot-error-trace {
      width: 100%;
      color: #aaa;
      font-size: 0.7rem;
      padding-left: 0.6rem;
      border-left: 1px solid #4a1414;
      margin-top: 0.1rem;
    }
    .tsot-error-trace-line {
      white-space: pre-wrap;
      overflow-wrap: break-word;
    }
    .tsot-error-raw {
      width: 100%;
      color: #aaa;
      background: #1a0606;
      padding: 0.4rem 0.5rem;
      margin: 0;
      border-radius: 2px;
      overflow-x: auto;
      font-size: 0.7rem;
      white-space: pre-wrap;
      overflow-wrap: break-word;
    }

    /* --- LOG-mirror line (historical / drawer) -------------------- */
    .tsot-error-log-line {
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.7rem;
      padding: 0.15rem 0;
      color: #ddd;
    }
    .tsot-error-tag {
      display: inline-block;
      min-width: 3.5rem;
      font-weight: bold;
      text-align: center;
      padding: 0 0.3rem;
      border-radius: 2px;
      background: rgba(255, 255, 255, 0.05);
    }
    .tsot-error-tag.tsot-error--info { color: #88f; }
    .tsot-error-tag.tsot-error--warn { color: #fc6; }
    .tsot-error-tag.tsot-error--error { color: #f66; }
    .tsot-error-tag.tsot-error--panic { color: #f0f; }
    .tsot-error-log-surface { color: #888; }
    .tsot-error-log-title { color: #ddd; }
    .tsot-error-log-why { color: #aaa; }
    """
