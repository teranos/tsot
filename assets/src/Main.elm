port module Main exposing (main)

{-| H7-Elm Stage 3 — the dev-tool LOG panel is now rendered by Elm.

Elm owns:

  - the LOG container (`<div id="elm-log">`) inside the existing
    `.zone` wrapper at the LOG slot in play.html;
  - the rendering of every entry — plain text lines and styled error
    blocks (header, meta, breadcrumb trail, JS stack, raw stderr,
    aborted-module footer).

JS still owns:

  - parsing of TraceEvent variants (the `fmtTraceEvent` formatter
    stays in play.html and pre-formats trace entries into strings
    before pushing them to Elm);
  - the inline-error-near-button path (`renderErrorAt` + the original
    `buildErrorBlock`), which is a separate concern from the LOG;
  - the build-info footer source value (`window.__TSOT_BUILD__`).

Two inbound ports carry events into Elm:

    logTextIn  : (String -> msg) -> Sub msg
    logErrorIn : (D.Value -> msg) -> Sub msg

The matching JS shims live in `assets/js-bridge.js`:

    window.tsotLogPushText(line)
    window.tsotLogPushError(formatted)

Auto-scroll: after each log entry lands, `update` returns a Cmd that
sets the LOG container's vertical viewport to a large value; the DOM
clamps to actual scroll height — same UX as the original
`logEl.scrollTop = logEl.scrollHeight` behavior.

Errors-as-first-class: an error event carries source, message,
location, ffi_call, at_us, breadcrumb (pre-formatted strings), JS
stack, raw stderr; every field that's present is rendered. Nothing
collapsed, nothing truncated.

-}

import Browser
import Browser.Dom
import Html exposing (Html, div, pre, span, text)
import Html.Attributes exposing (class, id, style)
import Json.Decode as D
import Task



-- PORTS


port buildInfoIn : (D.Value -> msg) -> Sub msg


port logTextIn : (String -> msg) -> Sub msg


port logErrorIn : (D.Value -> msg) -> Sub msg



-- MODEL


type alias BuildInfo =
    { profile : String
    , builtAt : String
    , commit : String
    }


type BuildState
    = AwaitingPort
    | NoBuildInfo
    | HasBuildInfo BuildInfo


type alias ErrorEvent =
    { source : String
    , message : String
    , location : Maybe String
    , ffiCall : Maybe String
    , atUs : Maybe Float
    , breadcrumb : List String
    , jsStack : Maybe String
    , rawStderr : Maybe String
    }


type LogEntry
    = TextLine String
    | ErrorEntry ErrorEvent


type alias Model =
    { build : BuildState
    , log : List LogEntry
    }


{-| Stable id for the LOG scroll container. JS bridge could also call
`document.getElementById(logContainerId)` if it ever needs to peek; we
keep it as a constant so the Elm view + the auto-scroll Task agree.
-}
logContainerId : String
logContainerId =
    "elm-log"



-- MSG


type Msg
    = BuildInfoReceived D.Value
    | LogTextReceived String
    | LogErrorReceived D.Value
    | NoOp



-- INIT


init : () -> ( Model, Cmd Msg )
init _ =
    ( { build = AwaitingPort, log = [] }, Cmd.none )



-- UPDATE


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        BuildInfoReceived value ->
            case D.decodeValue decodeBuildInfo value of
                Ok info ->
                    ( { model | build = HasBuildInfo info }, Cmd.none )

                Err _ ->
                    ( { model | build = NoBuildInfo }, Cmd.none )

        LogTextReceived line ->
            ( { model | log = model.log ++ [ TextLine line ] }
            , scrollLogToBottom
            )

        LogErrorReceived value ->
            case D.decodeValue decodeErrorEvent value of
                Ok ev ->
                    ( { model | log = model.log ++ [ ErrorEntry ev ] }
                    , scrollLogToBottom
                    )

                Err err ->
                    -- An error envelope that doesn't decode is itself
                    -- information worth showing. Surface the decode
                    -- failure as a TextLine so the developer sees
                    -- something landed.
                    ( { model
                        | log =
                            model.log
                                ++ [ TextLine ("[log decode failed] " ++ D.errorToString err) ]
                      }
                    , scrollLogToBottom
                    )

        NoOp ->
            ( model, Cmd.none )


scrollLogToBottom : Cmd Msg
scrollLogToBottom =
    -- Setting viewport y to a large value; the DOM clamps to actual
    -- scroll height, same effect as `el.scrollTop = el.scrollHeight`.
    -- Wrapped in Task.attempt because Browser.Dom.setViewportOf
    -- returns a Task that fails if the element isn't mounted yet —
    -- ignored either way.
    Browser.Dom.setViewportOf logContainerId 0 1000000
        |> Task.attempt (\_ -> NoOp)



-- DECODERS


decodeBuildInfo : D.Decoder BuildInfo
decodeBuildInfo =
    D.map3 BuildInfo
        (D.field "profile" D.string)
        (D.field "builtAt" D.string)
        (D.field "commit" D.string)


decodeErrorEvent : D.Decoder ErrorEvent
decodeErrorEvent =
    D.map8 ErrorEvent
        (optionalField "source" D.string |> D.map (Maybe.withDefault "error"))
        (optionalField "message" D.string |> D.map (Maybe.withDefault "(no message)"))
        (optionalField "location" D.string)
        (optionalField "ffi_call" D.string)
        (optionalField "at_us" D.float)
        (optionalField "breadcrumb" (D.list D.string) |> D.map (Maybe.withDefault []))
        (optionalField "js_stack" D.string)
        (optionalField "raw_stderr" D.string)


