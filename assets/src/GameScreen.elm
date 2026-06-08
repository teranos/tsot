module GameScreen exposing
    ( Activation
    , CardOpts
    , CardView
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
    , decodeCardView
    , decodePrompt
    , decodeUctPreview
    , defaultCardOpts
    , emptyCombatSelection
    , promptKindKey
    , resetCombatSelection
    , toggleAttacker
    , viewCard
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

import Dict exposing (Dict)
import Html exposing (Html, button, div, input, span, text)
import Html.Attributes as A exposing (class, style)
import Html.Events as E
import Json.Decode as D
import Set exposing (Set)



-- TYPES


{-| Mirrors `tsot::sim::snapshot::CardView` on the wire. `effective_cost`
differs from `cost` only when a static cost-reduction (Modern LCD Clock
etc.) applies at cast time; `cardEl` shows both with a strike-through
when they differ.
-}
type alias CardView =
    { iid : String
    , id : String
    , name : String
    , kind : String
    , colors : List String
    , symbols : List String
    , subtypes : List String
    , cost : String
    , effectiveCost : String
    , abilities : List String
    , tapped : Bool
    , summoningSick : Bool
    , damage : Float
    , power : Float
    , toughness : Float
    }


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



-- CARD OPTS


type alias CardOpts msg =
    { clickable : Maybe (String -> msg)
    , selected : Bool
    , dim : Bool
    , uctBadge : Maybe UctCandidate
    , uctChosen : Bool
    , borderColor : Maybe String
    , borderStyle : Maybe String
    , overlays : List (Html msg)
    }


defaultCardOpts : CardOpts msg
defaultCardOpts =
    { clickable = Nothing
    , selected = False
    , dim = False
    , uctBadge = Nothing
    , uctChosen = False
    , borderColor = Nothing
    , borderStyle = Nothing
    , overlays = []
    }



-- DECODERS


required : String -> D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
required field aDec fDec =
    D.map2 (\f a -> f a) fDec (D.field field aDec)


decodeCardView : D.Decoder CardView
decodeCardView =
    D.succeed CardView
        |> required "iid" D.string
        |> required "id" D.string
        |> required "name" D.string
        |> required "kind" D.string
        |> required "colors" (D.list D.string)
        |> required "symbols" (D.list D.string)
        |> required "subtypes" (D.list D.string)
        |> required "cost" D.string
        |> required "effective_cost" D.string
        |> required "abilities" (D.list D.string)
        |> required "tapped" D.bool
        |> required "summoning_sick" D.bool
        |> required "damage" D.float
        |> required "power" D.float
        |> required "toughness" D.float


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
    D.map4 ChooseCardData
        (D.maybe (D.field "pool" (D.list D.string)) |> D.map (Maybe.withDefault []))
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



-- CARD PRIMITIVE


{-| Port of the JS `cardEl(card, opts)` function. Mirrors the same DOM
shape so the existing CSS rules in `play.html`'s `<style>` block
(`.card`, `.head`, `.cost`, `.name`, `.stats`, `.color-X`, `.symbol`,
`.meta-line`, `.abilities`, `.clickable`, `.selected`, `.tapped`,
`.sick`, `.uct-recommended`, `.uct-badge`) keep matching unchanged.

`opts.clickable = Just toMsg` wires a click handler that fires
`toMsg iid` and adds the `.clickable` class. `opts.uctBadge` adds a
UCT preview win-rate badge in the top-right; `opts.uctChosen` flags
the card as UCT's recommendation. `opts.overlays` is a list of extra
Html appended after the meta-line + abilities (used for the per-card
labels JS added inline — "→ blocks X", "… click an attacker",
ability rows under cards in PickCard activations, "◆ casting" host
badge).

-}
viewCard : CardOpts msg -> CardView -> Html msg
viewCard opts card =
    let
        flag : Bool -> String -> String
        flag b name =
            if b then
                name

            else
                ""

        classes =
            String.join " " <|
                List.filter (not << String.isEmpty)
                    [ "card"
                    , flag (opts.clickable /= Nothing) "clickable"
                    , flag opts.selected "selected"
                    , flag card.tapped "tapped"
                    , flag card.summoningSick "sick"
                    , flag opts.uctChosen "uct-recommended"
                    ]

        styleAttrs =
            List.filterMap identity
                [ if opts.dim then
                    Just (style "opacity" "0.6")

                  else
                    Nothing
                , Maybe.map (style "border-color") opts.borderColor
                , Maybe.map (style "border-style") opts.borderStyle
                ]

        titleAttr =
            A.title
                (card.iid
                    ++ (if card.summoningSick then
                            "  (summoning sick)"

                        else
                            ""
                       )
                )

        clickAttrs =
            case opts.clickable of
                Just toMsg ->
                    [ E.onClick (toMsg card.iid) ]

                Nothing ->
                    []
    in
    div
        (class classes :: titleAttr :: styleAttrs ++ clickAttrs)
        (viewUctBadge opts.uctBadge
            ++ [ viewCardHead card ]
            ++ viewCardMeta card
            ++ viewCardAbilities card.abilities
            ++ opts.overlays
        )


viewUctBadge : Maybe UctCandidate -> List (Html msg)
viewUctBadge maybeCand =
    case maybeCand of
        Nothing ->
            []

        Just cand ->
            let
                pct =
                    String.fromInt (round (cand.winRate * 100))

                winsStr =
                    formatNumber (toFloat (round (cand.wins * 10)) / 10)
            in
            [ div
                [ class "uct-badge"
                , A.title
                    ("UCT visits="
                        ++ String.fromInt cand.visits
                        ++ " wins="
                        ++ winsStr
                    )
                ]
                [ text ("UCT " ++ pct ++ "%") ]
            ]


viewCardHead : CardView -> Html msg
viewCardHead card =
    let
        printed =
            card.cost

        effective =
            if String.isEmpty card.effectiveCost then
                printed

            else
                card.effectiveCost

        costSpans =
            if printed == effective then
                [ span [ class "cost" ] [ text printed ] ]

            else
                [ span [ class "cost" ] [ text effective ]
                , span
                    [ class "cost"
                    , style "color" "#666"
                    , style "text-decoration" "line-through"
                    , style "margin-left" "0.3rem"
                    ]
                    [ text printed ]
                ]
    in
    div [ class "head" ]
        (span [ class "name" ] [ text card.name ] :: costSpans)


viewCardMeta : CardView -> List (Html msg)
viewCardMeta card =
    let
        statsPart =
            if card.kind == "Creature" then
                let
                    effT =
                        card.toughness - card.damage

                    base =
                        formatNumber card.power ++ "/" ++ formatNumber effT

                    dmgTag =
                        if card.damage > 0 then
                            " (-" ++ formatNumber card.damage ++ ")"

                        else
                            ""
                in
                [ span [ class "stats" ] [ text (base ++ dmgTag) ] ]

            else
                []

        colorParts =
            List.map colorTag card.colors

        symbolParts =
            List.map symbolTag card.symbols

        subtypeParts =
            if List.isEmpty card.subtypes then
                []

            else
                [ span [ style "color" "#888" ] [ text (String.join "·" card.subtypes) ] ]

        groups =
            List.filter (not << List.isEmpty)
                [ statsPart
                , colorParts
                , symbolParts
                , subtypeParts
                ]

        joined =
            groups
                |> List.intersperse [ text " " ]
                |> List.concat
    in
    if List.isEmpty joined then
        []

    else
        [ div [ class "meta-line" ] joined ]


viewCardAbilities : List String -> List (Html msg)
viewCardAbilities abilities =
    if List.isEmpty abilities then
        []

    else
        [ div [ class "abilities" ]
            (List.map (\a -> div [] [ text a ]) abilities)
        ]


colorTag : String -> Html msg
colorTag c =
    let
        code =
            String.toLower c
    in
    span [ class ("color-" ++ code) ] [ text code ]


symbolTag : String -> Html msg
symbolTag s =
    span [ class "symbol" ] [ text s ]


{-| Render whole-numbered Floats without a trailing `.0` — power/toughness
in CardView are wire-typed Float but visually expected as integers
when they happen to be whole. Damage same.
-}
formatNumber : Float -> String
formatNumber n =
    if n == toFloat (floor n) then
        String.fromInt (floor n)

    else
        String.fromFloat n



-- WAVE 1: BUTTONS FOR SIMPLE PROMPTS


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
