module PromptTextTest exposing (suite)

{-| Stage 13 first half — `promptToText` pins the JS-side prompt-bar
strings byte-exactly so the port migration doesn't change what the
user sees. Each test below corresponds to one of the `setPrompt(...)`
call sites in `play.html`'s `_renderInner` prompt-kind dispatch.

Inputs:

  - `Prompt` decoded variant
  - `Maybe ViewerCtx` for the few kinds that need viewer/labels:
      * Spectate uses the viewer's perspective for the `you-are-X`
        suffix (handled implicitly — Spectate text is identity-free)
      * GameOver uses the viewer to detect "(you win)" vs "(you lose)"
      * PickBlocks lists attacker labels for the incoming line
      * ChooseCard payment-mode uses the host card's name

The viewer + labels come from the live game-state slice; this test
keeps that surface narrow by accepting it as a record.

-}

import Expect
import GameScreen exposing (..)
import Test exposing (Test, describe, test)


type alias Ctx =
    { viewer : String
    , labelByIid : String -> String
    }


noCtx : Maybe Ctx
noCtx =
    Nothing


withCtx : String -> List ( String, String ) -> Maybe Ctx
withCtx viewer pairs =
    Just
        { viewer = viewer
        , labelByIid =
            \iid ->
                pairs
                    |> List.filter (\( k, _ ) -> k == iid)
                    |> List.head
                    |> Maybe.map Tuple.second
                    |> Maybe.withDefault iid
        }


