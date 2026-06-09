module CardTest exposing (suite)

{-| Test the unified `Card` primitive. Phase 1 coverage: decoder from
the in-game engine wire shape (CardView in `src/sim/snapshot.rs`),
including the recursive `attached` list. Render-mode tests come in
later phases as each mode is wired to its replacement site.
-}

import Card exposing (..)
import Expect
import Json.Decode as D
import Json.Encode as E
import Test exposing (Test, describe, test)


suite : Test
suite =
    describe "Card primitive"
        [ describe "decode from in-game wire shape"
            [ test "full CardView shape decodes into Card" <|
                \_ ->
                    case D.decodeValue Card.decode sampleCreatureJson of
                        Ok (Card d) ->
                            Expect.all
                                [ \data -> Expect.equal data.iid (Just "A:0001:blue-jewel")
                                , \data -> Expect.equal data.id "blue-jewel"
                                , \data -> Expect.equal data.name "Blue Jewel"
                                , \data -> Expect.equal data.kind Artifact
                                , \data -> Expect.equal data.colors [ "blue" ]
                                , \data -> Expect.equal (List.map .glyph data.symbols) [ "\u{2738}" ]
                                , \data -> Expect.equal (List.map .slot data.symbols) [ Center ]
                                , \data -> Expect.equal data.subtypes [ "jewel" ]
                                , \data -> Expect.equal data.printedCost ""
                                , \data -> Expect.equal data.tapped False
                                , \data -> Expect.equal data.summoningSick False
                                , \data -> Expect.equal data.attached []
                                ]
                                d

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "nested attached card decodes recursively" <|
                \_ ->
                    let
                        attachedJson =
                            wireCardJson
                                { iid = "A:0099:rider"
                                , id = "rider"
                                , name = "Rider"
                                , kind = "Mutation"
                                , colors = [ "red" ]
                                , symbols = []
                                , attached = []
                                }

                        hostJson =
                            wireCardJsonAttached
                                { iid = "A:0001:host"
                                , id = "host"
                                , name = "Host"
                                , kind = "Creature"
                                , colors = [ "blue" ]
                                , symbols = []
                                }
                                [ attachedJson ]
                    in
                    case D.decodeValue Card.decode hostJson of
                        Ok (Card host) ->
                            host.attached
                                |> List.map (\(Card a) -> a.name)
                                |> Expect.equal [ "Rider" ]

                        Err err ->
                            Expect.fail (D.errorToString err)
            , test "symbols default to spiral-out slot order (SLOTS.md)" <|
                \_ ->
                    case D.decodeValue Card.decode threeSymbolJson of
                        Ok (Card d) ->
                            List.map .slot d.symbols
                                |> Expect.equal [ Center, U, UR ]

                        Err err ->
                            Expect.fail (D.errorToString err)
            ]
        , describe "Kind"
            [ test "creature lowercase decodes to Creature" <|
                \_ -> kindFromString "Creature" |> Expect.equal Creature
            , test "spell lowercase decodes to Spell" <|
                \_ -> kindFromString "spell" |> Expect.equal Spell
            , test "unknown kind becomes OtherKind" <|
                \_ -> kindFromString "Bogus" |> Expect.equal (OtherKind "bogus")
            ]
        , describe "Slot"
            [ test "Center serialises to 'C'" <|
                \_ -> slotKey Center |> Expect.equal "C"
            , test "spiral order starts at C and goes clockwise" <|
                \_ ->
                    slotSpiralOrder
                        |> List.take 5
                        |> List.map slotKey
                        |> Expect.equal [ "C", "U", "UR", "R", "DR" ]
            ]
        ]


sampleCreatureJson : E.Value
sampleCreatureJson =
    wireCardJson
        { iid = "A:0001:blue-jewel"
        , id = "blue-jewel"
        , name = "Blue Jewel"
        , kind = "Artifact"
        , colors = [ "blue" ]
        , symbols = [ "\u{2738}" ]
        , attached = []
        }


threeSymbolJson : E.Value
threeSymbolJson =
    wireCardJson
        { iid = "A:0002:multi"
        , id = "multi"
        , name = "Multi-symbol"
        , kind = "Spell"
        , colors = []
        , symbols = [ "\u{2A33}", "\u{2A73}", "\u{0E5C}" ]
        , attached = []
        }


type alias CardSpec =
    { iid : String
    , id : String
    , name : String
    , kind : String
    , colors : List String
    , symbols : List String
    , attached : List E.Value
    }


wireCardJson : CardSpec -> E.Value
wireCardJson spec =
    E.object
        [ ( "iid", E.string spec.iid )
        , ( "id", E.string spec.id )
        , ( "name", E.string spec.name )
        , ( "kind", E.string spec.kind )
        , ( "colors", E.list E.string spec.colors )
        , ( "symbols", E.list E.string spec.symbols )
        , ( "subtypes", E.list E.string [ "jewel" ] )
        , ( "cost", E.string "" )
        , ( "effective_cost", E.string "" )
        , ( "abilities", E.list E.string [] )
        , ( "tapped", E.bool False )
        , ( "summoning_sick", E.bool False )
        , ( "damage", E.float 0 )
        , ( "power", E.float 0 )
        , ( "toughness", E.float 0 )
        , ( "attached", E.list identity spec.attached )
        ]


wireCardJsonAttached :
    { iid : String
    , id : String
    , name : String
    , kind : String
    , colors : List String
    , symbols : List String
    }
    -> List E.Value
    -> E.Value
wireCardJsonAttached spec attached =
    wireCardJson
        { iid = spec.iid
        , id = spec.id
        , name = spec.name
        , kind = spec.kind
        , colors = spec.colors
        , symbols = spec.symbols
        , attached = attached
        }
