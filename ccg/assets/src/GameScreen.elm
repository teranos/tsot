module GameScreen exposing
    ( Activation
    , ChooseCardData
    , ChooseIntData
    , ChoosePlayerData
    , CombatSelection
    , GameOverData
    , PickAttackersData
    , PickBlocksData
    , PickCardData
    , Prompt(..)
    , PromptButtonsConfig
    , SpectateData
    , UctCandidate
    , UctPreview
    , assignAttackerToStaged
    , clickBlocker
    , decodePrompt
    , decodeUctPreview
    , emptyCombatSelection
    , promptKindKey
    , promptToText
    , resetCombatSelection
    , toggleAttacker
    , viewPromptButtons
    )

{-| Chunk B/C target module — game-screen render path: `cardEl`
primitive + prompt-kind dispatch + interactive state. Lands in waves:

  - Wave 0 (this commit): types + decoders + `viewCard` primitive +
    `viewPromptButtons` for the simple kinds (Confirm / ChoosePlayer /
    ChooseInt). `viewCard` has no callers yet; it's exported so
    subsequent waves wire it in.
  - Wave 2+: card-zone rendering, click handlers, UCT preview state,
    casting banner.

The primitive's signature mirrors the JS `cardEl(card, opts)` shape:
caller passes a `CardOpts msg` record of optional decorations (click
handler, selected, dim, UCT badge, border tweaks for casting-host or
PickBlocks attacker, overlays like "→ blocks X" labels). All fields
optional via `defaultCardOpts`. Pattern matches SpectatorBar's
`Config msg`: the module stays Msg-agnostic, callers wire concrete
Msgs in at the use site.

-}

import Card
import Dict exposing (Dict)
import Html exposing (Html, button, div, input, span, text)
import Html.Attributes as A exposing (class, style)
import Html.Events as E
import Json.Decode as D
import Set exposing (Set)



-- TYPES


-- CardView + AttachedView consolidated into Card.elm
-- (`Card.Card` / `Card.CardData`) 2026-06-09. Decoder is `Card.decode`.


type alias Activation =
    { iid : String
    , abilityIndex : Int
    , text : String
    , needsX : Bool
    }


{-| Discriminated union of every prompt-kind the engine emits. The
9 real variants + `LoadingPrompt` for the pre-first-state / unknown
fallback. Subsequent waves dispatch view + click semantics per variant.
-}
type Prompt
    = LoadingPrompt
    | SpectatePrompt SpectateData
    | GameOverPrompt GameOverData
    | PickCardPrompt PickCardData
    | PickAttackersPrompt PickAttackersData
    | PickBlocksPrompt PickBlocksData
    | ChooseCardPrompt ChooseCardData
    | ConfirmPrompt String
    | ChoosePlayerPrompt ChoosePlayerData
    | ChooseIntPrompt ChooseIntData


type alias SpectateData =
    { turn : Int
    , phase : String
    , activePlayer : String
    , atEnd : Bool
    , winner : Maybe String
    }


type alias GameOverData =
    { winner : Maybe String
    , turn : Int
    }


type alias PickCardData =
    { candidates : List String
    , activations : List Activation
    }


type alias PickAttackersData =
    { eligible : List String
    }


type alias PickBlocksData =
    { attackers : List String
    , eligibleBlockers : List String
    }


type alias ChooseCardData =
    { pool : List String
    , poolCards : List Card.Card
    , host : Maybe String
    , optional : Bool
    , text : String
    }


type alias ChoosePlayerData =
    { candidates : List String
    , optional : Bool
    , text : String
    }


type alias ChooseIntData =
    { min : Int
    , max : Int
    , text : String
    }


{-| UCT preview envelope (from `preview_uct` FFI). Slice 3+4: while a
`PickCard` / `Main2Pick` prompt is live and ≥2 candidates exist, JS
fires a background UCT search. When it returns, the worker pushes
this envelope through `uctPreviewIn`; Elm decorates each candidate
card with a win-rate badge and outlines the recommended pick.
-}
type alias UctCandidate =
    { iid : String
    , winRate : Float
    , visits : Int
    , wins : Float
    }


type alias UctPreview =
    { candidates : List UctCandidate
    , chosen : Maybe String
    , iterationsCompleted : Int
    , promptKey : String
    , inFlight : Bool
    }