suite : Test
suite =
    describe "promptToText"
        [ describe "Confirm / ChoosePlayer / ChooseInt — text comes straight from the prompt"
            [ test "Confirm uses the prompt's text" <|
                \_ ->
                    promptToText noCtx (ConfirmPrompt "aim at opponent? (no = burn a creature)")
                        |> Expect.equal "aim at opponent? (no = burn a creature)"
            , test "ChoosePlayer uses the prompt's text" <|
                \_ ->
                    promptToText noCtx
                        (ChoosePlayerPrompt
                            { candidates = [ "a", "b" ]
                            , optional = False
                            , text = "Pick a target player"
                            }
                        )
                        |> Expect.equal "Pick a target player"
            , test "ChooseInt appends the (min–max) range" <|
                \_ ->
                    promptToText noCtx
                        (ChooseIntPrompt
                            { min = 1
                            , max = 5
                            , text = "Choose X"
                            }
                        )
                        |> Expect.equal "Choose X (1\u{2013}5)"
            ]
        , describe "Spectate"
            [ test "mid-game variant: turn / phase / active player (uppercased)" <|
                \_ ->
                    promptToText noCtx
                        (SpectatePrompt
                            { turn = 3
                            , phase = "Main1"
                            , activePlayer = "a"
                            , atEnd = False
                            , winner = Nothing
                            }
                        )
                        |> Expect.equal "Spectating \u{00B7} turn 3 \u{00B7} Main1 \u{00B7} A acts"
            , test "end-of-game variant: appends GAME OVER + uppercased winner" <|
                \_ ->
                    promptToText noCtx
                        (SpectatePrompt
                            { turn = 10
                            , phase = "End"
                            , activePlayer = "a"
                            , atEnd = True
                            , winner = Just "a"
                            }
                        )
                        |> Expect.equal "Spectating \u{00B7} turn 10 \u{00B7} End \u{00B7} A acts \u{00B7} GAME OVER \u{00B7} A wins"
            , test "at_end true but no winner → no end tag" <|
                \_ ->
                    promptToText noCtx
                        (SpectatePrompt
                            { turn = 5
                            , phase = "Main1"
                            , activePlayer = "b"
                            , atEnd = True
                            , winner = Nothing
                            }
                        )
                        |> Expect.equal "Spectating \u{00B7} turn 5 \u{00B7} Main1 \u{00B7} B acts"
            ]
        , describe "GameOver"
            [ test "you win: winner matches viewer" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (GameOverPrompt { winner = Just "a", turn = 12 })
                        |> Expect.equal "Game over. Winner: A (you win)"
            , test "you lose: winner is the opponent" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (GameOverPrompt { winner = Just "b", turn = 12 })
                        |> Expect.equal "Game over. Winner: B (you lose)"
            , test "draw: no winner" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (GameOverPrompt { winner = Nothing, turn = 12 })
                        |> Expect.equal "Game over. Winner: draw "
            ]
        , describe "PickCard"
            [ test "no activations: just the affordable-cards count" <|
                \_ ->
                    promptToText noCtx
                        (PickCardPrompt
                            { candidates = [ "A:0001:x", "A:0002:y" ]
                            , activations = []
                            }
                        )
                        |> Expect.equal
                            "Your main phase \u{2014} 2 card(s) in hand affordable. Click a hand card to play, click a board ability to activate, or pass."
            , test "with activations: appends the ability count" <|
                \_ ->
                    let
                        act =
                            { iid = "A:0001:c"
                            , abilityIndex = 0
                            , text = "tap"
                            , needsX = False
                            }
                    in
                    promptToText noCtx
                        (PickCardPrompt
                            { candidates = [ "A:0001:x" ]
                            , activations = [ act, act ]
                            }
                        )
                        |> Expect.equal
                            "Your main phase \u{2014} 1 card(s) in hand affordable \u{00B7} 2 ability/abilities ready to activate. Click a hand card to play, click a board ability to activate, or pass."
            ]
        , describe "PickAttackers"
            [ test "no eligible creatures: short variant" <|
                \_ ->
                    promptToText noCtx (PickAttackersPrompt { eligible = [] })
                        |> Expect.equal "Combat \u{2014} no creatures can attack this turn."
            , test "eligible creatures: action variant with count" <|
                \_ ->
                    promptToText noCtx (PickAttackersPrompt { eligible = [ "x", "y", "z" ] })
                        |> Expect.equal "Combat \u{2014} click creatures to attack with (3 eligible), then confirm."
            ]
        , describe "PickBlocks"
            [ test "with eligible blockers: action variant lists incoming attacker labels" <|
                \_ ->
                    promptToText
                        (withCtx "a"
                            [ ( "B:0001:atk", "Dragon" )
                            , ( "B:0002:atk", "Bear" )
                            ]
                        )
                        (PickBlocksPrompt
                            { attackers = [ "B:0001:atk", "B:0002:atk" ]
                            , eligibleBlockers = [ "A:0001:blk" ]
                            }
                        )
                        |> Expect.equal
                            "Combat \u{2014} incoming: Dragon, Bear. Click one of your highlighted creatures to stage as blocker; then click an attacker. Multiple blockers may share one attacker."
            , test "no eligible blockers: 'no eligible blockers' variant" <|
                \_ ->
                    promptToText
                        (withCtx "a" [ ( "B:0001:atk", "Dragon" ) ])
                        (PickBlocksPrompt
                            { attackers = [ "B:0001:atk" ]
                            , eligibleBlockers = []
                            }
                        )
                        |> Expect.equal
                            "Combat \u{2014} incoming: Dragon. No eligible blockers (your creatures are all tapped, sick-from-attack, or restricted)."
            ]
        , describe "ChooseCard"
            [ test "non-payment variant: 'Choose a target'" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (ChooseCardPrompt
                            { pool = [ "A:0001:x" ]
                            , host = Nothing
                            , optional = False
                            , text = "deal 4 damage to"
                            }
                        )
                        |> Expect.equal "Choose a target \u{2014} deal 4 damage to."
            , test "non-payment variant: appends ' — may skip' when optional" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (ChooseCardPrompt
                            { pool = [ "A:0001:x" ]
                            , host = Nothing
                            , optional = True
                            , text = "deal 4 damage to"
                            }
                        )
                        |> Expect.equal "Choose a target \u{2014} deal 4 damage to. \u{2014} may skip"
            , test "payment variant: 'CASTING <hostName>' + slot extracted from text" <|
                \_ ->
                    promptToText (withCtx "a" [ ( "A:0099:wise-men", "Three Wandering Wise Men" ) ])
                        (ChooseCardPrompt
                            { pool = [ "A:0003:hand-card" ]
                            , host = Just "A:0099:wise-men"
                            , optional = False
                            , text = "pick a hand card to pay (slot 1)"
                            }
                        )
                        |> Expect.equal
                            "CASTING Three Wandering Wise Men \u{2014} pick a hand card to pay (slot 1)."
            , test "payment variant: host iid not in label table → falls back to 'a card'" <|
                \_ ->
                    promptToText (withCtx "a" [])
                        (ChooseCardPrompt
                            { pool = []
                            , host = Just "A:0099:unknown"
                            , optional = False
                            , text = "pick a hand card to pay"
                            }
                        )
                        |> Expect.equal "CASTING A:0099:unknown \u{2014} pick a hand card to pay."
            ]
        , describe "LoadingPrompt fallback"
            [ test "no JSON-encoded fallback; returns plain placeholder" <|
                \_ ->
                    promptToText noCtx LoadingPrompt
                        |> Expect.equal "Loading\u{2026}"
            ]
        ]
