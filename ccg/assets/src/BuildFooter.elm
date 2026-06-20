module BuildFooter exposing
    ( Info
    , State(..)
    , decode
    , view
    )

{-| Third `Main` split. The fixed bottom-right build pill driven by
`buildInfoIn` carrying `window.__TSOT_BUILD__`. Pure render; state
(`Model.build : BuildFooter.State`) and the Msg variant
(`BuildInfoReceived`) live in `Main`.
-}

import Html exposing (Html, div, text)
import Html.Attributes exposing (style)
import Json.Decode as D


type alias Info =
    { profile : String
    , builtAt : String
    , commit : String
    }


type State
    = AwaitingPort
    | NoBuildInfo
    | HasBuildInfo Info


decode : D.Decoder Info
decode =
    D.map3 Info
        (D.field "profile" D.string)
        (D.field "builtAt" D.string)
        (D.field "commit" D.string)


view : State -> Html msg
view state =
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