optionalField : String -> D.Decoder a -> D.Decoder (Maybe a)
optionalField field decoder =
    D.maybe (D.field field decoder)



-- SUBSCRIPTIONS


subscriptions : Model -> Sub Msg
subscriptions _ =
    Sub.batch
        [ buildInfoIn BuildInfoReceived
        , logTextIn LogTextReceived
        , logErrorIn LogErrorReceived
        ]



-- VIEW


view : Model -> Html Msg
view model =
    div []
        [ viewLog model.log
        , viewBuildFooter model.build
        ]


viewLog : List LogEntry -> Html Msg
viewLog entries =
    pre
        [ id logContainerId
        , style "max-height" "24rem"
        , style "overflow-y" "auto"
        , style "font-size" "0.75rem"
        , style "color" "#aaa"
        , style "white-space" "pre"
        , style "margin" "0"
        ]
        (List.map viewEntry entries)


viewEntry : LogEntry -> Html Msg
viewEntry entry =
    case entry of
        TextLine line ->
            -- Newline preserved so consecutive TextLines stack vertically
            -- inside the <pre> the same way the original
            -- `logEl.appendChild(document.createTextNode(line + '\n'))`
            -- behaved.
            text (line ++ "\n")

        ErrorEntry ev ->
            viewErrorBlock ev


viewErrorBlock : ErrorEvent -> Html Msg
viewErrorBlock ev =
    div [ class "log-error" ]
        ([ div [ class "log-error-header" ]
            [ text ("[" ++ String.toUpper ev.source ++ "] " ++ ev.message) ]
         , div [ class "log-error-meta" ]
            [ text (formatErrorMeta ev) ]
         ]
            ++ viewBreadcrumb ev.breadcrumb
            ++ viewJsStack ev.jsStack
            ++ viewRawStderr ev.rawStderr
            ++ viewAbortFooter ev.source
        )


formatErrorMeta : ErrorEvent -> String
formatErrorMeta ev =
    let
        parts =
            List.filterMap identity
                [ Maybe.map (\l -> "at " ++ l) ev.location
                , Maybe.map (\c -> "inside FFI " ++ c) ev.ffiCall
                , Maybe.map (\us -> "t=" ++ formatMillis us ++ "ms") ev.atUs
                ]
    in
    String.join "  ·  " parts


formatMillis : Float -> String
formatMillis us =
    -- Match the original `(at_us / 1000).toFixed(1)` formatting.
    let
        ms =
            us / 1000

        rounded =
            toFloat (round (ms * 10)) / 10
    in
    String.fromFloat rounded


viewBreadcrumb : List String -> List (Html Msg)
viewBreadcrumb crumbs =
    if List.isEmpty crumbs then
        []

    else
        div [ class "log-error-trail" ]
            [ text
                ("--- last "
                    ++ String.fromInt (List.length crumbs)
                    ++ " trace events before failure ---"
                )
            ]
            :: List.map
                (\line -> div [ class "log-error-trail-line" ] [ text line ])
                crumbs


viewJsStack : Maybe String -> List (Html Msg)
viewJsStack maybeStack =
    case maybeStack of
        Nothing ->
            []

        Just stack ->
            [ div [ class "log-error-trail" ] [ text "--- JS exception stack ---" ]
            , div
                [ class "log-error-trail-line"
                , style "white-space" "pre-wrap"
                ]
                [ text stack ]
            ]


viewRawStderr : Maybe String -> List (Html Msg)
viewRawStderr maybeStderr =
    case maybeStderr of
        Nothing ->
            []

        Just stderrText ->
            [ div [ class "log-error-trail" ] [ text "--- raw stderr from wasm ---" ]
            , div
                [ class "log-error-trail-line"
                , style "white-space" "pre-wrap"
                ]
                [ text stderrText ]
            ]


viewAbortFooter : String -> List (Html Msg)
viewAbortFooter source =
    if source == "rust-panic" || source == "wasm-trap" then
        [ div [ class "log-error-meta" ]
            [ text "wasm module aborted after this point — reload the page to continue" ]
        ]

    else
        []


viewBuildFooter : BuildState -> Html Msg
viewBuildFooter state =
    case state of
        AwaitingPort ->
            text ""

        NoBuildInfo ->
            footerDiv [ text "tsot · build info unavailable" ]

        HasBuildInfo info ->
            footerDiv
                [ text
                    ("tsot · "
                        ++ info.profile
                        ++ " · built "
                        ++ info.builtAt
                        ++ " · "
                        ++ info.commit
                    )
                ]


footerDiv : List (Html msg) -> Html msg
footerDiv children =
    div
        [ style "position" "fixed"
        , style "bottom" "0"
        , style "right" "0"
        , style "padding" "0.15rem 0.5rem"
        , style "background" "rgba(20,20,28,0.85)"
        , style "border-top-left-radius" "4px"
        , style "color" "#555"
        , style "font-size" "0.65rem"
        , style "font-family" "ui-monospace, SFMono-Regular, Menlo, monospace"
        , style "pointer-events" "none"
        , style "z-index" "1000"
        ]
        children



-- MAIN


main : Program () Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        }
