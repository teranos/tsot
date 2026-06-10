module Error exposing
    ( Anchor
    , Context
    , Error
    , Severity(..)
    , clickAnchorDecoder
    , decode
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

import Html exposing (Html, div, node, span, text)
import Html.Attributes as A exposing (class)
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



-- VIEW ---------------------------------------------------------------


{-| Canonical render — the terminal-style overlay anchored at the
originating surface. Per `ERROR.md` § Visual contract:

  - Dark red background, monospace, severity ribbon on the left.
  - Content order: `error: <title>` → `why: <reason>` →
    `trace: <chain>` → dismiss affordance.
  - The caller's CSS positions this absolute / fixed to the surface
    (via `context.surface` + `context.region` data attributes that
    selector-driven container CSS reads). The Error doesn't position
    itself — placement is the caller's responsibility, render is the
    Error's.
-}
view : Error -> Html msg
view e =
    div
        ([ class "tsot-error"
         , class ("tsot-error--" ++ severityClass e.severity)
         , A.attribute "data-error-id" e.id
         , A.attribute "data-surface" e.context.surface
         ]
            ++ (case e.context.region of
                    Just r ->
                        [ A.attribute "data-region" r ]

                    Nothing ->
                        []
               )
            ++ anchorAttrs e.context.anchor
        )
        [ div [ class "tsot-error-ribbon" ] []
        , div [ class "tsot-error-body" ]
            (viewField "error:" e.title [ class "tsot-error-title" ]
                :: viewField "why:" e.why [ class "tsot-error-why" ]
                :: viewTrace e.trace
                ++ viewRaw e.raw
                ++ [ viewDismiss ]
            )
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


viewDismiss : Html msg
viewDismiss =
    div [ class "tsot-error-dismiss" ] [ text "dismiss [esc]" ]


{-| Per ERROR.md § Visual contract — primary anchor case is the
cursor position captured at the failing interaction. When present we
switch to `position: fixed` rooted in the viewport at the click point;
when absent we fall back to the default `position: absolute` and let
the caller's surface container position the overlay.

Small offset (+8/+8 px) keeps the overlay from sitting *under* the
cursor, which would leave the user dragging the pointer through their
own diagnostic.
-}
anchorAttrs : Maybe Anchor -> List (Html.Attribute msg)
anchorAttrs maybeAnchor =
    case maybeAnchor of
        Just a ->
            [ A.style "position" "fixed"
            , A.style "left" (String.fromFloat (a.x + 8) ++ "px")
            , A.style "top" (String.fromFloat (a.y + 8) ++ "px")
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
    /* Position is caller-anchored: the surface that owns this Error
       wraps it in a container positioned relative + a child slot
       that this overlay fills. Default `position: absolute` reads
       from the nearest positioned ancestor; surfaces can override. */
    .tsot-error {
      position: absolute;
      z-index: 1000;
      max-width: 32rem;
      background: #2a0c0c;
      border: 1px solid #4a1414;
      border-radius: 4px;
      box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);
      color: #ddd;
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.75rem;
      line-height: 1.4;
      display: flex;
      align-items: stretch;
    }
    /* Severity ribbon — left edge stripe, full height. */
    .tsot-error-ribbon {
      width: 4px;
      flex: 0 0 auto;
      border-radius: 4px 0 0 4px;
    }
    .tsot-error--info .tsot-error-ribbon { background: #88f; }
    .tsot-error--warn .tsot-error-ribbon { background: #fc6; }
    .tsot-error--error .tsot-error-ribbon { background: #f66; }
    .tsot-error--panic .tsot-error-ribbon { background: #f0f; }

    .tsot-error-body {
      flex: 1 1 auto;
      padding: 0.6rem 0.8rem;
      display: flex;
      flex-direction: column;
      gap: 0.35rem;
      overflow-x: auto;
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
    .tsot-error-value {
      color: #ddd;
      white-space: pre-wrap;
      word-break: break-word;
    }
    /* The title takes the severity color so the eye finds the
       failure summary first. */
    .tsot-error--info .tsot-error-title { color: #aaf; }
    .tsot-error--warn .tsot-error-title { color: #fc6; }
    .tsot-error--error .tsot-error-title { color: #f88; }
    .tsot-error--panic .tsot-error-title { color: #f8f; }

    .tsot-error-why { color: #ddd; }

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
      word-break: break-word;
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
      word-break: break-word;
    }
    .tsot-error-dismiss {
      align-self: flex-end;
      color: #666;
      font-size: 0.65rem;
      cursor: pointer;
      user-select: none;
    }
    .tsot-error-dismiss:hover { color: #aaa; }

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
