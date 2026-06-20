module SpectatorBar exposing
    ( Config
    , Model
    , Snapshot
    , decode
    , init
    , view
    )

{-| First Elm dev-tool module split out of `Main`. Pattern: this
module owns the spectator bar's state shape + decoder + render; the
Msg constructors stay in `Main` and are passed in via a `Config msg`
record so `Main`'s update/dispatch keeps centralized control. No
sub-`update` here — `Main.update` mutates `Model.spectatorBar`
directly on `SpectatorStateReceived` and the local speed change.

The bar's state is mostly JS-owned (snapshots array, interval handle,
seek index). Elm tracks the projection it needs to render: index,
total, playing flag, current snapshot's display fields, speed
setting, winner. JS pushes the projection on every spectator state
change (seek / step / play tick / pause / exit) via the
`spectatorStateIn` port wired in `Main`.

The seven outbound clicks (back-end, step-back, play/pause, step-fwd,
fwd-end, slider drag, speed change, exit) each fire a `workerCmdOut`
envelope from `Main.update`; this module knows nothing about ports.

-}

import Html exposing (Html, button, div, input, label, option, select, span, text)
import Html.Attributes as A
import Html.Events as E
import Json.Decode as D


type alias Model =
    { active : Bool
    , index : Int
    , total : Int
    , playing : Bool
    , msPerStep : Int
    , winner : Maybe String
    , currentSnapshot : Maybe Snapshot
    }


type alias Snapshot =
    { turn : Int
    , phase : String
    , activePlayer : String
    }


{-| Constructor record passed by `Main`: each click handler becomes a
`msg` value so the view tree is polymorphic in `msg`. Slider and speed
events carry the new value as `String` (raw `event.target.value`).
-}
type alias Config msg =
    { onBackEnd : msg
    , onStepBack : msg
    , onPlayPause : msg
    , onStepFwd : msg
    , onFwdEnd : msg
    , onSliderChange : String -> msg
    , onSpeedChange : String -> msg
    , onExit : msg
    }


init : Model
init =
    { active = False
    , index = 0
    , total = 0
    , playing = False
    , msPerStep = 500
    , winner = Nothing
    , currentSnapshot = Nothing
    }


{-| Mirrors the envelope `play.html`'s `window.tsotPushSpectatorState`
sends — `active` is the visibility gate (false when spectate exits);
`snapshot` is null between snapshots loading and the first
`spectateRenderCurrent` call.
-}
decode : D.Decoder Model
decode =
    D.map7 Model
        (D.field "active" D.bool)
        (D.field "index" D.int)
        (D.field "total" D.int)
        (D.field "playing" D.bool)
        (D.field "msPerStep" D.int)
        (D.maybe (D.field "winner" D.string))
        (D.maybe (D.field "snapshot" decodeSnapshot))


decodeSnapshot : D.Decoder Snapshot
decodeSnapshot =
    D.map3 Snapshot
        (D.field "turn" D.int)
        (D.field "phase" D.string)
        (D.field "activePlayer" D.string)


view : Config msg -> Model -> Html msg
view cfg m =
    if not m.active then
        text ""

    else
        div
            [ A.id "spectator-bar"
            , A.style "padding" "0.4rem 0.6rem"
            , A.style "background" "#1c1c24"
            , A.style "border" "1px solid #333"
            , A.style "margin-bottom" "0.5rem"
            ]
            [ controlsRow cfg m
            , slider cfg m
            ]


controlsRow : Config msg -> Model -> Html msg
controlsRow cfg m =
    div
        [ A.style "display" "flex"
        , A.style "gap" "0.5rem"
        , A.style "align-items" "center"
        , A.style "flex-wrap" "wrap"
        ]
        [ button [ E.onClick cfg.onBackEnd ] [ text "⏮" ]
        , button [ E.onClick cfg.onStepBack ] [ text "◀" ]
        , button [ E.onClick cfg.onPlayPause ]
            [ text
                (if m.playing then
                    "⏸"

                 else
                    "⏵"
                )
            ]
        , button [ E.onClick cfg.onStepFwd ] [ text "▶" ]
        , button [ E.onClick cfg.onFwdEnd ] [ text "⏭" ]
        , span
            [ A.id "spec-readout"
            , A.style "color" "#888"
            , A.style "font-size" "0.75rem"
            , A.style "margin-left" "0.5rem"
            , A.style "min-width" "18rem"
            ]
            [ text (readoutText m) ]
        , speedPicker cfg m
        , button
            [ A.class "danger"
            , E.onClick cfg.onExit
            ]
            [ text "Exit spectate" ]
        ]


speedPicker : Config msg -> Model -> Html msg
speedPicker cfg m =
    label
        [ A.style "color" "#888"
        , A.style "font-size" "0.7rem"
        , A.style "margin-left" "auto"
        ]
        [ text "speed "
        , select
            [ E.onInput cfg.onSpeedChange
            , A.value (String.fromInt m.msPerStep)
            , A.style "background" "#1c1c20"
            , A.style "color" "#ddd"
            , A.style "border" "1px solid #444"
            , A.style "padding" "0.15rem 0.3rem"
            , A.style "font-family" "inherit"
            ]
            [ option [ A.value "200" ] [ text "fast (200ms)" ]
            , option [ A.value "500" ] [ text "normal (500ms)" ]
            , option [ A.value "1000" ] [ text "slow (1s)" ]
            , option [ A.value "2000" ] [ text "very slow (2s)" ]
            ]
        ]


slider : Config msg -> Model -> Html msg
slider cfg m =
    input
        [ A.type_ "range"
        , A.min "0"
        , A.max (String.fromInt (max 0 (m.total - 1)))
        , A.value (String.fromInt m.index)
        , E.onInput cfg.onSliderChange
        , A.style "width" "100%"
        , A.style "margin-top" "0.4rem"
        ]
        []


readoutText : Model -> String
readoutText m =
    case m.currentSnapshot of
        Nothing ->
            ""

        Just snap ->
            let
                ap =
                    String.toUpper snap.activePlayer

                endTag =
                    if m.index == m.total - 1 then
                        case m.winner of
                            Just w ->
                                "  · GAME OVER · " ++ String.toUpper w ++ " wins"

                            Nothing ->
                                ""

                    else
                        ""
            in
            "snapshot "
                ++ String.fromInt (m.index + 1)
                ++ "/"
                ++ String.fromInt m.total
                ++ "  · turn "
                ++ String.fromInt snap.turn
                ++ " · "
                ++ snap.phase
                ++ " · "
                ++ ap
                ++ " acts"
                ++ endTag
