module GameScreenTest exposing (suite)

{-| Prompt decoder + helper tests for `GameScreen.elm`. Card-related
tests (decodeCardView, viewCard) moved into `CardTest.elm` when the
card render path consolidated into the `Card` primitive — those types
no longer live in `GameScreen`.

Surviving here:

  - `decodePrompt` — kind discriminator → Prompt ADT (9 variants)
  - `promptKindKey` — reverse lookup
-}

import Expect
import GameScreen exposing (..)
import Json.Decode as D
import Json.Encode as E
import Test exposing (Test, describe, test)


suite : Test
suite =
    describe "GameScreen prompt decoding"
        [ describe "decodePrompt"
            [ test "Spectate variant decodes turn/phase/active_player + optional winner/at_end" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "kind", E.string "Spectate" )
                                , ( "turn", E.int 5 )
                                , ( "phase", E.string "Main1" )
                                , ( "active_player", E.string "a" )
                                , ( "at_end", E.bool True )
                                , ( "winner", E.string "a" )
                                ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok (SpectatePrompt d) ->
                            Expect.all
                                [ \data -> Expect.equal data.turn 5
                                , \data -> Expect.equal data.phase "Main1"
                                , \data -> Expect.equal data.activePlayer "a"
                                , \data -> Expect.equal data.atEnd True
                                , \data -> Expect.equal data.winner (Just "a")
                                ]
                                d

                        Ok other ->
                            Expect.fail ("expected SpectatePrompt; got " ++ promptKindKey other)

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "PickCard variant decodes candidates + activations" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "kind", E.string "PickCard" )
                                , ( "candidates", E.list E.string [ "A:0001:x", "A:0002:y" ] )
                                , ( "activations", E.list identity [] )
                                ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok (PickCardPrompt d) ->
                            Expect.equal d.candidates [ "A:0001:x", "A:0002:y" ]

                        Ok other ->
                            Expect.fail ("expected PickCardPrompt; got " ++ promptKindKey other)

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "ChooseCard variant decodes pool/host/optional/prompt" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "kind", E.string "ChooseCard" )
                                , ( "pool", E.list E.string [ "A:0003:hand-card" ] )
                                , ( "host", E.string "A:0099:wise-men" )
                                , ( "optional", E.bool False )
                                , ( "prompt", E.string "pick a hand card to pay (slot 1)" )
                                ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok (ChooseCardPrompt d) ->
                            Expect.all
                                [ \data -> Expect.equal data.pool [ "A:0003:hand-card" ]
                                , \data -> Expect.equal data.host (Just "A:0099:wise-men")
                                , \data -> Expect.equal data.optional False
                                , \data -> Expect.equal data.text "pick a hand card to pay (slot 1)"
                                ]
                                d

                        Ok other ->
                            Expect.fail ("expected ChooseCardPrompt; got " ++ promptKindKey other)

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "PickAttackers decodes eligible list" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "kind", E.string "PickAttackers" )
                                , ( "eligible", E.list E.string [ "A:0001:c1", "A:0002:c2" ] )
                                ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok (PickAttackersPrompt d) ->
                            Expect.equal d.eligible [ "A:0001:c1", "A:0002:c2" ]

                        _ ->
                            Expect.fail "expected PickAttackersPrompt"
            , test "PickBlocks decodes attackers + eligible_blockers" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "kind", E.string "PickBlocks" )
                                , ( "attackers", E.list E.string [ "B:0001:atk" ] )
                                , ( "eligible_blockers", E.list E.string [ "A:0001:blk" ] )
                                ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok (PickBlocksPrompt d) ->
                            Expect.all
                                [ \data -> Expect.equal data.attackers [ "B:0001:atk" ]
                                , \data -> Expect.equal data.eligibleBlockers [ "A:0001:blk" ]
                                ]
                                d

                        _ ->
                            Expect.fail "expected PickBlocksPrompt"
            , test "unknown kind decodes to LoadingPrompt fallback" <|
                \_ ->
                    let
                        json =
                            E.object [ ( "kind", E.string "Bogus" ) ]
                    in
                    case D.decodeValue decodePrompt json of
                        Ok LoadingPrompt ->
                            Expect.pass

                        Ok other ->
                            Expect.fail ("expected LoadingPrompt; got " ++ promptKindKey other)

                        Err err ->
                            Expect.fail (D.errorToString err)
            ]
        ]
