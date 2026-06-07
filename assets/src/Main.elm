module Main exposing (main)

{-| H7-Elm Stage 1 — minimal Elm app, mounted into `<div id="elm-root">`
by `assets/js-bridge.js`. Renders a fixed-position "Elm: ready" pill in
the bottom-left so the developer can see the bundle loaded + Browser.element
initialized end-to-end. No ports yet, no model state — Stage 2 is when
the build-info footer ports over and the wiring gets exercised.
-}

import Browser
import Html exposing (Html, div, text)
import Html.Attributes exposing (style)


type alias Model =
    ()


type alias Msg =
    ()


main : Program () Model Msg
main =
    Browser.element
        { init = init
        , update = update
        , view = view
        , subscriptions = subscriptions
        }


init : () -> ( Model, Cmd Msg )
init _ =
    ( (), Cmd.none )


update : Msg -> Model -> ( Model, Cmd Msg )
update _ model =
    ( model, Cmd.none )


subscriptions : Model -> Sub Msg
subscriptions _ =
    Sub.none


view : Model -> Html Msg
view _ =
    div
        [ style "position" "fixed"
        , style "bottom" "4px"
        , style "left" "4px"
        , style "padding" "2px 6px"
        , style "background" "#1a3018"
        , style "border" "1px solid #4a8"
        , style "color" "#6f9"
        , style "font-family" "ui-monospace, SFMono-Regular, Menlo, monospace"
        , style "font-size" "0.65rem"
        , style "border-radius" "2px"
        , style "pointer-events" "none"
        , style "z-index" "9999"
        ]
        [ text "Elm: ready" ]