-- COMBAT SELECTION STATE


{-| Local interaction state for PickAttackers + PickBlocks. Held in
`Main.Model.combat` and mutated by the click handlers; reset to empty
after a Confirm/No-* action fires and on prompt-kind transitions out
of combat. Pure helpers below — exhaustively pinned by
`tests/CombatSelectionTest.elm`.

`attackers`     — iids the player has toggled on in PickAttackers
`blocks`        — blocker iid → attacker iid (the engine's pairs shape)
`blockerPickFor`— blocker staged in PickBlocks awaiting an attacker click

-}
type alias CombatSelection =
    { attackers : Set String
    , blocks : Dict String String
    , blockerPickFor : Maybe String
    }


emptyCombatSelection : CombatSelection
emptyCombatSelection =
    { attackers = Set.empty
    , blocks = Dict.empty
    , blockerPickFor = Nothing
    }


toggleAttacker : String -> CombatSelection -> CombatSelection
toggleAttacker iid s =
    if Set.member iid s.attackers then
        { s | attackers = Set.remove iid s.attackers }

    else
        { s | attackers = Set.insert iid s.attackers }


{-| Click on one of your eligible blockers during PickBlocks. Three
modes:

  - same iid as `blockerPickFor`: unstage (cancel the in-flight stage)
  - iid is a key in `blocks`: unassign (free the blocker for re-use)
  - otherwise: stage this blocker (next click on an attacker assigns)

If a *different* blocker was already staged, this re-stages to the
new one — matches the "click another eligible blocker before picking
an attacker" feel without leaving the player wondering why the old
stage is gone.

-}
clickBlocker : String -> CombatSelection -> CombatSelection
clickBlocker iid s =
    if s.blockerPickFor == Just iid then
        { s | blockerPickFor = Nothing }

    else if Dict.member iid s.blocks then
        { s | blocks = Dict.remove iid s.blocks }

    else
        { s | blockerPickFor = Just iid }


{-| Click on an attacker on the opponent's board during PickBlocks.
Only meaningful when a blocker is staged — assigns and clears the
stage so the player can immediately stage another blocker.
-}
assignAttackerToStaged : String -> CombatSelection -> CombatSelection
assignAttackerToStaged atkIid s =
    case s.blockerPickFor of
        Just blkIid ->
            { s
                | blocks = Dict.insert blkIid atkIid s.blocks
                , blockerPickFor = Nothing
            }

        Nothing ->
            s


resetCombatSelection : CombatSelection -> CombatSelection
resetCombatSelection _ =
    emptyCombatSelection



-- PROMPT-BAR TEXT


{-| Per-prompt-kind bar text, matching the JS-side `setPrompt(...)`
calls in `_renderInner` byte-exactly so the migration doesn't surprise
the user. The `ctx` carries viewer + iid→name labels for the few
variants that need them (GameOver, PickBlocks, ChooseCard host).
Test-pinned in `tests/PromptTextTest.elm`.

-}
promptToText :
    Maybe { viewer : String, labelByIid : String -> String }
    -> Prompt
    -> String
promptToText maybeCtx p =
    case p of
        LoadingPrompt ->
            "Loading\u{2026}"

        ConfirmPrompt text ->
            text

        ChoosePlayerPrompt data ->
            data.text

        ChooseIntPrompt data ->
            data.text
                ++ " ("
                ++ String.fromInt data.min
                ++ "\u{2013}"
                ++ String.fromInt data.max
                ++ ")"

        SpectatePrompt data ->
            let
                ap =
                    String.toUpper data.activePlayer

                endTag =
                    case ( data.atEnd, data.winner ) of
                        ( True, Just w ) ->
                            " \u{00B7} GAME OVER \u{00B7} " ++ String.toUpper w ++ " wins"

                        _ ->
                            ""
            in
            "Spectating \u{00B7} turn "
                ++ String.fromInt data.turn
                ++ " \u{00B7} "
                ++ data.phase
                ++ " \u{00B7} "
                ++ ap
                ++ " acts"
                ++ endTag

        GameOverPrompt data ->
            let
                viewer =
                    maybeCtx |> Maybe.map .viewer |> Maybe.withDefault ""

                youWon =
                    case data.winner of
                        Just w ->
                            String.toLower w == viewer

                        Nothing ->
                            False

                winnerText =
                    case data.winner of
                        Just w ->
                            String.toUpper w

                        Nothing ->
                            "draw"

                outcomeSuffix =
                    case data.winner of
                        Just _ ->
                            if youWon then
                                "(you win)"

                            else
                                "(you lose)"

                        Nothing ->
                            ""
            in
            "Game over. Winner: " ++ winnerText ++ " " ++ outcomeSuffix

        PickCardPrompt data ->
            let
                candPart =
                    String.fromInt (List.length data.candidates) ++ " card(s) in hand affordable"

                actPart =
                    if List.isEmpty data.activations then
                        []

                    else
                        [ String.fromInt (List.length data.activations) ++ " ability/abilities ready to activate" ]

                parts =
                    candPart :: actPart
            in
            "Your main phase \u{2014} "
                ++ String.join " \u{00B7} " parts
                ++ ". Click a hand card to play, click a board ability to activate, or pass."

        PickAttackersPrompt data ->
            if List.isEmpty data.eligible then
                "Combat \u{2014} no creatures can attack this turn."

            else
                "Combat \u{2014} click creatures to attack with ("
                    ++ String.fromInt (List.length data.eligible)
                    ++ " eligible), then confirm."

        PickBlocksPrompt data ->
            let
                label =
                    maybeCtx
                        |> Maybe.map .labelByIid
                        |> Maybe.withDefault identity

                incoming =
                    data.attackers
                        |> List.map label
                        |> String.join ", "
            in
            if List.isEmpty data.eligibleBlockers then
                "Combat \u{2014} incoming: "
                    ++ incoming
                    ++ ". No eligible blockers (your creatures are all tapped, sick-from-attack, or restricted)."

            else
                "Combat \u{2014} incoming: "
                    ++ incoming
                    ++ ". Click one of your highlighted creatures to stage as blocker; then click an attacker. Multiple blockers may share one attacker."

        ChooseCardPrompt data ->
            let
                label =
                    maybeCtx
                        |> Maybe.map .labelByIid
                        |> Maybe.withDefault identity

                base =
                    case data.host of
                        Just hostIid ->
                            "CASTING " ++ label hostIid ++ " \u{2014} " ++ data.text ++ "."

                        Nothing ->
                            "Choose a target \u{2014} " ++ data.text ++ "."

                maySkip =
                    if data.optional then
                        " \u{2014} may skip"

                    else
                        ""
            in
            base ++ maySkip



-- CardOpts + defaultCardOpts + decodeCardView + optionalList +
-- decodeAttached + the `required` pipeline helper consolidated into
-- Card.elm 2026-06-09. `Card.Config` is the new opts type;
-- `Card.decode` is the new decoder.


-- DECODERS


decodeActivation : D.Decoder Activation
decodeActivation =
    D.map4 Activation
        (D.field "iid" D.string)
        (D.field "ability_index" D.int)
        (D.field "text" D.string)
        (D.field "needs_x" D.bool)


decodePrompt : D.Decoder Prompt
decodePrompt =
    D.field "kind" D.string |> D.andThen decodePromptByKind


decodePromptByKind : String -> D.Decoder Prompt
decodePromptByKind kind =
    case kind of
        "Spectate" ->
            D.map SpectatePrompt decodeSpectateData

        "GameOver" ->
            D.map GameOverPrompt decodeGameOverData

        "PickCard" ->
            D.map PickCardPrompt decodePickCardData

        "PickAttackers" ->
            D.map PickAttackersPrompt decodePickAttackersData

        "PickBlocks" ->
            D.map PickBlocksPrompt decodePickBlocksData

        "ChooseCard" ->
            D.map ChooseCardPrompt decodeChooseCardData

        "Confirm" ->
            D.map ConfirmPrompt
                (D.maybe (D.field "prompt" D.string)
                    |> D.map (Maybe.withDefault "Confirm?")
                )

        "ChoosePlayer" ->
            D.map ChoosePlayerPrompt decodeChoosePlayerData

        "ChooseInt" ->
            D.map ChooseIntPrompt decodeChooseIntData

        _ ->
            D.succeed LoadingPrompt


decodeSpectateData : D.Decoder SpectateData
decodeSpectateData =
    D.map5 SpectateData
        (D.field "turn" D.int)
        (D.field "phase" D.string)
        (D.field "active_player" D.string)
        (D.maybe (D.field "at_end" D.bool) |> D.map (Maybe.withDefault False))
        (D.maybe (D.field "winner" D.string))


decodeGameOverData : D.Decoder GameOverData
decodeGameOverData =
    D.map2 GameOverData
        (D.maybe (D.field "winner" D.string))
        (D.maybe (D.field "turn" D.int) |> D.map (Maybe.withDefault 0))


decodePickCardData : D.Decoder PickCardData
decodePickCardData =
    D.map2 PickCardData
        (D.maybe (D.field "candidates" (D.list D.string)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "activations" (D.list decodeActivation)) |> D.map (Maybe.withDefault []))


decodePickAttackersData : D.Decoder PickAttackersData
decodePickAttackersData =
    D.map PickAttackersData
        (D.maybe (D.field "eligible" (D.list D.string)) |> D.map (Maybe.withDefault []))


decodePickBlocksData : D.Decoder PickBlocksData
decodePickBlocksData =
    D.map2 PickBlocksData
        (D.maybe (D.field "attackers" (D.list D.string)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "eligible_blockers" (D.list D.string)) |> D.map (Maybe.withDefault []))


decodeChooseCardData : D.Decoder ChooseCardData
decodeChooseCardData =
    D.map5 ChooseCardData
        (D.maybe (D.field "pool" (D.list D.string)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "pool_cards" (D.list Card.decode)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "host" D.string))
        (D.maybe (D.field "optional" D.bool) |> D.map (Maybe.withDefault False))
        (D.maybe (D.field "prompt" D.string) |> D.map (Maybe.withDefault ""))


decodeChoosePlayerData : D.Decoder ChoosePlayerData
decodeChoosePlayerData =
    D.map3 ChoosePlayerData
        (D.maybe (D.field "candidates" (D.list D.string)) |> D.map (Maybe.withDefault []))
        (D.maybe (D.field "optional" D.bool) |> D.map (Maybe.withDefault False))
        (D.maybe (D.field "prompt" D.string) |> D.map (Maybe.withDefault "Choose a player."))


decodeChooseIntData : D.Decoder ChooseIntData
decodeChooseIntData =
    D.map3 ChooseIntData
        (D.field "min" D.int)
        (D.field "max" D.int)
        (D.maybe (D.field "prompt" D.string) |> D.map (Maybe.withDefault "Choose a number."))


decodeUctCandidate : D.Decoder UctCandidate
decodeUctCandidate =
    D.map4 UctCandidate
        (D.field "iid" D.string)
        (D.maybe (D.field "win_rate" D.float) |> D.map (Maybe.withDefault 0))
        (D.maybe (D.field "visits" D.int) |> D.map (Maybe.withDefault 0))
        (D.maybe (D.field "wins" D.float) |> D.map (Maybe.withDefault 0))


decodeUctPreview : D.Decoder UctPreview
decodeUctPreview =
    D.map5 UctPreview
        (D.field "candidates" (D.list decodeUctCandidate))
        (D.maybe (D.field "chosen" D.string))
        (D.maybe (D.field "iterations_completed" D.int) |> D.map (Maybe.withDefault 0))
        (D.maybe (D.field "prompt_key" D.string) |> D.map (Maybe.withDefault ""))
        (D.maybe (D.field "in_flight" D.bool) |> D.map (Maybe.withDefault False))


promptKindKey : Prompt -> String
promptKindKey p =
    case p of
        LoadingPrompt ->
            "Loading"

        SpectatePrompt _ ->
            "Spectate"

        GameOverPrompt _ ->
            "GameOver"

        PickCardPrompt _ ->
            "PickCard"

        PickAttackersPrompt _ ->
            "PickAttackers"

        PickBlocksPrompt _ ->
            "PickBlocks"

        ChooseCardPrompt _ ->
            "ChooseCard"

        ConfirmPrompt _ ->
            "Confirm"

        ChoosePlayerPrompt _ ->
            "ChoosePlayer"

        ChooseIntPrompt _ ->
            "ChooseInt"



-- Card primitive (viewCard, CardView, CardOpts, all the supporting
-- helpers — viewCardHead / viewCardMeta / viewCardAbilities / colorTag
-- / symbolTag / formatNumber / titleTextFor / viewUctBadge) consolidated
-- into Card.elm 2026-06-09. This module no longer renders cards; it
-- only owns Prompt / CombatSelection / UctPreview types and the
-- prompt-bar text + buttons.


-- PROMPT-BAR BUTTONS


{-| Wave 1 scope: render the buttons + number-input for Confirm /
ChoosePlayer / ChooseInt. Card containers stay JS-owned in this wave;
Wave 2 takes them over. The DIV sits in the game-screen scaffold
(after JS-owned `#buttons`) so transitions don't shuffle layout.

`chooseIntDraft` is the current value of the number input — lives in
`Main.Model.chooseIntDraft` so Elm tracks the in-flight value between
keystrokes; the Confirm-click action reads it, parses, fires the
ChoiceInt FFI.

-}
type alias PromptButtonsConfig msg =
    { onConfirmYes : msg
    , onConfirmNo : msg
    , onPlayerChoice : Maybe String -> msg
    , onIntInput : String -> msg
    , onIntConfirm : Int -> msg
    , onPass : msg
    , onSkipChoiceCard : msg
    , onConfirmAttackers : msg
    , onNoAttack : msg
    , onConfirmBlocks : msg
    , onNoBlocks : msg
    }


viewPromptButtons : PromptButtonsConfig msg -> String -> CombatSelection -> Prompt -> Html msg
viewPromptButtons cfg chooseIntDraft combat prompt =
    div [ A.id "elm-prompt-buttons", style "margin-top" "0.5rem" ]
        (case prompt of
            ConfirmPrompt _ ->
                [ button [ E.onClick cfg.onConfirmYes ] [ text "Yes" ]
                , button [ class "danger", E.onClick cfg.onConfirmNo ] [ text "No" ]
                ]

            ChoosePlayerPrompt data ->
                List.map
                    (\pid ->
                        button
                            [ E.onClick (cfg.onPlayerChoice (Just pid)) ]
                            [ text pid ]
                    )
                    data.candidates
                    ++ (if data.optional then
                            [ button
                                [ E.onClick (cfg.onPlayerChoice Nothing) ]
                                [ text "Skip" ]
                            ]

                        else
                            []
                       )

            ChooseIntPrompt data ->
                let
                    parsed =
                        String.toInt chooseIntDraft |> Maybe.withDefault data.min
                in
                [ input
                    [ A.type_ "number"
                    , A.min (String.fromInt data.min)
                    , A.max (String.fromInt data.max)
                    , A.value chooseIntDraft
                    , E.onInput cfg.onIntInput
                    , style "background" "#234"
                    , style "color" "#ddd"
                    , style "border" "1px solid #4af"
                    , style "padding" "0.3rem"
                    , style "font-family" "inherit"
                    , style "width" "5rem"
                    , style "margin-right" "0.5rem"
                    ]
                    []
                , button
                    [ E.onClick (cfg.onIntConfirm parsed) ]
                    [ text "Confirm" ]
                ]

            PickCardPrompt _ ->
                [ button [ E.onClick cfg.onPass ] [ text "Pass" ] ]

            ChooseCardPrompt data ->
                if data.optional then
                    [ button [ E.onClick cfg.onSkipChoiceCard ] [ text "Skip" ] ]

                else
                    []

            PickAttackersPrompt data ->
                let
                    eligibleCount =
                        List.length data.eligible

                    attackBtn =
                        if eligibleCount > 0 then
                            [ button
                                [ E.onClick cfg.onConfirmAttackers ]
                                [ text ("Attack (" ++ String.fromInt (Set.size combat.attackers) ++ ")") ]
                            ]

                        else
                            []

                    skipLabel =
                        if eligibleCount == 0 then
                            "End combat"

                        else
                            "No attack"
                in
                attackBtn
                    ++ [ button
                            [ class "danger", E.onClick cfg.onNoAttack ]
                            [ text skipLabel ]
                       ]

            PickBlocksPrompt _ ->
                [ button
                    [ E.onClick cfg.onConfirmBlocks ]
                    [ text ("Confirm blocks (" ++ String.fromInt (Dict.size combat.blocks) ++ ")") ]
                , button
                    [ class "danger", E.onClick cfg.onNoBlocks ]
                    [ text "No blocks" ]
                ]

            _ ->
                []
        )
