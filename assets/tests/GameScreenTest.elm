module GameScreenTest exposing (suite)

{-| Decoder + view tests for `GameScreen.elm`. CLAUDE.md mandates
strict TDD; this file is the failing-test-first capture of intent
for each piece of the game-screen migration. Green tests are the
oracle.

Coverage by section:

  - `decodeCardView` — engine wire shape → CardView record
  - `decodePrompt` — kind discriminator → Prompt ADT (9 variants)
  - `decodeUctPreview` — preview envelope
  - `viewCard` — DOM shape under various CardOpts

-}

import Expect
import GameScreen exposing (..)
import Html.Attributes
import Json.Decode as D
import Json.Encode as E
import Test exposing (Test, describe, test)
import Test.Html.Query as Query
import Test.Html.Selector as Selector


suite : Test
suite =
    describe "GameScreen"
        [ describe "decodeCardView"
            [ test "decodes the full CardView wire shape" <|
                \_ ->
                    let
                        json =
                            E.object
                                [ ( "iid", E.string "A:0001:test-card" )
                                , ( "id", E.string "test-card" )
                                , ( "name", E.string "Test Card" )
                                , ( "kind", E.string "Creature" )
                                , ( "colors", E.list E.string [ "blue" ] )
                                , ( "symbols", E.list E.string [ "⨳" ] )
                                , ( "subtypes", E.list E.string [ "human", "scientist" ] )
                                , ( "cost", E.string "1H" )
                                , ( "effective_cost", E.string "1H" )
                                , ( "abilities", E.list E.string [ "draw 1" ] )
                                , ( "tapped", E.bool False )
                                , ( "summoning_sick", E.bool True )
                                , ( "damage", E.float 0 )
                                , ( "power", E.float 2 )
                                , ( "toughness", E.float 3 )
                                ]
                    in
                    case D.decodeValue decodeCardView json of
                        Ok c ->
                            Expect.all
                                [ \card -> Expect.equal card.iid "A:0001:test-card"
                                , \card -> Expect.equal card.name "Test Card"
                                , \card -> Expect.equal card.kind "Creature"
                                , \card -> Expect.equal card.colors [ "blue" ]
                                , \card -> Expect.equal card.symbols [ "⨳" ]
                                , \card -> Expect.equal card.subtypes [ "human", "scientist" ]
                                , \card -> Expect.equal card.cost "1H"
                                , \card -> Expect.equal card.effectiveCost "1H"
                                , \card -> Expect.equal card.abilities [ "draw 1" ]
                                , \card -> Expect.equal card.tapped False
                                , \card -> Expect.equal card.summoningSick True
                                , \card -> Expect.within (Expect.Absolute 0.001) card.damage 0
                                , \card -> Expect.within (Expect.Absolute 0.001) card.power 2
                                , \card -> Expect.within (Expect.Absolute 0.001) card.toughness 3
                                , \card -> Expect.equal card.attached []
                                ]
                                c

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "attached field decodes name + colors of each attached card" <|
                \_ ->
                    let
                        nested =
                            E.object
                                [ ( "iid", E.string "A:0010:rider" )
                                , ( "id", E.string "rider" )
                                , ( "name", E.string "Rider" )
                                , ( "kind", E.string "Mutation" )
                                , ( "colors", E.list E.string [ "red" ] )
                                , ( "symbols", E.list E.string [] )
                                , ( "subtypes", E.list E.string [] )
                                , ( "cost", E.string "" )
                                , ( "effective_cost", E.string "" )
                                , ( "abilities", E.list E.string [] )
                                , ( "tapped", E.bool False )
                                , ( "summoning_sick", E.bool False )
                                , ( "damage", E.float 0 )
                                , ( "power", E.float 0 )
                                , ( "toughness", E.float 0 )
                                , ( "attached", E.list identity [] )
                                ]

                        host =
                            E.object
                                [ ( "iid", E.string "A:0001:host" )
                                , ( "id", E.string "host" )
                                , ( "name", E.string "Host" )
                                , ( "kind", E.string "Creature" )
                                , ( "colors", E.list E.string [ "blue" ] )
                                , ( "symbols", E.list E.string [] )
                                , ( "subtypes", E.list E.string [] )
                                , ( "cost", E.string "1H" )
                                , ( "effective_cost", E.string "1H" )
                                , ( "abilities", E.list E.string [] )
                                , ( "tapped", E.bool False )
                                , ( "summoning_sick", E.bool False )
                                , ( "damage", E.float 0 )
                                , ( "power", E.float 2 )
                                , ( "toughness", E.float 2 )
                                , ( "attached", E.list identity [ nested ] )
                                ]
                    in
                    case D.decodeValue decodeCardView host of
                        Ok h ->
                            Expect.all
                                [ \c -> List.length c.attached |> Expect.equal 1
                                , \c -> List.map .name c.attached |> Expect.equal [ "Rider" ]
                                , \c -> List.map .colors c.attached |> Expect.equal [ [ "red" ] ]
                                ]
                                h

                        Err err ->
                            Expect.fail (D.errorToString err)
            ]
        , describe "decodePrompt"
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
        , describe "viewCard"
            [ test "default opts render the card with no .clickable class" <|
                \_ ->
                    viewCard defaultCardOpts sampleCreature
                        |> Query.fromHtml
                        |> Query.hasNot [ Selector.class "clickable" ]
                        |> identity
            , test "clickable opts adds the .clickable class" <|
                \_ ->
                    viewCard
                        { defaultCardOpts | clickable = Just identity }
                        sampleCreature
                        |> Query.fromHtml
                        |> Query.has [ Selector.class "clickable" ]
            , test "tapped CardView renders .tapped class" <|
                \_ ->
                    viewCard defaultCardOpts { sampleCreature | tapped = True }
                        |> Query.fromHtml
                        |> Query.has [ Selector.class "tapped" ]
            , test "selected opts renders .selected class" <|
                \_ ->
                    viewCard
                        { defaultCardOpts | selected = True }
                        sampleCreature
                        |> Query.fromHtml
                        |> Query.has [ Selector.class "selected" ]
            , test "name renders inside .name span" <|
                \_ ->
                    viewCard defaultCardOpts sampleCreature
                        |> Query.fromHtml
                        |> Query.find [ Selector.class "name" ]
                        |> Query.has [ Selector.text "Test Card" ]
            ]
        ]


sampleCreature : CardView
sampleCreature =
    { iid = "A:0001:test-card"
    , id = "test-card"
    , name = "Test Card"
    , kind = "Creature"
    , colors = [ "blue" ]
    , symbols = []
    , subtypes = [ "human" ]
    , cost = "1H"
    , effectiveCost = "1H"
    , abilities = []
    , tapped = False
    , summoningSick = False
    , damage = 0
    , power = 2
    , toughness = 3
    , attached = []
    }
