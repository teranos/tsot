module LogPanel exposing
    ( Entry(..)
    , ErrorEvent
    , containerId
    , decodeError
    , view
    )

{-| Second `Main` split. The LOG panel is pure render — text lines +
structured error blocks — driven by `logTextIn` / `logErrorIn` ports
wired in `Main`. The error-event decoder lives here so the `LogErrorReceived`
branch in `Main.update` only needs to know about `LogPanel.Entry`.

State (`List Entry`) stays in `Main.Model.log`; this module exposes no
init/update. `containerId` is also exported so the auto-scroll-to-
bottom `Browser.Dom.setViewportOf` Cmd in `Main` can target the same
element id.

-}

import Html exposing (Html, div, pre, span, text)
import Html.Attributes exposing (class, id, style)
import Json.Decode as D


type Entry
    = TextLine String
    | ErrorEntry ErrorEvent


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


{-| Stable DOM id; used by the scroll-to-bottom Cmd in Main as well as
the rendered `<pre>` element here.
-}
containerId : String
containerId =
    "elm-log"


decodeError : D.Decoder ErrorEvent
decodeError =
    D.map8 ErrorEvent
        (D.maybe (D.field "source" D.string) |> D.map (Maybe.withDefault "error"))
        (D.maybe (D.field "message" D.string) |> D.map (Maybe.withDefault "(no message)"))
        (D.maybe (D.field "location" D.string))
        (D.maybe (D.field "ffi_call" D.string))
        (D.maybe (D.field "at_us" D.float))
        (D.maybe (D.field "breadcrumb" (D.list D.string)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "js_stack" D.string))
        (D.maybe (D.field "raw_stderr" D.string))


view : List Entry -> Html msg
view entries =
    pre
        [ id containerId
        , style "max-height" "24rem"
        , style "overflow-y" "auto"
        , style "font-size" "0.75rem"
        , style "color" "#aaa"
        , style "white-space" "pre"
        , style "margin" "0"
        ]
        (List.map viewEntry entries)


viewEntry : Entry -> Html msg
viewEntry entry =
    case entry of
        TextLine line ->
            if String.contains "ERR:" line then
                span [ style "color" "#f88" ] [ text (line ++ "\n") ]

            else
                text (line ++ "\n")

        ErrorEntry ev ->
            viewErrorBlock ev


viewErrorBlock : ErrorEvent -> Html msg
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
    let
        ms =
            us / 1000

        rounded =
            toFloat (round (ms * 10)) / 10
    in
    String.fromFloat rounded


viewBreadcrumb : List String -> List (Html msg)
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


viewJsStack : Maybe String -> List (Html msg)
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


viewRawStderr : Maybe String -> List (Html msg)
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


viewAbortFooter : String -> List (Html msg)
viewAbortFooter source =
    if source == "rust-panic" || source == "wasm-trap" then
        [ div [ class "log-error-meta" ]
            [ text "wasm module aborted after this point — reload the page to continue" ]
        ]

    else
        []
