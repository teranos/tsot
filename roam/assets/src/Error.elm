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
across every layer of roam (Elm decode failures, JS catches, FFI
envelope `errors` field, Rust panic envelopes).

CONVERGENCE NOTE: This is roam's local copy. TSOT's
`assets/src/Error.elm` carries the canonical implementation upstream.
Diffs against TSOT's version should be limited to the CSS class prefix
(`roam-error` vs `tsot-error`).

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


type alias Anchor =
    { x : Float
    , y : Float
    }


key : Error -> String
key e =
    e.id



-- DECODER ------------------------------------------------------------


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


clickAnchorDecoder : D.Decoder Anchor
clickAnchorDecoder =
    D.map2 Anchor
        (D.field "clientX" D.float)
        (D.field "clientY" D.float)


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


type alias ViewConfig msg =
    { onDismiss : String -> msg
    , onDragStart : String -> DragOffset -> msg
    , position : Maybe Anchor
    , viewport : { w : Float, h : Float }
    , buildLabel : Maybe String
    }


view : ViewConfig msg -> Error -> Html msg
view cfg e =
    let
        positionAttrs =
            case cfg.position of
                Just dragged ->
                    [ A.class "roam-error-anchored"
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
        ([ class "roam-error"
         , class ("roam-error--" ++ severityClass e.severity)
         , A.attribute "data-error-id" e.id
         , A.attribute "data-surface" e.context.surface
         , E.stopPropagationOn "click" (D.succeed ( cfg.onDismiss "", True ))
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
        , div [ class "roam-error-body" ]
            (viewField "error:" e.title [ class "roam-error-title" ]
                :: viewField "why:" e.why [ class "roam-error-why" ]
                :: viewTrace e.trace
                ++ viewRaw e.raw
            )
        , viewBuildFooter cfg.buildLabel
        ]


viewBuildFooter : Maybe String -> Html msg
viewBuildFooter maybeLabel =
    case maybeLabel of
        Just label ->
            div [ class "roam-error-buildfooter" ] [ text label ]

        Nothing ->
            text ""


cornerAnchorAttrs :
    Anchor
    -> { w : Float, h : Float }
    -> List (Html.Attribute msg)
cornerAnchorAttrs cursor viewport =
    let
        approxBoxW =
            448

        approxBoxH =
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
    [ A.class "roam-error-anchored"
    , horizontalStyle
    , verticalStyle
    ]


viewTitlebar : ViewConfig msg -> Error -> Html msg
viewTitlebar cfg e =
    div
        [ class "roam-error-titlebar"
        , E.on "mousedown" (D.map (cfg.onDragStart e.id) dragOffsetDecoder)
        ]
        [ span [ class "roam-error-titlebar-label" ]
            [ text (severityLabel e.severity ++ " — " ++ e.context.surface) ]
        , button
            [ class "roam-error-titlebar-close"
            , A.attribute "aria-label" "close"
            , E.onClick (cfg.onDismiss e.id)
            , E.stopPropagationOn "mousedown" (D.succeed ( cfg.onDismiss "", True ))
            ]
            [ text "\u{00D7}" ]
        ]


viewField : String -> String -> List (Html.Attribute msg) -> Html msg
viewField label value valueAttrs =
    div [ class "roam-error-field" ]
        [ span [ class "roam-error-label" ] [ text label ]
        , span ([ class "roam-error-value" ] ++ valueAttrs) [ text value ]
        ]


viewTrace : List String -> List (Html msg)
viewTrace trace =
    if List.isEmpty trace then
        []

    else
        [ div [ class "roam-error-field roam-error-field--trace" ]
            [ span [ class "roam-error-label" ] [ text "trace:" ]
            , div [ class "roam-error-trace" ]
                (List.map
                    (\line ->
                        div [ class "roam-error-trace-line" ] [ text line ]
                    )
                    trace
                )
            ]
        ]


viewRaw : Maybe String -> List (Html msg)
viewRaw raw =
    case raw of
        Just sample ->
            [ div [ class "roam-error-field roam-error-field--raw" ]
                [ span [ class "roam-error-label" ] [ text "raw:" ]
                , Html.pre [ class "roam-error-raw" ] [ text sample ]
                ]
            ]

        Nothing ->
            []


viewLogLine : Error -> Html msg
viewLogLine e =
    div [ class "roam-error-log-line" ]
        [ span [ class ("roam-error-tag roam-error--" ++ severityClass e.severity) ]
            [ text (severityLabel e.severity) ]
        , text " "
        , span [ class "roam-error-log-surface" ] [ text e.context.surface ]
        , text " — "
        , span [ class "roam-error-log-title" ] [ text e.title ]
        , text " · "
        , span [ class "roam-error-log-why" ] [ text e.why ]
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


styles : Html msg
styles =
    node "style" [] [ text errorCss ]


errorCss : String
errorCss =
    """
    .roam-error {
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

    .roam-error-titlebar {
      position: relative;
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.2rem 1.6rem 0.2rem 0.55rem;
      border-bottom: 1px solid #4a1414;
      user-select: none;
      cursor: move;
    }
    .roam-error-anchored {
      position: fixed;
    }

    .roam-error--info .roam-error-titlebar { background: linear-gradient(#1a1a3a, #0e0e22); }
    .roam-error--warn .roam-error-titlebar { background: linear-gradient(#3a2a0a, #1f1605); }
    .roam-error--error .roam-error-titlebar { background: linear-gradient(#3a0a0a, #1f0505); }
    .roam-error--panic .roam-error-titlebar { background: linear-gradient(#3a0a3a, #1f051f); }

    .roam-error-titlebar-label {
      font-weight: bold;
      font-size: 0.7rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
    }
    .roam-error--info .roam-error-titlebar-label { color: #aaf; }
    .roam-error--warn .roam-error-titlebar-label { color: #fc6; }
    .roam-error--error .roam-error-titlebar-label { color: #f88; }
    .roam-error--panic .roam-error-titlebar-label { color: #f8f; }

    .roam-error-titlebar-close {
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
    .roam-error-titlebar-close:hover { background: #6a1c1c; color: #fff; }
    .roam-error-titlebar-close:active { background: #2a0c0c; }

    .roam-error-body {
      padding: 0.5rem 0.7rem;
      display: flex;
      flex-direction: column;
      gap: 0.35rem;
      overflow-x: auto;
      max-height: 50vh;
      overflow-y: auto;
    }
    .roam-error-field {
      display: flex;
      gap: 0.5rem;
      align-items: baseline;
    }
    .roam-error-field--trace,
    .roam-error-field--raw {
      align-items: flex-start;
      flex-direction: column;
      gap: 0.2rem;
    }
    .roam-error-label {
      color: #888;
      font-weight: bold;
      min-width: 3.5rem;
      flex: 0 0 auto;
    }
    .roam-error-value {
      color: #ddd;
      white-space: pre-wrap;
      overflow-wrap: break-word;
      flex: 1 1 auto;
      min-width: 0;
    }
    .roam-error--info .roam-error-title { color: #aaf; }
    .roam-error--warn .roam-error-title { color: #fc6; }
    .roam-error--error .roam-error-title { color: #f88; }
    .roam-error--panic .roam-error-title { color: #f8f; }

    .roam-error-why { color: #ddd; }

    .roam-error-buildfooter {
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

    .roam-error-trace {
      width: 100%;
      color: #aaa;
      font-size: 0.7rem;
      padding-left: 0.6rem;
      border-left: 1px solid #4a1414;
      margin-top: 0.1rem;
    }
    .roam-error-trace-line {
      white-space: pre-wrap;
      overflow-wrap: break-word;
    }
    .roam-error-raw {
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

    .roam-error-log-line {
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.7rem;
      padding: 0.15rem 0;
      color: #ddd;
    }
    .roam-error-tag {
      display: inline-block;
      min-width: 3.5rem;
      font-weight: bold;
      text-align: center;
      padding: 0 0.3rem;
      border-radius: 2px;
      background: rgba(255, 255, 255, 0.05);
    }
    .roam-error-tag.roam-error--info { color: #88f; }
    .roam-error-tag.roam-error--warn { color: #fc6; }
    .roam-error-tag.roam-error--error { color: #f66; }
    .roam-error-tag.roam-error--panic { color: #f0f; }
    .roam-error-log-surface { color: #888; }
    .roam-error-log-title { color: #ddd; }
    .roam-error-log-why { color: #aaa; }
    """
