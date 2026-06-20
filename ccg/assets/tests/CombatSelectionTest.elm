module CombatSelectionTest exposing (suite)

{-| TDD-first capture of the PickAttackers + PickBlocks state machine.
The JS-side selection logic lives in three `state.*` mutables today;
this test file pins the semantics we need before extracting them into
pure helpers under `GameScreen` for `Main.update` to delegate to.

Click semantics in PickBlocks (mirrors the original JS branch):

  - stage:      blockerPickFor was Nothing, click eligible blocker → blockerPickFor = Just iid
  - unstage:    blockerPickFor = Just iid, click same blocker → blockerPickFor = Nothing
  - unassign:   blocker iid is already a key in blocks, click it → remove from blocks
  - assign:     blockerPickFor = Just blkIid, click any attacker → blocks[blkIid] = atkIid + clear blockerPickFor

`toggleAttacker` is straightforward set-toggle.

`resetCombatSelection` is called after a Confirm/No-* action so the
next prompt-kind starts clean. Also called when the prompt transitions
OUT of PickAttackers/PickBlocks via GameStateReceived (handled in
Main; not pinned here).

-}

import Dict
import Expect
import GameScreen exposing (CombatSelection, assignAttackerToStaged, clickBlocker, emptyCombatSelection, resetCombatSelection, toggleAttacker)
import Set
import Test exposing (Test, describe, test)


suite : Test
suite =
    describe "CombatSelection state machine"
        [ describe "toggleAttacker"
            [ test "adds the iid when not present" <|
                \_ ->
                    emptyCombatSelection
                        |> toggleAttacker "A:0001:dragon"
                        |> .attackers
                        |> Expect.equal (Set.fromList [ "A:0001:dragon" ])
            , test "removes the iid when already present" <|
                \_ ->
                    { emptyCombatSelection | attackers = Set.fromList [ "A:0001:dragon" ] }
                        |> toggleAttacker "A:0001:dragon"
                        |> .attackers
                        |> Expect.equal Set.empty
            , test "toggling two different iids keeps both" <|
                \_ ->
                    emptyCombatSelection
                        |> toggleAttacker "A:0001:dragon"
                        |> toggleAttacker "A:0002:bear"
                        |> .attackers
                        |> Expect.equal (Set.fromList [ "A:0001:dragon", "A:0002:bear" ])
            ]
        , describe "clickBlocker"
            [ test "with no stage and no assignment → stages the blocker" <|
                \_ ->
                    emptyCombatSelection
                        |> clickBlocker "A:0010:knight"
                        |> .blockerPickFor
                        |> Expect.equal (Just "A:0010:knight")
            , test "with same blocker already staged → unstages" <|
                \_ ->
                    { emptyCombatSelection | blockerPickFor = Just "A:0010:knight" }
                        |> clickBlocker "A:0010:knight"
                        |> .blockerPickFor
                        |> Expect.equal Nothing
            , test "with a different blocker staged → re-stages to the new one" <|
                \_ ->
                    { emptyCombatSelection | blockerPickFor = Just "A:0010:knight" }
                        |> clickBlocker "A:0011:wolf"
                        |> .blockerPickFor
                        |> Expect.equal (Just "A:0011:wolf")
            , test "with the blocker already assigned → unassigns" <|
                \_ ->
                    { emptyCombatSelection
                        | blocks = Dict.fromList [ ( "A:0010:knight", "B:0001:atk" ) ]
                    }
                        |> clickBlocker "A:0010:knight"
                        |> .blocks
                        |> Expect.equal Dict.empty
            , test "unassigning preserves other assignments" <|
                \_ ->
                    { emptyCombatSelection
                        | blocks =
                            Dict.fromList
                                [ ( "A:0010:knight", "B:0001:atk" )
                                , ( "A:0011:wolf", "B:0002:atk" )
                                ]
                    }
                        |> clickBlocker "A:0010:knight"
                        |> .blocks
                        |> Expect.equal (Dict.fromList [ ( "A:0011:wolf", "B:0002:atk" ) ])
            ]
        , describe "assignAttackerToStaged"
            [ test "with a blocker staged → assigns that blocker to the attacker + clears stage" <|
                \_ ->
                    let
                        result =
                            { emptyCombatSelection | blockerPickFor = Just "A:0010:knight" }
                                |> assignAttackerToStaged "B:0001:atk"
                    in
                    Expect.all
                        [ \r -> Expect.equal r.blocks (Dict.fromList [ ( "A:0010:knight", "B:0001:atk" ) ])
                        , \r -> Expect.equal r.blockerPickFor Nothing
                        ]
                        result
            , test "no-op when no blocker is staged" <|
                \_ ->
                    let
                        before =
                            { emptyCombatSelection | blocks = Dict.fromList [ ( "X", "Y" ) ] }
                    in
                    before
                        |> assignAttackerToStaged "B:0001:atk"
                        |> Expect.equal before
            , test "multiple blockers can share the same attacker (gang-block)" <|
                \_ ->
                    emptyCombatSelection
                        |> clickBlocker "A:0010:knight"
                        |> assignAttackerToStaged "B:0001:atk"
                        |> clickBlocker "A:0011:wolf"
                        |> assignAttackerToStaged "B:0001:atk"
                        |> .blocks
                        |> Expect.equal
                            (Dict.fromList
                                [ ( "A:0010:knight", "B:0001:atk" )
                                , ( "A:0011:wolf", "B:0001:atk" )
                                ]
                            )
            ]
        , describe "resetCombatSelection"
            [ test "clears attackers, blocks, and blockerPickFor" <|
                \_ ->
                    let
                        dirty : CombatSelection
                        dirty =
                            { attackers = Set.fromList [ "A:0001:dragon" ]
                            , blocks = Dict.fromList [ ( "A:0010:knight", "B:0001:atk" ) ]
                            , blockerPickFor = Just "A:0011:wolf"
                            }
                    in
                    resetCombatSelection dirty
                        |> Expect.equal emptyCombatSelection
            ]
        ]
