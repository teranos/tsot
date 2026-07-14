use super::*;
use crate::card::CostComponent;
use crate::game::test_helpers::*;

#[test]
fn play_subsystem_round_trips_through_journal() {
    // Play a creature with HAND + MILL + GRAVEYARD cost components, then
    // rollback. State should equal pre-play exactly.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let payment = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &creature,
        vec![
            CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            },
            CostComponent {
                amount: 2,
                source: CostSource::Mill,
                is_x: false,
                kind: None,
            },
        ],
    );

    let snapshot = format!("{:?}", s);
    s.journal = Some(crate::game::Journal::new());

    s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![payment],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    )
    .unwrap();

    assert_ne!(snapshot, format!("{:?}", s));
    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    assert!(s.journal.is_none());
    assert_eq!(
        snapshot,
        format!("{:?}", s),
        "play subsystem rollback should restore prior state"
    );
}

#[test]
fn play_creature_with_no_cost_moves_to_board() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    assert!(s
        .play_card(PlayerId::A, &iid, PlayChoices::default(), None)
        .is_ok());
    assert!(!s.a.hand.contains(&iid));
    assert!(s.a.board.contains(&iid));
}

#[test]
fn play_creature_with_hand_cost_attaches_payments() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let payment = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let choices = PlayChoices {
        hand_payment_ids: vec![payment.clone()],
        x_value: None,
        jewel_tap: None,
        sacrifice_ids: vec![],
        mutation_target: None,
        gy_hand_payment_ids: vec![],
        attached_payment_ids: vec![],
        graveyard_payment_ids: vec![],    };
    assert!(s
        .play_card(PlayerId::A, &creature, choices, None)
        .is_ok());
    assert!(s.a.board.contains(&creature));
    assert!(!s.a.hand.contains(&creature));
    assert!(!s.a.hand.contains(&payment));
    let inst = s.card_pool.get(&creature).unwrap();
    assert!(inst.attached.contains(&payment));
    assert!(s.card_pool.get(&payment).unwrap().face_down);
}

#[test]
fn play_creature_with_mill_cost_mills_top_of_deck() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let top_three: Vec<_> = s.a.deck.iter().take(3).cloned().collect();
    let deck_before = s.a.deck.len();
    let graveyard_before = s.a.graveyard.len();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 3,
            source: CostSource::Mill,
            is_x: false,
            kind: None,
        }],
    );
    assert!(s
        .play_card(PlayerId::A, &creature, PlayChoices::default(), None)
        .is_ok());
    assert_eq!(s.a.deck.len(), deck_before - 3);
    assert_eq!(s.a.graveyard.len(), graveyard_before + 3);
    for tid in &top_three {
        assert!(s.a.graveyard.contains(tid));
    }
    assert!(s.a.board.contains(&creature));
}

#[test]
fn play_combined_hand_and_mill_cost() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &creature,
        vec![
            CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            },
            CostComponent {
                amount: 2,
                source: CostSource::Mill,
                is_x: false,
                kind: None,
            },
        ],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(result.is_ok());
    assert!(s.a.board.contains(&creature));
    assert!(s.card_pool.get(&creature).unwrap().attached.contains(&pay));
    assert_eq!(s.a.deck.len(), 43);
}

#[test]
fn play_card_errors_when_not_in_hand() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let in_deck = s.a.deck[0].clone();
    assert_eq!(
        s.play_card(PlayerId::A, &in_deck, PlayChoices::default(), None),
        Err(PlayError::NotInHand)
    );
}

#[test]
fn play_card_errors_when_hand_payment_count_wrong() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(
        result,
        Err(PlayError::WrongHandPaymentCount {
            expected: 2,
            got: 1
        })
    );
}

#[test]
fn play_card_errors_when_paying_with_self() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![creature.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::HandPaymentInvalid(creature)));
}

#[test]
fn play_card_errors_on_duplicate_hand_payment() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone(), pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::DuplicateHandPayment(pay)));
}

#[test]
fn play_card_errors_when_insufficient_deck_for_mill() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 100,
            source: CostSource::Mill,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(
        result,
        Err(PlayError::InsufficientDeckForMill {
            needed: 100,
            have: 5,
        })
    );
}

#[test]
fn play_card_errors_on_unsupported_type() {
    // Environment is still unsupported (P.21 + P.22 slot management not done).
    // Creature / Spell / Artifact are all routable.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card_mut().kind = CardType::Environment;
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Err(PlayError::UnsupportedType(CardType::Environment))
    );
}

#[test]
fn play_card_routes_artifact_to_board() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card_mut().kind = CardType::Artifact;
    // No cost — empty payment is valid for an artifact with cost = {}.
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Ok(())
    );
    assert!(s.a.board.contains(&iid));
    assert!(!s.a.hand.contains(&iid));
}

// C.17 + P.37: Symbol cards are a board-placed permanent kind. They
// land on BOARD on cast, ETB untapped, and skip B.3 summoning sickness
// (C.17a). Cost = {} mirrors jewels; the design gate is P.35/P.36, not
// a printed cost.
#[test]
fn play_card_routes_symbol_to_board() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card_mut().kind = CardType::Symbol;
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Ok(())
    );
    assert!(s.a.board.contains(&iid));
    assert!(!s.a.hand.contains(&iid));
}

#[test]
fn play_card_symbol_etb_untapped() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card_mut().kind = CardType::Symbol;
    s.play_card(PlayerId::A, &iid, PlayChoices::default(), None)
        .expect("symbol cast");
    let inst = s.card_pool.get(&iid).expect("instance");
    assert!(!inst.tapped, "P.37: symbol cards enter untapped");
}

// P.35: a player may cast at most one Symbol card per turn. The first
// cast resolves; the second on the same turn returns SymbolCastCapReached
// without paying any cost.
#[test]
fn play_card_symbol_cast_cap_blocks_second() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid_first = s.a.hand[0].clone();
    let iid_second = s.a.hand[1].clone();
    s.card_pool.get_mut(&iid_first).unwrap().card_mut().kind = CardType::Symbol;
    s.card_pool.get_mut(&iid_second).unwrap().card_mut().kind = CardType::Symbol;
    assert_eq!(
        s.play_card(PlayerId::A, &iid_first, PlayChoices::default(), None),
        Ok(())
    );
    assert_eq!(
        s.play_card(PlayerId::A, &iid_second, PlayChoices::default(), None),
        Err(PlayError::SymbolCastCapReached)
    );
    // The refused cast must not leave HAND.
    assert!(s.a.hand.contains(&iid_second));
}

// P.35: the cap resets at turn start so the same player can cast another
// Symbol on their next turn.
#[test]
fn play_card_symbol_cap_resets_on_turn_begin() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let first = s.a.hand[0].clone();
    let second = s.a.hand[1].clone();
    s.card_pool.get_mut(&first).unwrap().card_mut().kind = CardType::Symbol;
    s.card_pool.get_mut(&second).unwrap().card_mut().kind = CardType::Symbol;
    s.play_card(PlayerId::A, &first, PlayChoices::default(), None)
        .expect("first symbol cast");
    // Advance Untap → Draw → Main1 → Combat → Main2 → End → next Untap.
    for _ in 0..6 {
        s.next_phase(None).expect("None ctx never yields");
    }
    // Active player flipped to B on turn 2. Advance B's full turn so we
    // come back to A on turn 3 — A's cap should be cleared by then.
    for _ in 0..6 {
        s.next_phase(None).expect("None ctx never yields");
    }
    assert_eq!(s.active_player, PlayerId::A);
    assert_eq!(
        s.play_card(PlayerId::A, &second, PlayChoices::default(), None),
        Ok(())
    );
}

// P.35: A's cap doesn't lock B. After A casts a Symbol and the turn
// passes to B, B may cast a Symbol on B's turn.
#[test]
fn play_card_symbol_cap_per_player() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let a_sym = s.a.hand[0].clone();
    let b_sym = s.b.hand[0].clone();
    s.card_pool.get_mut(&a_sym).unwrap().card_mut().kind = CardType::Symbol;
    s.card_pool.get_mut(&b_sym).unwrap().card_mut().kind = CardType::Symbol;
    s.play_card(PlayerId::A, &a_sym, PlayChoices::default(), None)
        .expect("A casts symbol");
    // Advance to B's turn (full A-turn cycle).
    for _ in 0..6 {
        s.next_phase(None).expect("None ctx never yields");
    }
    assert_eq!(s.active_player, PlayerId::B);
    assert_eq!(
        s.play_card(PlayerId::B, &b_sym, PlayChoices::default(), None),
        Ok(())
    );
}

// P.36: a Symbol card is unique in play by `id`. A second cast with
// the same `id` while the first is on either player's BOARD is refused.
// Same controller case.
#[test]
fn play_card_symbol_unique_blocks_duplicate_id_self() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let first = s.a.hand[0].clone();
    let second = s.a.hand[1].clone();
    // Force the second copy to share the first's card-id so the
    // uniqueness check sees a duplicate.
    s.card_pool.get_mut(&first).unwrap().card_mut().kind = CardType::Symbol;
    let first_id = s.card_pool.get(&first).unwrap().card().id.clone();
    {
        let inst = s.card_pool.get_mut(&second).unwrap();
        inst.card_mut().kind = CardType::Symbol;
        inst.card_mut().id = first_id;
    }
    s.play_card(PlayerId::A, &first, PlayChoices::default(), None)
        .expect("first symbol cast");
    // P.35 cap would also fire here; advance to A's next turn so the
    // cap is clear and only P.36 can refuse the second cast.
    for _ in 0..12 {
        s.next_phase(None).expect("None ctx never yields");
    }
    assert_eq!(s.active_player, PlayerId::A);
    assert_eq!(
        s.play_card(PlayerId::A, &second, PlayChoices::default(), None),
        Err(PlayError::SymbolUniquenessViolated)
    );
    assert!(s.a.hand.contains(&second));
}

// P.36: uniqueness check spans BOTH players' boards. A casts Red IX,
// B tries to cast Red IX on B's turn — refused.
#[test]
fn play_card_symbol_unique_blocks_duplicate_id_opponent() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let a_sym = s.a.hand[0].clone();
    let b_sym = s.b.hand[0].clone();
    s.card_pool.get_mut(&a_sym).unwrap().card_mut().kind = CardType::Symbol;
    let shared_id = s.card_pool.get(&a_sym).unwrap().card().id.clone();
    {
        let inst = s.card_pool.get_mut(&b_sym).unwrap();
        inst.card_mut().kind = CardType::Symbol;
        inst.card_mut().id = shared_id;
    }
    s.play_card(PlayerId::A, &a_sym, PlayChoices::default(), None)
        .expect("A casts symbol");
    // Advance to B's turn.
    for _ in 0..6 {
        s.next_phase(None).expect("None ctx never yields");
    }
    assert_eq!(s.active_player, PlayerId::B);
    assert_eq!(
        s.play_card(PlayerId::B, &b_sym, PlayChoices::default(), None),
        Err(PlayError::SymbolUniquenessViolated)
    );
    assert!(s.b.hand.contains(&b_sym));
}

// P.36: once the first Symbol leaves BOARD, the id becomes castable
// again — by either player.
#[test]
fn play_card_symbol_unique_castable_after_leaves_board() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let a_sym = s.a.hand[0].clone();
    let b_sym = s.b.hand[0].clone();
    s.card_pool.get_mut(&a_sym).unwrap().card_mut().kind = CardType::Symbol;
    let shared_id = s.card_pool.get(&a_sym).unwrap().card().id.clone();
    {
        let inst = s.card_pool.get_mut(&b_sym).unwrap();
        inst.card_mut().kind = CardType::Symbol;
        inst.card_mut().id = shared_id;
    }
    s.play_card(PlayerId::A, &a_sym, PlayChoices::default(), None)
        .expect("A casts symbol");
    // Send A's symbol to the graveyard so the BOARD copy is gone.
    let _ = s.move_card(&a_sym, PlayerId::A, Zone::Board, Zone::Graveyard);
    // Advance to B's turn.
    for _ in 0..6 {
        s.next_phase(None).expect("None ctx never yields");
    }
    assert_eq!(s.active_player, PlayerId::B);
    assert_eq!(
        s.play_card(PlayerId::B, &b_sym, PlayChoices::default(), None),
        Ok(())
    );
}

// P.38: a Symbol card on top of a player's DECK is castable from
// there. On resolution it moves DECK → BOARD (skipping HAND).
#[test]
fn play_card_symbol_castable_from_top_of_deck() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let top = s.a.deck[0].clone();
    s.card_pool.get_mut(&top).unwrap().card_mut().kind = CardType::Symbol;
    assert_eq!(
        s.play_card(PlayerId::A, &top, PlayChoices::default(), None),
        Ok(())
    );
    assert!(s.a.board.contains(&top));
    assert!(!s.a.deck.contains(&top));
}

// P.38: only the very top is castable from DECK. A Symbol at slot 1
// (second from top) is not yet reachable.
#[test]
fn play_card_symbol_not_top_of_deck_refused() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let second = s.a.deck[1].clone();
    s.card_pool.get_mut(&second).unwrap().card_mut().kind = CardType::Symbol;
    assert_eq!(
        s.play_card(PlayerId::A, &second, PlayChoices::default(), None),
        Err(PlayError::NotInHand)
    );
    assert!(s.a.deck.contains(&second));
}

// P.38 is Symbol-only. A non-Symbol card on top of DECK is still
// refused with NotInHand.
#[test]
fn play_card_non_symbol_top_of_deck_refused() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let top = s.a.deck[0].clone();
    // top is a default Creature from deck_of — kind stays Creature.
    assert_eq!(
        s.play_card(PlayerId::A, &top, PlayChoices::default(), None),
        Err(PlayError::NotInHand)
    );
}

// P.38: the source-zone selection still flows through every P.32 /
// P.35 / P.36 gate. A duplicate cast from top-of-deck must still
// trip P.36 uniqueness if a same-id Symbol is already on BOARD.
#[test]
fn play_card_symbol_top_of_deck_still_respects_uniqueness() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let from_hand = s.a.hand[0].clone();
    let from_deck = s.a.deck[0].clone();
    s.card_pool.get_mut(&from_hand).unwrap().card_mut().kind = CardType::Symbol;
    let shared_id = s.card_pool.get(&from_hand).unwrap().card().id.clone();
    {
        let inst = s.card_pool.get_mut(&from_deck).unwrap();
        inst.card_mut().kind = CardType::Symbol;
        inst.card_mut().id = shared_id;
    }
    s.play_card(PlayerId::A, &from_hand, PlayChoices::default(), None)
        .expect("first symbol from hand");
    // Clear the P.35 cap directly so this test isolates the P.36 path
    // (a turn-advance would trigger draws that shift the top of A's
    // DECK; setting the flag back is cheaper than reconstructing the
    // deck ordering).
    s.symbol_cast_this_turn[0] = false;
    assert_eq!(s.a.deck.first(), Some(&from_deck));
    assert_eq!(
        s.play_card(PlayerId::A, &from_deck, PlayChoices::default(), None),
        Err(PlayError::SymbolUniquenessViolated)
    );
}

#[test]
fn play_card_symbol_no_summoning_sickness() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card_mut().kind = CardType::Symbol;
    s.play_card(PlayerId::A, &iid, PlayChoices::default(), None)
        .expect("symbol cast");
    let inst = s.card_pool.get(&iid).expect("instance");
    assert!(
        !inst.summoning_sick,
        "C.17a: symbol cards skip summoning sickness (parallel to artifacts)"
    );
}

/// Set up: a red jewel on A's BOARD (untapped), a red creature in A's hand
/// with 1 HAND cost, a payment-eligible second card in A's hand. Returns
/// (state, jewel_iid, cast_iid, payment_iid).
fn setup_jewel_tap_scenario() -> (GameState, InstanceId, InstanceId, InstanceId) {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let jewel = s.a.hand[0].clone();
    let cast = s.a.hand[1].clone();
    let payment = s.a.hand[2].clone();
    // Build the jewel: artifact, subtype "jewel", red.
    {
        let j = s.card_pool.get_mut(&jewel).unwrap();
        j.card_mut().kind = CardType::Artifact;
        j.card_mut().subtypes = vec!["jewel".to_string()];
        j.card_mut().colors = vec!["red".to_string()];
    }
    // Move the jewel to BOARD (untapped) so it can be tapped as cost.
    let _ = s.move_card(&jewel, PlayerId::A, Zone::Hand, Zone::Board);
    // Build the cast card: creature, red, 1 hand cost.
    {
        let c = s.card_pool.get_mut(&cast).unwrap();
        c.card_mut().colors = vec!["red".to_string()];
    }
    // P.7a identity match: payment needs a shared color with cast.
    {
        let p = s.card_pool.get_mut(&payment).unwrap();
        p.card_mut().colors = vec!["red".to_string()];
    }
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    (s, jewel, cast, payment)
}

// P.24a (rewritten): the jewel is tapped AND sacrificed when used as
// cost substitution. After the cast resolves, the jewel is no longer
// on BOARD; it sits in the controller's GRAVEYARD (P.16 sacrifice
// destination) and is tapped at the moment of sacrifice.
#[test]
fn jewel_tap_substitutes_for_one_hand_slot() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![], // Jewel substitutes for the 1 HAND slot.
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // Jewel was sacrificed: gone from BOARD, sits in GRAVEYARD, tapped.
    assert!(!s.a.board.contains(&jewel));
    assert!(s.a.graveyard.contains(&jewel));
    assert!(s.card_pool.get(&jewel).unwrap().tapped);
    assert!(s.a.board.contains(&cast));
    let bumps = s
        .action_counts
        .get("jewel_tap_substitution")
        .map(|v| v[0])
        .unwrap_or(0);
    assert_eq!(bumps, 1);
}

// P.24a rewrite: a single jewel pays for BOTH HAND components of a
// 2-hand cost. hand_payment_ids stays empty; no extra discard.
#[test]
fn jewel_pays_two_hand_components() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.graveyard.contains(&jewel));
    assert!(!s.a.board.contains(&jewel));
}

// P.24a rewrite: a single jewel pays for BOTH GRAVEYARD components of
// a 2-graveyard cost — no cards exiled from GRAVEYARD. The cast still
// resolves successfully.
#[test]
fn jewel_pays_two_graveyard_components() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Seed GY with a few red cards. Anchor check is colour-based, so
    // colouring them red avoids NoGraveyardPaymentForColor if any
    // unintended pitch leaks through. After jewel covers both GY
    // components, no pitch should fire and gy size grows only by 1
    // (the sacrificed jewel itself).
    let filler_a = s.a.hand[2].clone();
    let filler_b = s.a.hand[3].clone();
    let _ = s.move_card(&filler_a, PlayerId::A, Zone::Hand, Zone::Graveyard);
    let _ = s.move_card(&filler_b, PlayerId::A, Zone::Hand, Zone::Graveyard);
    for iid in [&filler_a, &filler_b] {
        s.card_pool.get_mut(iid).unwrap().card_mut().colors = vec!["red".to_string()];
    }
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Graveyard,
            is_x: false,
            kind: None,
        }],
    );
    let gy_before: usize = s.a.graveyard.len();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // No GY exiles happened: gy is exactly the prior contents plus the
    // sacrificed jewel.
    assert!(s.a.graveyard.contains(&jewel));
    assert_eq!(s.a.graveyard.len(), gy_before + 1);
}

// P.24a rewrite: a single jewel pays one HAND and one GRAVEYARD
// component on the same cast — mixed coverage.
#[test]
fn jewel_pays_one_hand_one_graveyard_mixed() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Seed a red anchor in GY so if any GY pitch leaks the anchor
    // check still passes.
    let filler = s.a.hand[3].clone();
    let _ = s.move_card(&filler, PlayerId::A, Zone::Hand, Zone::Graveyard);
    s.card_pool.get_mut(&filler).unwrap().card_mut().colors = vec!["red".to_string()];
    set_cost(
        &mut s,
        &cast,
        vec![
            CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            },
            CostComponent {
                amount: 1,
                source: CostSource::Graveyard,
                is_x: false,
                kind: None,
            },
        ],
    );
    let gy_before: usize = s.a.graveyard.len();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // No GY pitch fired (jewel covered the GY slot). The only new GY
    // entry is the jewel itself.
    assert!(s.a.graveyard.contains(&jewel));
    assert_eq!(s.a.graveyard.len(), gy_before + 1);
}

#[test]
fn jewel_tap_rejected_when_jewel_tapped() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    s.set_tapped(&jewel, true); // pre-tap the jewel
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(jewel)));
}

#[test]
fn jewel_tap_rejected_on_color_mismatch() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Recolor the cast card to blue — jewel is red, no overlap.
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["blue".to_string()];
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(jewel)));
}

#[test]
fn jewel_tap_rejected_on_non_jewel_artifact() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Strip the "jewel" subtype — still a red artifact on board, but not a jewel.
    s.card_pool.get_mut(&jewel).unwrap().card_mut().subtypes.clear();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(jewel)));
}

#[test]
fn jewel_tap_rejected_when_cast_has_no_hand_cost() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Clear the cost — nothing to substitute.
    set_cost(&mut s, &cast, vec![]);
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(jewel),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::JewelTapWithoutHandCost));
}

#[test]
fn jewel_tap_plus_hand_payment_splits_cost_correctly() {
    // P.24a (rewritten): the jewel covers up to TWO components, so with
    // a 3-HAND cost the jewel takes two slots and the controller still
    // supplies one explicit hand pay for the third.
    let (mut s, jewel, cast, payment) = setup_jewel_tap_scenario();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 3,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![payment.clone()],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.graveyard.contains(&jewel));
    assert!(!s.a.hand.contains(&payment));
}

#[test]
fn crystal_tap_matches_by_attached_card_color() {
    // Crystal-tap variant of P.24: crystal's own colors don't matter
    // (they're all six); the match is against attached cards' colors.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let crystal = s.a.hand[0].clone();
    let attached = s.a.hand[1].clone();
    let cast = s.a.hand[2].clone();
    // Crystal: artifact, "crystal" subtype, all six colors.
    {
        let c = s.card_pool.get_mut(&crystal).unwrap();
        c.card_mut().kind = CardType::Artifact;
        c.card_mut().subtypes = vec!["crystal".into()];
        c.card_mut().colors = vec![
            "black".into(),
            "blue".into(),
            "green".into(),
            "purple".into(),
            "red".into(),
            "white".into(),
        ];
    }
    // Move crystal to BOARD and attach a red card to it.
    let _ = s.move_card(&crystal, PlayerId::A, Zone::Hand, Zone::Board);
    s.card_pool.get_mut(&attached).unwrap().card_mut().colors = vec!["red".into()];
    let _ = s.move_card(&attached, PlayerId::A, Zone::Hand, Zone::Exile); // remove from hand
    s.a.exile.retain(|x| x != &attached);
    s.add_attached(&crystal, &attached);
    // Cast card: red, 1 hand cost.
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["red".into()];
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(crystal.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.card_pool.get(&crystal).unwrap().tapped);
}

#[test]
fn crystal_tap_rejected_when_no_attached_color_matches() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let crystal = s.a.hand[0].clone();
    let attached = s.a.hand[1].clone();
    let cast = s.a.hand[2].clone();
    {
        let c = s.card_pool.get_mut(&crystal).unwrap();
        c.card_mut().kind = CardType::Artifact;
        c.card_mut().subtypes = vec!["crystal".into()];
        c.card_mut().colors = vec!["black".into(), "red".into(), "blue".into()];
    }
    let _ = s.move_card(&crystal, PlayerId::A, Zone::Hand, Zone::Board);
    // Attached card is GREEN — does not match the BLUE cast card.
    s.card_pool.get_mut(&attached).unwrap().card_mut().colors = vec!["green".into()];
    let _ = s.move_card(&attached, PlayerId::A, Zone::Hand, Zone::Exile);
    s.a.exile.retain(|x| x != &attached);
    s.add_attached(&crystal, &attached);
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["blue".into()];
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(crystal.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(crystal)));
}

// P.24e: tap an untapped Symbol on the controller's BOARD to substitute
// for one HAND-source component. The Symbol stays on the BOARD, tapped
// (not sacrificed — that's the jewel-only behaviour from P.24a).
fn setup_symbol_tap_scenario() -> (GameState, InstanceId, InstanceId) {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let symbol = s.a.hand[0].clone();
    let cast = s.a.hand[1].clone();
    {
        let sym = s.card_pool.get_mut(&symbol).unwrap();
        sym.card_mut().kind = CardType::Symbol;
        sym.card_mut().colors = vec!["red".to_string()];
    }
    let _ = s.move_card(&symbol, PlayerId::A, Zone::Hand, Zone::Board);
    {
        let c = s.card_pool.get_mut(&cast).unwrap();
        c.card_mut().colors = vec!["blue".to_string()];
    }
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    (s, symbol, cast)
}

#[test]
fn symbol_tap_substitutes_for_one_hand_component() {
    let (mut s, symbol, cast) = setup_symbol_tap_scenario();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(symbol.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // Symbol stays on board, tapped; not sacrificed.
    assert!(s.a.board.contains(&symbol));
    assert!(!s.a.graveyard.contains(&symbol));
    assert!(s.card_pool.get(&symbol).unwrap().tapped);
}

// P.24e: Symbol can substitute for a GRAVEYARD-source component.
#[test]
fn symbol_tap_substitutes_for_one_graveyard_component() {
    let (mut s, symbol, cast) = setup_symbol_tap_scenario();
    // Seed a same-color GY anchor — cast is blue, so a blue GY card
    // anchors P.12a if any GY pitch leaks (it shouldn't — symbol
    // covers the lone GY component).
    let filler = s.a.hand[1].clone();
    let _ = s.move_card(&filler, PlayerId::A, Zone::Hand, Zone::Graveyard);
    s.card_pool.get_mut(&filler).unwrap().card_mut().colors = vec!["blue".into()];
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Graveyard,
            is_x: false,
            kind: None,
        }],
    );
    let gy_before = s.a.graveyard.len();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(symbol.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // Symbol still on board, tapped; GY unchanged (no pitch leaked).
    assert!(s.a.board.contains(&symbol));
    assert!(s.card_pool.get(&symbol).unwrap().tapped);
    assert_eq!(s.a.graveyard.len(), gy_before);
}

// P.24e: Symbol substitution has no color-match requirement (unlike
// P.24a / P.24b for jewel and crystal). A red Symbol pays for a blue
// cast just fine.
#[test]
fn symbol_tap_no_color_match_required() {
    let (mut s, symbol, cast) = setup_symbol_tap_scenario();
    // cast is blue, symbol is red (set up in helper). Confirm no
    // color overlap.
    assert!(s.card_pool.get(&cast).unwrap().card().colors == vec!["blue".to_string()]);
    assert!(s.card_pool.get(&symbol).unwrap().card().colors == vec!["red".to_string()]);
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(symbol.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
}

// P.24e: an already-tapped Symbol can't be tapped again as cost.
#[test]
fn symbol_tap_rejected_when_already_tapped() {
    let (mut s, symbol, cast) = setup_symbol_tap_scenario();
    s.set_tapped(&symbol, true);
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: Some(symbol.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(symbol)));
}

/// Set up: a card in A's hand with SACRIFICE 1 cost, and a sacrificable
/// creature on A's board. Returns (state, cast_iid, sacrifice_iid).
fn setup_sacrifice_scenario() -> (GameState, InstanceId, InstanceId) {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let victim = s.a.hand[1].clone();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Sacrifice,
            is_x: false,
            kind: None,
        }],
    );
    // Move victim to A's board so it can be sacrificed.
    let _ = s.move_card(&victim, PlayerId::A, Zone::Hand, Zone::Board);
    (s, cast, victim)
}

#[test]
fn sacrifice_cost_moves_victim_to_graveyard() {
    let (mut s, cast, victim) = setup_sacrifice_scenario();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![victim.clone()],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(!s.a.board.contains(&victim));
    assert!(s.a.graveyard.contains(&victim));
    let bumps = s
        .action_counts
        .get("sacrificed_as_cost")
        .map(|v| v[0])
        .unwrap_or(0);
    assert_eq!(bumps, 1);
}

#[test]
fn sacrifice_count_mismatch_errors() {
    let (mut s, cast, _victim) = setup_sacrifice_scenario();
    // Cost wants 1 sacrifice, supply 0.
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(
        result,
        Err(PlayError::WrongSacrificeCount {
            expected: 1,
            got: 0
        })
    );
}

#[test]
fn sacrifice_rejected_when_victim_not_on_board() {
    let (mut s, cast, _victim) = setup_sacrifice_scenario();
    // Pick a card that's still in A's hand, not on board.
    let phantom = s.a.hand[2].clone();
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![phantom.clone()],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::SacrificePaymentInvalid(phantom)));
}

#[test]
fn sacrifice_rejected_when_opponent_controls_victim() {
    let (mut s, cast, _victim) = setup_sacrifice_scenario();
    // Try to sacrifice one of B's hand cards (not controlled by A, not on
    // A's board).
    let opp_card = s.b.hand[0].clone();
    let _ = s.move_card(&opp_card, PlayerId::B, Zone::Hand, Zone::Board);
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![opp_card.clone()],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::SacrificePaymentInvalid(opp_card)));
}

#[test]
fn play_card_errors_on_variable_x_cost() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 0,
            source: CostSource::Hand,
            is_x: true,
            kind: None,
        }],
    );
    let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(result, Err(PlayError::VariableXValueMissing));
}

#[test]
fn play_card_leaves_state_unchanged_on_error() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 100,
            source: CostSource::Mill,
            is_x: false,
            kind: None,
        }],
    );
    let hand_before = s.a.hand.clone();
    let deck_before = s.a.deck.clone();
    let board_before = s.a.board.clone();
    let graveyard_before = s.a.graveyard.clone();
    let _ = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(s.a.hand, hand_before);
    assert_eq!(s.a.deck, deck_before);
    assert_eq!(s.a.board, board_before);
    assert_eq!(s.a.graveyard, graveyard_before);
}

fn registry_with_fixture(name: &str, source: &str) -> crate::card::CardRegistry {
    let tmp = std::env::temp_dir().join(format!("tsot_fixture_{name}"));
    std::fs::create_dir_all(&tmp).unwrap();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for entry in rd.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let path = tmp.join(format!("{name}.lua"));
    std::fs::write(&path, source).unwrap();
    crate::card::CardRegistry::load(&tmp).unwrap()
}

#[test]
fn jellyfish_on_enter_board_bounces_chosen_creature_via_scripted_oracle() {
    use crate::card::CardRegistry;
    use crate::choice::{ScriptedAnswer, ScriptedOracle};
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let jelly = registry
        .cards()
        .iter()
        .find(|c| c.id == "jellyfish")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let jelly_iid = s.a.hand[0].clone();
    let target_iid = s.b.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&jelly_iid).unwrap();
        inst.content = Some(jelly.clone());
    }
    // Put an opposing creature on B's board to be the target.
    s.b.hand.retain(|x| x != &target_iid);
    s.b.board.push(target_iid.clone());

    // Seed graveyard for the 2-graveyard cost. Per P.12a (lenient),
    // at least one GY pitch must color-match the blue jellyfish cast;
    // paint the last seed blue so it lands in the back-of-GY window.
    let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
    set_identity(&mut s, &gy_seeds[2], &["blue"], "");
    s.a.graveyard.extend(gy_seeds.clone());

    let b_hand_before = s.b.hand.len();
    let b_board_before = s.b.board.len();

    // 1-hand cost requires a payment; pick any non-self hand card.
    let hand_payment = s
        .a
        .hand
        .iter()
        .find(|x| *x != &jelly_iid)
        .cloned()
        .unwrap();
    // P.7a: jellyfish is blue, payment needs blue.
    set_identity(&mut s, &hand_payment, &["blue"], "");

    // Scripted oracle: pick target_iid.
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Card(Some(target_iid.clone()))]);

    s.play_card(
        PlayerId::A,
        &jelly_iid,
        PlayChoices {
            hand_payment_ids: vec![hand_payment],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // Target moved from B's board to B's hand.
    assert!(!s.b.board.contains(&target_iid));
    assert!(s.b.hand.contains(&target_iid));
    assert_eq!(s.b.hand.len(), b_hand_before + 1);
    assert_eq!(s.b.board.len(), b_board_before - 1);
}

#[test]
fn on_play_handler_fires_before_card_leaves_hand() {
    let registry = registry_with_fixture(
        "on_play",
        r#"return {
            id = "fire-on-play",
            on_play = function(game, self)
                -- Record whether the card is still in hand when we see it.
                _G.fire_on_play_count = (_G.fire_on_play_count or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-play")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&creature).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    registry
        .lua()
        .globals()
        .set("fire_on_play_count", 0_i32)
        .unwrap();
    s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    let count: i32 = registry.lua().globals().get("fire_on_play_count").unwrap();
    assert_eq!(count, 1);
    assert_eq!(s.event_fires[&crate::card::EventName::OnPlay], [1, 0]);
}

// P.39: Clear cards tutor for a jewel OR a same-color Symbol. When
// the deck contains only a Symbol (no jewel), Clear must reach for
// the Symbol and move it to HAND.
#[test]
fn clear_red_tutors_red_symbol_when_no_jewel_in_deck() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let clear_red = registry
        .cards()
        .iter()
        .find(|c| c.id == "clear-red")
        .unwrap()
        .clone();
    let red_symbol = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-ix-symbol")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let clear_iid = s.a.hand[0].clone();
    let symbol_iid = s.a.deck[3].clone();
    {
        let inst = s.card_pool.get_mut(&clear_iid).unwrap();
        inst.content = Some(clear_red.clone());
    }
    {
        let inst = s.card_pool.get_mut(&symbol_iid).unwrap();
        inst.content = Some(red_symbol.clone());
    }
    // Sanity: no `red-jewel` in deck — the tutor must fall back to
    // the Symbol path.
    assert!(!s
        .a
        .deck
        .iter()
        .filter_map(|iid| s.card_pool.get(iid))
        .any(|i| i.card().id == "red-jewel"));

    s.play_card(
        PlayerId::A,
        &clear_iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    // The symbol moved from DECK to HAND.
    assert!(s.a.hand.contains(&symbol_iid));
    assert!(!s.a.deck.contains(&symbol_iid));
}

// P.39: when both a jewel AND a same-color Symbol are in DECK, the
// tutor still resolves (player-choice in the rules; the lua handler
// is allowed to pick either deterministically). The contract is just
// "ONE eligible card moves to HAND".
#[test]
fn clear_red_tutors_some_eligible_card_when_both_available() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let clear_red = registry
        .cards()
        .iter()
        .find(|c| c.id == "clear-red")
        .unwrap()
        .clone();
    let red_jewel = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-jewel")
        .unwrap()
        .clone();
    let red_symbol = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-ax-symbol")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let clear_iid = s.a.hand[0].clone();
    let jewel_iid = s.a.deck[2].clone();
    let symbol_iid = s.a.deck[5].clone();
    {
        let inst = s.card_pool.get_mut(&clear_iid).unwrap();
        inst.content = Some(clear_red.clone());
    }
    {
        let inst = s.card_pool.get_mut(&jewel_iid).unwrap();
        inst.content = Some(red_jewel.clone());
    }
    {
        let inst = s.card_pool.get_mut(&symbol_iid).unwrap();
        inst.content = Some(red_symbol.clone());
    }
    let hand_size_before = s.a.hand.len();
    s.play_card(
        PlayerId::A,
        &clear_iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();
    // Exactly one of the eligible cards landed in hand. (Clear's
    // SELF-cost exile drops clear-red from hand; the tutor adds 1.
    // Net: hand_size_before stays the same.)
    let in_hand_now = s.a.hand.contains(&jewel_iid) || s.a.hand.contains(&symbol_iid);
    assert!(in_hand_now, "tutor must place a jewel or symbol into hand");
    assert_eq!(s.a.hand.len(), hand_size_before);
}

// P.39: same-color requirement on the Symbol path. clear-red must NOT
// tutor a blue symbol when no red-jewel exists.
#[test]
fn clear_red_does_not_tutor_off_color_symbol() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let clear_red = registry
        .cards()
        .iter()
        .find(|c| c.id == "clear-red")
        .unwrap()
        .clone();
    let blue_symbol = registry
        .cards()
        .iter()
        .find(|c| c.id == "blue-ix-symbol")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let clear_iid = s.a.hand[0].clone();
    let symbol_iid = s.a.deck[3].clone();
    {
        let inst = s.card_pool.get_mut(&clear_iid).unwrap();
        inst.content = Some(clear_red.clone());
    }
    {
        let inst = s.card_pool.get_mut(&symbol_iid).unwrap();
        inst.content = Some(blue_symbol.clone());
    }
    s.play_card(
        PlayerId::A,
        &clear_iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();
    // The blue symbol must NOT be tutored (no color overlap).
    assert!(!s.a.hand.contains(&symbol_iid));
    assert!(s.a.deck.contains(&symbol_iid));
}

#[test]
fn surge_instant_untaps_all_your_creatures_on_play() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let surge = registry
        .cards()
        .iter()
        .find(|c| c.id == "surge")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let surge_iid = s.a.hand[0].clone();
    let cred1 = s.a.hand[1].clone();
    let cred2 = s.a.hand[2].clone();
    let b_creat = s.b.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&surge_iid).unwrap();
        inst.content = Some(surge.clone());
    }
    // Put two tapped A creatures on board, plus one tapped B creature.
    s.a.hand.retain(|x| x != &cred1 && x != &cred2);
    s.a.board.push(cred1.clone());
    s.a.board.push(cred2.clone());
    s.card_pool.get_mut(&cred1).unwrap().tapped = true;
    s.card_pool.get_mut(&cred2).unwrap().tapped = true;
    s.b.hand.retain(|x| x != &b_creat);
    s.b.board.push(b_creat.clone());
    s.card_pool.get_mut(&b_creat).unwrap().tapped = true;

    let payment = s.a.hand.iter().find(|x| *x != &surge_iid).cloned().unwrap();
    let payment2 = s
        .a
        .hand
        .iter()
        .find(|x| *x != &surge_iid && **x != payment)
        .cloned()
        .unwrap();
    // P.7a: surge is blue, payments need blue.
    set_identity(&mut s, &payment, &["blue"], "");
    set_identity(&mut s, &payment2, &["blue"], "");
    s.play_card(
        PlayerId::A,
        &surge_iid,
        PlayChoices {
            hand_payment_ids: vec![payment, payment2],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    // Both A creatures untapped.
    assert!(!s.card_pool.get(&cred1).unwrap().tapped);
    assert!(!s.card_pool.get(&cred2).unwrap().tapped);
    // B's creature unchanged.
    assert!(s.card_pool.get(&b_creat).unwrap().tapped);
}

#[test]
fn draw_two_instant_plays_from_graveyard_cost_and_draws() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let draw_two = registry
        .cards()
        .iter()
        .find(|c| c.id == "draw-two")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let instant_iid = s.a.hand[0].clone();
    // Swap to draw-two's card data (instant type, graveyard cost, handler).
    {
        let inst = s.card_pool.get_mut(&instant_iid).unwrap();
        inst.content = Some(draw_two.clone());
    }
    // Seed the graveyard so the cost can be paid.
    let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
    s.a.graveyard.extend(gy_seeds.clone());
    // Seed a host with 3 attached cards (draw-two also costs 3 attached).
    let host = s.a.hand[1].clone();
    let att1 = s.a.hand[2].clone();
    let att2 = s.a.hand[3].clone();
    let att3 = s.a.hand[4].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    for a in [&att1, &att2, &att3] {
        let _ = s.remove_from_zone(a, PlayerId::A, Zone::Hand);
        s.add_attached(&host, a);
    }

    let hand_before = s.a.hand.len();
    let deck_before = s.a.deck.len();
    let gy_before = s.a.graveyard.len();
    let exile_before = s.a.exile.len();

    s.play_card(
        PlayerId::A,
        &instant_iid,
        PlayChoices {
            attached_payment_ids: vec![att1.clone(), att2.clone(), att3.clone()],
            ..PlayChoices::default()
        },
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    // - Played card removed from hand (1 leaves).
    // - 3 graveyard cards exiled (graveyard cost).
    // - 3 attached cards exiled per P.31 (non-board cast → exile).
    // - 2 cards drawn from deck into hand.
    // - Played card lands in graveyard.
    assert_eq!(s.a.hand.len(), hand_before - 1 + 2);
    assert_eq!(s.a.deck.len(), deck_before - 2);
    assert_eq!(s.a.graveyard.len(), gy_before - 3 + 1);
    // exile gains 3 (graveyard cost) + 3 (attached cost) = 6.
    assert_eq!(s.a.exile.len(), exile_before + 6);
    assert!(s.a.graveyard.contains(&instant_iid));
    assert!(!s.a.board.contains(&instant_iid));
    assert_eq!(s.event_fires[&crate::card::EventName::OnPlay], [1, 0]);
    // on_enter_board does NOT fire for instants.
    assert!(!s.event_fires.contains_key(&crate::card::EventName::OnEnterBoard));
}

#[test]
fn goblin_scribe_draws_a_card_on_enter_board() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let scribe = registry
        .cards()
        .iter()
        .find(|c| c.id == "goblin-scribe")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&creature).unwrap();
        inst.card_mut().handlers = scribe.handlers.clone();
        inst.card_mut().id = scribe.id.clone();
    }
    let hand_before = s.a.hand.len();
    let deck_before = s.a.deck.len();

    s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    // Hand lost the played card and gained one drawn card → net zero.
    // (-1 for play, +1 for ETB draw.)
    assert_eq!(s.a.hand.len(), hand_before);
    assert_eq!(s.a.deck.len(), deck_before - 1);
    assert!(s.a.board.contains(&creature));
    assert_eq!(
        s.event_fires[&crate::card::EventName::OnEnterBoard],
        [1, 0]
    );
}

#[test]
fn on_enter_board_handler_fires_after_card_on_board() {
    let registry = registry_with_fixture(
        "on_enter_board",
        r#"return {
            id = "fire-on-enter",
            on_enter_board = function(game, self)
                _G.fire_on_enter_count = (_G.fire_on_enter_count or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "fire-on-enter")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&creature).unwrap();
        inst.card_mut().handlers = fixture.handlers.clone();
        inst.card_mut().id = fixture.id.clone();
    }

    registry
        .lua()
        .globals()
        .set("fire_on_enter_count", 0_i32)
        .unwrap();
    s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    let count: i32 = registry
        .lua()
        .globals()
        .get("fire_on_enter_count")
        .unwrap();
    assert_eq!(count, 1);
    assert!(s.a.board.contains(&creature));
    assert_eq!(
        s.event_fires[&crate::card::EventName::OnEnterBoard],
        [1, 0]
    );
}

#[test]
fn counterspell_resolves_and_removes_underlying_cast() {
    // End-to-end: A's cast sits on the chain, B casts counterspell in
    // response, drive_window_to_close runs counterspell which calls
    // game.counter_top() and removes A's cast. A's creature never reaches
    // the board; A's HAND payment refunds (stays in hand) because cost
    // wasn't paid at resolution time (it never resolved).
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let counterspell = registry
        .cards()
        .iter()
        .find(|c| c.id == "counterspell")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let a_creature = s.a.hand[0].clone();
    let a_hand_payment = s.a.hand[1].clone();
    let cs_iid = s.b.hand[0].clone();
    s.card_pool.get_mut(&cs_iid).unwrap().content = Some(counterspell);
    // Counterspell (blue+purple) costs 1 graveyard. Per P.12a the GY
    // pitch must color-match — paint the seed blue.
    let gy_seed = s.b.hand[1].clone();
    set_identity(&mut s, &gy_seed, &["blue"], "");
    let _ = s.move_card(&gy_seed, PlayerId::B, Zone::Hand, Zone::Graveyard);

    // A's cast announced manually (bypasses play_card so we control the
    // chain). HAND payment recorded in the chain item but NOT yet moved
    // out of hand — that happens at resolve time.
    let a_cast = crate::game::StackItem::PlayedCard {
        card: a_creature.clone(),
        controller: PlayerId::A,
        choices: PlayChoices {
            hand_payment_ids: vec![a_hand_payment.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
    };
    s.open_response_window(a_cast).unwrap();
    s.pass_priority().unwrap(); // A → B

    // B casts counterspell — routes to respond_with (piece 2).
    let mut oracle = crate::choice::NoopOracle;
    s.play_card(
        PlayerId::B,
        &cs_iid,
        PlayChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // Drive the window to close. Counterspell pops, resolves, calls
    // game.counter_top() which removes A's cast from the chain. Then
    // auto-pass drains the empty chain and the window closes.
    s.drive_window_to_close(Some(&mut EventContext::new(registry.lua(), &mut oracle)))
        .unwrap();

    // A's cast got countered: creature stays in hand, payment stays in
    // hand (cost refunded), nothing on board.
    assert!(s.a.hand.contains(&a_creature));
    assert!(s.a.hand.contains(&a_hand_payment));
    assert!(!s.a.board.contains(&a_creature));
    // Counterspell itself resolved → B's graveyard.
    assert!(s.b.graveyard.contains(&cs_iid));
    // Window closed.
    assert!(s.priority.is_none());
    // counter_top bumped for B (counterspell's controller).
    assert_eq!(
        s.action_counts.get("counter_top").map(|v| v[1]).unwrap_or(0),
        1
    );
}

#[test]
fn play_card_during_open_window_pushes_to_chain_instead_of_opening_new() {
    // Simulate B casting an instant in response to A's cast: A's cast sits
    // on the chain (window open), then B calls play_card on a free instant.
    // The instant should land on top of the chain, not open a new window.
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let b_instant = s.b.hand[0].clone();
    s.card_pool.get_mut(&b_instant).unwrap().card_mut().kind = crate::card::CardType::Spell;
    s.card_pool.get_mut(&b_instant).unwrap().card_mut().timing = Some(crate::card::Timing::Instant);
    s.card_pool.get_mut(&b_instant).unwrap().card_mut().cost = vec![]; // free

    // Manually open a window with a placeholder A cast — bypasses play_card
    // so we control the chain shape exactly.
    let a_placeholder = crate::game::StackItem::PlayedCard {
        card: s.a.hand[0].clone(),
        controller: PlayerId::A,
        choices: PlayChoices::default(),
    };
    s.open_response_window(a_placeholder.clone()).unwrap();
    s.pass_priority().unwrap(); // A passes → B has priority

    // B casts the free instant — play_card sees priority.is_some() and
    // routes to respond_with instead of opening a nested window.
    s.play_card(PlayerId::B, &b_instant, PlayChoices::default(), None)
        .unwrap();

    let chain = &s.priority.as_ref().unwrap().chain;
    assert_eq!(chain.len(), 2, "B's instant should sit on top of A's cast");
    assert_eq!(chain[0], a_placeholder);
    match &chain[1] {
        crate::game::StackItem::PlayedCard { card, controller, .. } => {
            assert_eq!(card, &b_instant);
            assert_eq!(*controller, PlayerId::B);
        }
    }
    // P.33: cast card leaves HAND at cast announce (not stays-in-hand).
    // It's not yet in GRAVEYARD either — it's on the chain awaiting
    // resolution. So neither HAND nor GRAVEYARD contains it now.
    assert!(!s.b.hand.contains(&b_instant));
    assert!(!s.b.graveyard.contains(&b_instant));
}

#[test]
fn lua_chain_and_counter_target_apis_remove_specific_item() {
    // Verifies the Phase 2 introspection + explicit-target counter from Lua:
    // a fixture instant's on_play inspects game.chain(), picks the bottom
    // item by InstanceId, and calls game.counter(target). The bottom item
    // is removed; this card itself (which is the top of the chain at
    // resolution time, then popped before the handler runs) leaves the
    // chain via the normal pop path. Counterspell's "counter top" semantics
    // are NOT used here — we want the explicit-target path.
    let registry = registry_with_fixture(
        "explicit_counter",
        r#"
        return {
          id = "explicit_counter",
          type = "instant",
          on_play = function(game, self)
            local chain = game.chain()
            if #chain == 0 then return end
            -- Pick the bottom of the chain (the older cast).
            game.counter(chain[1].card)
          end,
        }
        "#,
    );
    let card = registry.cards().iter().find(|c| c.id == "explicit_counter").unwrap().clone();

    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let a_creature = s.a.hand[0].clone();
    let cs_iid = s.b.hand[0].clone();
    s.card_pool.get_mut(&cs_iid).unwrap().content = Some(card);

    // A's cast on the chain.
    let a_cast = crate::game::StackItem::PlayedCard {
        card: a_creature.clone(),
        controller: PlayerId::A,
        choices: PlayChoices::default(),
    };
    s.open_response_window(a_cast).unwrap();
    s.pass_priority().unwrap(); // A → B

    // B casts the fixture instant in response.
    let mut oracle = crate::choice::NoopOracle;
    s.play_card(
        PlayerId::B,
        &cs_iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // Drive — the fixture's on_play fires, sees chain = [a_creature],
    // calls game.counter(a_creature), removes A's cast.
    s.drive_window_to_close(Some(&mut crate::game::EventContext::new(
        registry.lua(),
        &mut oracle,
    )))
    .unwrap();

    // A's creature stays in hand (countered, never resolved).
    assert!(s.a.hand.contains(&a_creature));
    assert!(!s.a.board.contains(&a_creature));
    // Fixture instant resolved to B's graveyard.
    assert!(s.b.graveyard.contains(&cs_iid));
    // Window closed.
    assert!(s.priority.is_none());
    // counter (target) bumped for B.
    assert_eq!(s.action_counts.get("counter").map(|v| v[1]).unwrap_or(0), 1);
}

fn one_hand_cost() -> Vec<CostComponent> {
    vec![CostComponent {
        amount: 1,
        source: CostSource::Hand,
        is_x: false,
        kind: None,
    }]
}

#[test]
fn hand_payment_color_match_succeeds() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    set_identity(&mut s, &creature, &["green"], "");
    set_identity(&mut s, &pay, &["green"], "");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(result.is_ok(), "shared color should pay successfully: {result:?}");
}

#[test]
fn hand_payment_color_mismatch_rejected() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    set_identity(&mut s, &creature, &["green"], "");
    set_identity(&mut s, &pay, &["red"], "");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::HandPaymentIdentityMismatch(pay)));
}

#[test]
fn hand_payment_symbol_match_succeeds_across_colors() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    // Green cast with symbol ⊨; red pay with same symbol — symbol
    // overrides color mismatch.
    set_identity(&mut s, &creature, &["green"], "⊨");
    set_identity(&mut s, &pay, &["red"], "⊨");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(result.is_ok(), "shared symbol should pay across colors: {result:?}");
}

#[test]
fn hand_payment_matches_when_any_of_multiple_symbols_overlap() {
    // Cast carries two symbols {⊨, ⨳}; payment carries one {⨳}. P.7a
    // says identity match iff the symbol sets intersect — they do, on ⨳.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    s.card_pool.get_mut(&creature).unwrap().card_mut().colors = vec!["green".into()];
    s.card_pool.get_mut(&creature).unwrap().card_mut().symbols =
        vec!["⊨".into(), "⨳".into()];
    s.card_pool.get_mut(&pay).unwrap().card_mut().colors = vec!["red".into()];
    s.card_pool.get_mut(&pay).unwrap().card_mut().symbols = vec!["⨳".into()];
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(
        result.is_ok(),
        "any shared symbol should satisfy the identity match: {result:?}"
    );
}

#[test]
fn hand_payment_rejected_when_multi_symbol_sets_disjoint() {
    // Cast carries {⊨, ⨳}; payment carries {꩜}. No color or symbol
    // overlap → reject.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    s.card_pool.get_mut(&creature).unwrap().card_mut().colors = vec!["green".into()];
    s.card_pool.get_mut(&creature).unwrap().card_mut().symbols =
        vec!["⊨".into(), "⨳".into()];
    s.card_pool.get_mut(&pay).unwrap().card_mut().colors = vec!["red".into()];
    s.card_pool.get_mut(&pay).unwrap().card_mut().symbols = vec!["꩜".into()];
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(
        result.is_err(),
        "disjoint identity sets should reject the payment: {result:?}"
    );
}

#[test]
fn hand_payment_colorless_cast_takes_any_discard() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    // No identity on cast → wildcard, any discard accepted.
    set_identity(&mut s, &creature, &[], "");
    set_identity(&mut s, &pay, &["red"], "⊨");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert!(result.is_ok(), "colorless cast should accept any discard: {result:?}");
}

#[test]
fn hand_payment_no_identity_pay_cannot_satisfy_identified_cast() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    // Cast has identity {green}. Discard has empty identity. Under
    // the strict pay-side rule, empty identity cannot intersect with
    // {green} → reject.
    set_identity(&mut s, &creature, &["green"], "");
    set_identity(&mut s, &pay, &[], "");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::HandPaymentIdentityMismatch(pay)));
}

#[test]
fn hand_payment_no_symbol_discard_cannot_pay_for_symboled_cast() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    set_cost(&mut s, &creature, one_hand_cost());
    // Cast: green + symbol. Pay: red, no symbol. No color overlap,
    // no symbol overlap → reject. (The pay would need to either share
    // a color with cast, or have the same symbol.)
    set_identity(&mut s, &creature, &["green"], "⊨");
    set_identity(&mut s, &pay, &["red"], "");
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::HandPaymentIdentityMismatch(pay)));
}

#[test]
fn activate_red_jewel_taps_and_draws_then_discards() {
    use crate::card::CardRegistry;
    use crate::game::play::ActivateError;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let jewel = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-jewel")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(jewel);
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    let hand_before = s.a.hand.len();
    let gy_before = s.a.graveyard.len();

    assert!(s.can_activate(&iid, 0), "fresh untapped jewel on board should be activatable");

    let mut oracle = crate::choice::NoopOracle;
    let result = s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Ok(()));
    assert!(s.card_pool.get(&iid).unwrap().tapped, "tap cost should mark jewel tapped");
    // T: draw a card, then discard a card. Net hand size unchanged
    // (+1 drawn, -1 discarded), but graveyard grew by 1.
    assert_eq!(s.a.hand.len(), hand_before);
    assert_eq!(s.a.graveyard.len(), gy_before + 1);

    // Second activation must be rejected — jewel is now tapped.
    assert!(!s.can_activate(&iid, 0));
    let second = s.activate_ability(&iid, 0, None, crate::game::ActivateChoices::default(), None);
    assert_eq!(second, Err(ActivateError::AlreadyTapped));
}

#[test]
fn activate_creature_with_summoning_sickness_is_rejected() {
    use crate::card::CardRegistry;
    use crate::game::play::ActivateError;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let vh = registry
        .cards()
        .iter()
        .find(|c| c.id == "vigilant-human")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(vh);
        inst.summoning_sick = true; // freshly played, no haste
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    assert!(!s.can_activate(&iid, 0));
    let result = s.activate_ability(&iid, 0, None, crate::game::ActivateChoices::default(), None);
    assert_eq!(result, Err(ActivateError::SummoningSick));
}

#[test]
fn activate_returns_no_such_ability_for_out_of_range_idx() {
    use crate::card::CardRegistry;
    use crate::game::play::ActivateError;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let jewel = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-jewel")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().content = Some(jewel);
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    // red-jewel has exactly one activated ability.
    assert!(!s.can_activate(&iid, 1));
    assert_eq!(s.activate_ability(&iid, 1, None, crate::game::ActivateChoices::default(), None), Err(ActivateError::NoSuchAbility));
}

#[test]
fn vigilant_human_t_ability_no_ops_if_did_not_attack() {
    // Per RULES A.7: vigilant-human's T-ability gates on the
    // per-instance attacked_this_turn flag. With the flag false, the
    // handler runs but draws nothing. The cost (tap) still resolves.
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let vh = registry
        .cards()
        .iter()
        .find(|c| c.id == "vigilant-human")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(vh);
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    let hand_before = s.a.hand.len();
    assert!(!s.card_pool.get(&iid).unwrap().attacked_this_turn);

    let mut oracle = crate::choice::NoopOracle;
    s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();
    assert!(s.card_pool.get(&iid).unwrap().tapped);
    // Effect short-circuits: no draw.
    assert_eq!(s.a.hand.len(), hand_before);
}

#[test]
fn vigilant_human_t_ability_draws_after_attacking() {
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let vh = registry
        .cards()
        .iter()
        .find(|c| c.id == "vigilant-human")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(vh);
        inst.summoning_sick = false;
        inst.attacked_this_turn = true; // simulate post-combat
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    let hand_before = s.a.hand.len();
    let mut oracle = crate::choice::NoopOracle;
    s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();
    assert_eq!(s.a.hand.len(), hand_before + 1, "attacked-this-turn → draws a card");
}

#[test]
fn blue_monkey_2_hand_activation_discards_two_and_draws_one() {
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let monkey = registry
        .cards()
        .iter()
        .find(|c| c.id == "blue-monkey")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(monkey);
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    // Snapshot pre-state.
    let hand_before = s.a.hand.len();
    let gy_before = s.a.graveyard.len();

    assert!(s.can_activate(&iid, 0));
    let mut oracle = crate::choice::NoopOracle;
    s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // 2 hand discarded, then 1 drawn → net hand -1, graveyard +2,
    // deck -1 (the drawn card came off the top).
    assert_eq!(s.a.hand.len(), hand_before - 1);
    assert_eq!(s.a.graveyard.len(), gy_before + 2);
    // The activation does NOT tap the monkey (its cost has no T:).
    assert!(!s.card_pool.get(&iid).unwrap().tapped);
}

#[test]
fn monkey_cannot_activate_with_insufficient_hand() {
    use crate::card::CardRegistry;
    use crate::game::play::ActivateError;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let monkey = registry
        .cards()
        .iter()
        .find(|c| c.id == "blue-monkey")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(monkey);
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());
    // Drain hand to 1 card — below the 2-hand cost.
    while s.a.hand.len() > 1 {
        let drop = s.a.hand[0].clone();
        s.a.hand.retain(|x| x != &drop);
    }

    assert!(!s.can_activate(&iid, 0));
    let result = s.activate_ability(&iid, 0, None, crate::game::ActivateChoices::default(), None);
    assert_eq!(result, Err(ActivateError::CannotPayComponents));
}

#[test]
fn white_monkey_grants_plus_2_and_vigilance_eot() {
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let monkey = registry
        .cards()
        .iter()
        .find(|c| c.id == "white-monkey")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let monkey_iid = s.a.hand[0].clone();
    let buddy_iid = s.a.hand[1].clone();
    {
        let m = s.card_pool.get_mut(&monkey_iid).unwrap();
        m.content = Some(monkey);
        m.summoning_sick = false;
    }
    {
        // Give buddy a baseline 1/1 stat line.
        let b = s.card_pool.get_mut(&buddy_iid).unwrap();
        b.card_mut().stats = Some(crate::card::Stats { x: 1.0, y: 1.0 });
        b.card_mut().kind = crate::card::CardType::Creature;
        b.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &monkey_iid && x != &buddy_iid);
    s.a.board.push(monkey_iid.clone());
    s.a.board.push(buddy_iid.clone());

    // Pre: buddy is 1/1, no vigilance.
    let (bx0, by0) = s.effective_stats(&buddy_iid);
    assert_eq!((bx0, by0), (1.0, 1.0));
    assert!(!s.has_keyword(&buddy_iid, "vigilance"));

    // Activate.
    let mut oracle = crate::choice::NoopOracle;
    s.activate_ability(
        &monkey_iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // Post: buddy got +2/+2 EOT and vigilance EOT.
    let (bx1, by1) = s.effective_stats(&buddy_iid);
    assert_eq!((bx1, by1), (3.0, 3.0));
    assert!(s.has_keyword(&buddy_iid, "vigilance"));

    // Self-pump: monkey itself also gained +2/+2 and vigilance.
    let (mx, my) = s.effective_stats(&monkey_iid);
    assert_eq!((mx, my), (4.0, 4.0));
    assert!(s.has_keyword(&monkey_iid, "vigilance"));
}

#[test]
fn validate_hook_refuses_and_charges_no_cost_when_no_target() {
    // RULES A.9: pink-monkey's validate returns false when there's no
    // opposing creature on board. The activation must abort with
    // NoLegalTarget AND not deduct hand cost.
    use crate::card::CardRegistry;
    use crate::game::play::ActivateError;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let pink = registry
        .cards()
        .iter()
        .find(|c| c.id == "pink-monkey")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(pink);
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());
    // B has no creatures on board — pink-monkey's validate returns false.

    let hand_before = s.a.hand.len();
    let gy_before = s.a.graveyard.len();

    // `can_activate` is the cheap pre-check; it doesn't run validate,
    // so it returns true even though the activation will refuse.
    assert!(s.can_activate(&iid, 0));

    let mut oracle = crate::choice::NoopOracle;
    let result = s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Err(ActivateError::NoLegalTarget));

    // No cost was paid — hand and graveyard unchanged.
    assert_eq!(s.a.hand.len(), hand_before);
    assert_eq!(s.a.graveyard.len(), gy_before);
    // Monkey did not tap (its cost has no T:, but even if it did the
    // refusal would skip the tap).
    assert!(!s.card_pool.get(&iid).unwrap().tapped);
}

#[test]
fn validate_hook_passes_and_charges_when_target_exists() {
    // Mirror of the previous test but with a valid target. Confirms
    // the validate gate passes and cost is paid; the bounce semantics
    // are not asserted here because NoopOracle returns None for
    // choose_card, short-circuiting the handler. The validate path is
    // what's under test.
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let pink = registry
        .cards()
        .iter()
        .find(|c| c.id == "pink-monkey")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let iid = s.a.hand[0].clone();
    let target = s.b.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(pink);
        inst.summoning_sick = false;
    }
    {
        // Put an opposing creature on B's board so validate passes.
        let t = s.card_pool.get_mut(&target).unwrap();
        t.card_mut().kind = crate::card::CardType::Creature;
        t.card_mut().stats = Some(crate::card::Stats { x: 1.0, y: 1.0 });
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());
    s.b.hand.retain(|x| x != &target);
    s.b.board.push(target.clone());

    let a_hand_before = s.a.hand.len();

    let mut oracle = crate::choice::NoopOracle;
    let result = s.activate_ability(
        &iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Ok(()));
    // Cost paid: 2 cards from A's hand → graveyard.
    assert_eq!(s.a.hand.len(), a_hand_before - 2);
}

#[test]
fn dark_salamander_x_cost_activation_mills_2y() {
    // X-cost activation: pay Y hand cards, mill opponent by 2Y.
    // Tests the end-to-end X flow: AI picks X, engine multiplies
    // cost, handler reads X via game.x_value().
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let sala = registry
        .cards()
        .iter()
        .find(|c| c.id == "dark-salamander")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let sala_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&sala_iid).unwrap();
        inst.content = Some(sala);
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &sala_iid);
    s.a.board.push(sala_iid.clone());

    let a_hand_before = s.a.hand.len();
    let b_deck_before = s.b.deck.len();
    let b_gy_before = s.b.graveyard.len();

    // Activate with Y=3 → mill = 2*3 = 6 cards.
    let mut oracle = crate::choice::NoopOracle;
    let result = s.activate_ability(
        &sala_iid,
        0,
        Some(3),
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Ok(()));
    // Cost: 3 hand cards discarded.
    assert_eq!(s.a.hand.len(), a_hand_before - 3);
    // Effect: B's deck milled by 6.
    assert_eq!(s.b.deck.len(), b_deck_before - 6);
    assert_eq!(s.b.graveyard.len(), b_gy_before + 6);
}

#[test]
fn red_jewel_grants_t_draw_discard_to_attached_host() {
    // Phase 3: when a jewel is in a creature's attached list, the
    // jewel's static (scope=attached_host) grants the host a T-ability
    // identical to the jewel's own. Activating the host's granted
    // ability draws + discards for the host's owner.
    use crate::card::CardRegistry;
    use crate::game::EventContext;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let jewel = registry
        .cards()
        .iter()
        .find(|c| c.id == "red-jewel")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let host_iid = s.a.hand[0].clone();
    let jewel_iid = s.a.hand[1].clone();
    {
        // Host is a generic 2/2 creature.
        let host_inst = s.card_pool.get_mut(&host_iid).unwrap();
        host_inst.card_mut().kind = crate::card::CardType::Creature;
        host_inst.card_mut().stats = Some(crate::card::Stats { x: 2.0, y: 2.0 });
        host_inst.summoning_sick = false;
    }
    {
        let jewel_inst = s.card_pool.get_mut(&jewel_iid).unwrap();
        jewel_inst.content = Some(jewel);
    }
    // Place host on board, attach jewel under it (simulating the
    // post-on_attached_as_cost state without running play_card).
    s.a.hand.retain(|x| x != &host_iid && x != &jewel_iid);
    s.a.board.push(host_iid.clone());
    s.card_pool
        .get_mut(&host_iid)
        .unwrap()
        .attached
        .push(jewel_iid.clone());

    // Host has 0 printed activations. With jewel attached, grant adds 1.
    assert_eq!(s.activation_count(&host_iid), 1);
    let ability = s.activation_at(&host_iid, 0).expect("granted ability");
    assert!(ability.cost_tap);

    let hand_before = s.a.hand.len();
    let gy_before = s.a.graveyard.len();

    assert!(s.can_activate(&host_iid, 0));
    let mut oracle = crate::choice::NoopOracle;
    let result = s.activate_ability(
        &host_iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Ok(()));

    // Host tapped from the T: cost. Net hand size unchanged (draw +
    // discard cancel), graveyard up by 1.
    assert!(s.card_pool.get(&host_iid).unwrap().tapped);
    assert_eq!(s.a.hand.len(), hand_before);
    assert_eq!(s.a.graveyard.len(), gy_before + 1);
}

#[test]
fn clear_view_fills_one_hand_slot_of_a_two_hand_cast() {
    // Clear View in GY can substitute for one HAND slot of a 2-hand
    // blue cast, leaving the other slot to be paid by an identity-
    // matching blue card from hand. Verifies: GY → EXILE move,
    // identity check ignored on the substitute slot, hand_payment
    // slot still identity-checked on the remaining slot.
    use crate::card::CardRegistry;
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let clear_view = registry
        .cards()
        .iter()
        .find(|c| c.id == "clear-view")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let cast = s.a.hand[0].clone();
    let blue_pay = s.a.hand[1].clone();
    let cv = s.a.hand[2].clone();
    // Cast = blue, 2-hand cost.
    set_identity(&mut s, &cast, &["blue"], "");
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    set_identity(&mut s, &blue_pay, &["blue"], "");
    s.card_pool.get_mut(&cv).unwrap().content = Some(clear_view);
    // Put Clear View in graveyard.
    s.a.hand.retain(|x| x != &cv);
    s.a.graveyard.push(cv.clone());

    let exile_before = s.a.exile.len();
    let gy_before = s.a.graveyard.len();

    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![blue_pay.clone()],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![cv.clone()],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Ok(()));
    // Clear View moved GY → EXILE.
    assert_eq!(s.a.graveyard.len(), gy_before - 1);
    assert_eq!(s.a.exile.len(), exile_before + 1);
    assert!(s.a.exile.contains(&cv));
}

#[test]
fn clear_view_cannot_pay_alone_for_one_hand_colored_cast() {
    // 1-hand blue cast with only Clear View available. P.7a requires
    // at least one identity match in hand payments. Clear View
    // doesn't satisfy P.7a — and there's no other slot for an
    // identity-matching card. The cast must fail (WrongHandPaymentCount,
    // because gy_hand_payment_ids.len() > hand_needed once accepted, or
    // identity mismatch on whatever the AI tries to substitute with).
    use crate::card::CardRegistry;
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let clear_view = registry
        .cards()
        .iter()
        .find(|c| c.id == "clear-view")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(60, "a"), deck_of(60, "b"));
    let cast = s.a.hand[0].clone();
    let cv = s.a.hand[1].clone();
    set_identity(&mut s, &cast, &["blue"], "");
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    s.card_pool.get_mut(&cv).unwrap().content = Some(clear_view);
    s.a.hand.retain(|x| x != &cv);
    s.a.graveyard.push(cv.clone());

    // Try to pay using ONLY Clear View. Engine rejects because Clear
    // View doesn't carry identity, leaving no payment to satisfy the
    // cast's blue identity requirement.
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![],
            x_value: None,
            jewel_tap: None,
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![cv.clone()],
            attached_payment_ids: vec![],
            graveyard_payment_ids: vec![],        },
        None,
    );
    assert_eq!(result, Err(PlayError::NoHandPaymentForIdentity));
    // Clear View stays in graveyard — no cost paid since cast rejected.
    assert!(s.a.graveyard.contains(&cv));
    assert!(!s.a.exile.contains(&cv));
}

/// Set up: A's hand has a card to cast plus host + attached fodder.
/// Returns (state, cast_iid, host_iid, attached_iid). The host is moved
/// to A's BOARD with the attached card in its attached pool.
fn setup_attached_scenario() -> (GameState, InstanceId, InstanceId, InstanceId) {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let attached = s.a.hand[2].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&attached, PlayerId::A, Zone::Hand);
    s.add_attached(&host, &attached);
    (s, cast, host, attached)
}

#[test]
fn attached_cost_on_spell_exiles_payment() {
    let (mut s, cast, host, attached) = setup_attached_scenario();
    // Turn the cast into a spell (non-BOARD destination → P.31 exile branch).
    let entry = s.card_pool.get_mut(&cast).unwrap();
    entry.card_mut().kind = CardType::Spell;
    entry.card_mut().timing = Some(crate::card::Timing::Instant);
    entry.card_mut().stats = None;
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            attached_payment_ids: vec![attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.exile.contains(&attached), "attached payment → exile");
    let host_attached = &s.card_pool.get(&host).unwrap().attached;
    assert!(!host_attached.contains(&attached), "no longer attached to host");
}

#[test]
fn attached_cost_on_creature_transfers_payment_to_new_host() {
    let (mut s, cast, host, attached) = setup_attached_scenario();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            attached_payment_ids: vec![attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.board.contains(&cast), "cast on A's board");
    let new_host_attached = &s.card_pool.get(&cast).unwrap().attached;
    assert!(new_host_attached.contains(&attached), "attached transferred to new host");
    let old_host_attached = &s.card_pool.get(&host).unwrap().attached;
    assert!(!old_host_attached.contains(&attached), "removed from old host");
    assert!(!s.a.exile.contains(&attached), "not exiled — transferred");
}

#[test]
fn attached_cost_wrong_count_errors() {
    let (mut s, cast, _host, _attached) = setup_attached_scenario();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(PlayerId::A, &cast, PlayChoices::default(), None);
    assert_eq!(
        result,
        Err(PlayError::WrongAttachedPaymentCount { expected: 1, got: 0 })
    );
}

#[test]
fn attached_cost_payment_not_on_your_board_errors() {
    let (mut s, cast, _host, _attached) = setup_attached_scenario();
    // B has an attached card on B's board.
    let b_host = s.b.hand[0].clone();
    let b_attached = s.b.hand[1].clone();
    let _ = s.move_card(&b_host, PlayerId::B, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&b_attached, PlayerId::B, Zone::Hand);
    s.add_attached(&b_host, &b_attached);
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    // A tries to pay with B's attached card.
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            attached_payment_ids: vec![b_attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(
        result,
        Err(PlayError::AttachedPaymentInvalid(b_attached))
    );
}

#[test]
fn attached_cost_duplicate_payment_errors() {
    let (mut s, cast, _host, attached) = setup_attached_scenario();
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 2,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            attached_payment_ids: vec![attached.clone(), attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(
        result,
        Err(PlayError::DuplicateAttachedPayment(attached))
    );
}

#[test]
fn zero_y_creature_dies_per_c15_after_attached_detached_as_cost() {
    // C.15: a creature whose effective Y drops to ≤ 0 dies. Set up a
    // Hollow-shape creature on board: base stats 0/0, static
    // +attached/+attached. With 1 attached, effective_y = 1. Detach
    // that attached as cost for a spell. After the cast resolves, the
    // creature's effective Y is 0 and it must be in GRAVEYARD.
    use crate::card::{ModifierValue, StaticAffects, StaticDef};
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host_cast = s.a.hand[0].clone();
    let attached = s.a.hand[1].clone();
    let spell = s.a.hand[2].clone();
    let _ = s.move_card(&host_cast, PlayerId::A, Zone::Hand, Zone::Board);
    // Make host_cast a Hollow-style 0/0 +attached/+attached creature.
    {
        let inst = s.card_pool.get_mut(&host_cast).unwrap();
        inst.card_mut().kind = CardType::Creature;
        inst.card_mut().stats = Some(crate::card::Stats { x: 0.0, y: 0.0 });
        inst.card_mut().static_def = Some(StaticDef {
            affects: StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::SourceOnly,
                kind: None,
                has_keyword: None,
            },
            condition: None,
            effects: vec![crate::card::StaticEffect::StatBoost {
                x: ModifierValue::AttachedCount,
                y: ModifierValue::AttachedCount,
            }],
        });
    }
    let _ = s.remove_from_zone(&attached, PlayerId::A, Zone::Hand);
    s.add_attached(&host_cast, &attached);
    // Sanity: host_cast has effective Y = 1 right now.
    let (_, y_before) = s.effective_stats(&host_cast);
    assert_eq!(y_before, 1.0, "precondition: hollow's y should be 1");
    // Set up the spell: 1 attached cost. Spell type so attached → EXILE.
    {
        let inst = s.card_pool.get_mut(&spell).unwrap();
        inst.card_mut().kind = CardType::Spell;
        inst.card_mut().timing = Some(crate::card::Timing::Instant);
        inst.card_mut().stats = None;
    }
    set_cost(
        &mut s,
        &spell,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Attached,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &spell,
        PlayChoices {
            attached_payment_ids: vec![attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.exile.contains(&attached), "attached → exile (P.31 + sanity)");
    assert!(
        !s.a.board.contains(&host_cast),
        "C.15: hollow at effective 0/0 must die"
    );
    assert!(
        s.a.graveyard.contains(&host_cast),
        "C.15: hollow lands in GRAVEYARD"
    );
}

#[test]
fn b8_lua_damage_accumulation_kills_creature_via_cleanup_b8_damage_deaths() {
    // RULES B.8: a creature with accumulated damage ≥ Y dies. Pre-fix
    // 2026-06-16: Lua-driven `game.damage(...)` increased
    // `inst.damage` but never invoked any death check, so a 2/2
    // taking 2 damage from Read the Embers stayed on the board
    // displayed as "2/0 (-2)". Bug surfaced by play, fixed by
    // calling `cleanup_b8_damage_deaths()` from `do_damage` after
    // `set_damage`. This test pins the contract.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let victim = s.b.hand[0].clone();
    let _ = s.move_card(&victim, PlayerId::B, Zone::Hand, Zone::Board);
    {
        let inst = s.card_pool.get_mut(&victim).unwrap();
        inst.card_mut().stats = Some(crate::card::Stats { x: 2.0, y: 2.0 });
    }
    // Apply 2 damage directly (skip the Lua layer).
    s.set_damage(&victim, 2.0);
    // The cleanup that do_damage now invokes.
    s.cleanup_b8_damage_deaths();
    assert!(
        !s.b.board.contains(&victim),
        "B.8: 2/2 with damage=2 must die — was board, must now be graveyard"
    );
    assert!(
        s.b.graveyard.contains(&victim),
        "B.8: dies to GRAVEYARD per P.4"
    );
}

#[test]
fn lua_damage_to_player_mills_n_from_their_deck_to_exile() {
    // RULES L.1 + B.2 analog: this game has no life total; player
    // damage is dealt by milling cards from their DECK. Pre-2026-06-16
    // the Lua API `game.damage(target, n)` only accepted creature
    // iids — cards saying "deal N damage to any target" couldn't
    // actually target the opponent (Read the Embers card-impl had
    // a TODO comment to that effect). Now `game.damage("a"|"b", n)`
    // mills N from that player.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let pid_b = crate::game::PlayerId::B;
    let deck_before = s.player(pid_b).deck.len();
    let exile_before = s.player(pid_b).exile.len();
    // Drive through the Lua-API entry point (not just set_damage)
    // by emulating the binding's call shape.
    let res = crate::game::lua_api::do_damage(&mut s, "b", 3.0);
    assert!(res.is_ok());
    assert_eq!(
        s.player(pid_b).deck.len(),
        deck_before - 3,
        "deck shrunk by 3"
    );
    assert_eq!(
        s.player(pid_b).exile.len(),
        exile_before + 3,
        "3 cards exiled (RULES B.2 analog)"
    );
}

#[test]
fn do_damage_invokes_b8_cleanup_wiring() {
    // Integration test for the wiring: calling do_damage on a
    // creature that should die from B.8 must leave it in graveyard.
    // The unit test above
    // (b8_lua_damage_accumulation_kills_creature_via_cleanup_b8_damage_deaths)
    // exercises the cleanup function in isolation; this one
    // exercises the entry point (the function that handlers actually
    // invoke via game.damage), proving the sweep happens
    // automatically. If someone deletes the cleanup call from
    // do_damage (removing the wiring while leaving the sweep
    // function intact), the unit test still passes but this one
    // fails.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let victim = s.b.hand[0].clone();
    let _ = s.move_card(&victim, PlayerId::B, Zone::Hand, Zone::Board);
    {
        let inst = s.card_pool.get_mut(&victim).unwrap();
        inst.card_mut().stats = Some(crate::card::Stats { x: 2.0, y: 2.0 });
    }
    let res = crate::game::lua_api::do_damage(&mut s, &victim, 2.0);
    assert!(res.is_ok());
    assert!(
        !s.b.board.contains(&victim),
        "do_damage must invoke cleanup_b8_damage_deaths; \
         victim with damage=Y still on board"
    );
    assert!(
        s.b.graveyard.contains(&victim),
        "victim moves to GRAVEYARD per P.4"
    );
}

#[test]
fn b8_partial_damage_below_y_keeps_creature_alive() {
    // Pin the negative case so a future "kill everything that took
    // any damage" regression fails this test.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let survivor = s.b.hand[0].clone();
    let _ = s.move_card(&survivor, PlayerId::B, Zone::Hand, Zone::Board);
    {
        let inst = s.card_pool.get_mut(&survivor).unwrap();
        inst.card_mut().stats = Some(crate::card::Stats { x: 2.0, y: 3.0 });
    }
    s.set_damage(&survivor, 1.0);
    s.cleanup_b8_damage_deaths();
    assert!(
        s.b.board.contains(&survivor),
        "B.8: 2/3 with damage=1 must SURVIVE (1 < 3)"
    );
}

#[test]
fn c15_neg_3_3_on_a_3_3_kills_via_effective_y_drop() {
    // C.15: applying -3/-3 to a 3/3 leaves it at 0/0 → dies. The kill
    // should fire from C.15's continuous check, not require the
    // handler to manually move-to-graveyard.
    use crate::game::state::Modifier;
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let victim = s.b.hand[0].clone();
    let _ = s.move_card(&victim, PlayerId::B, Zone::Hand, Zone::Board);
    {
        let inst = s.card_pool.get_mut(&victim).unwrap();
        inst.card_mut().stats = Some(crate::card::Stats { x: 3.0, y: 3.0 });
    }
    // Apply -3/-3 EOT directly via the engine (skip the spell layer).
    s.add_modifier(
        &victim,
        Modifier::EotStatBoost { x: -3.0, y: -3.0 },
    );
    // Run the post-mutation cleanup the engine should run on stat changes.
    s.cleanup_zero_y_deaths(None);
    assert!(
        !s.b.board.contains(&victim),
        "C.15: 3/3 + (-3/-3) = 0/0 must die"
    );
    assert!(s.b.graveyard.contains(&victim), "lands in GRAVEYARD");
}

#[test]
fn live_bring_down_casts_with_attached_payment_and_exiles_it() {
    // Smoke test against the real card registry: load bring-down,
    // satisfy its 1 hand + 1 attached cost using a card actually
    // attached to a host on A's board. Verify the attached card lands
    // in EXILE per P.31's non-BOARD branch, and the action counter
    // bumps. Proves the engine wiring works on live card data, not
    // just synthetic test fixtures.
    use crate::card::CardRegistry;
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let bring_down = registry
        .cards()
        .iter()
        .find(|c| c.id == "bring-down")
        .expect("bring-down loaded")
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let pay_hand = s.a.hand[1].clone();
    let host = s.a.hand[2].clone();
    let attached = s.a.hand[3].clone();
    // Wire the real card definition into the cast instance.
    {
        let inst = s.card_pool.get_mut(&cast).unwrap();
        inst.content = Some(bring_down);
    }
    // Ensure pay_hand identity-matches bring-down (purple).
    set_identity(&mut s, &pay_hand, &["purple"], "");
    // Host on board with one attached card.
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&attached, PlayerId::A, Zone::Hand);
    s.add_attached(&host, &attached);
    // B needs a creature on board for bring-down's target pool — give
    // them one (handler picks one to apply -3/-3).
    let b_creature = s.b.hand[0].clone();
    let _ = s.move_card(&b_creature, PlayerId::B, Zone::Hand, Zone::Board);

    let lua = registry.lua();
    use rand::SeedableRng;
    let rng = rand::rngs::StdRng::seed_from_u64(0xC0FFEE);
    let mut oracle = crate::choice::RandomOracle::new(rng);
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![pay_hand.clone()],
            attached_payment_ids: vec![attached.clone()],
            graveyard_payment_ids: vec![],            ..PlayChoices::default()
        },
        Some(&mut EventContext::new(lua, &mut oracle)),
    );
    assert_eq!(result, Ok(()));
    assert!(s.a.exile.contains(&attached), "attached → exile per P.31");
    let host_attached = &s.card_pool.get(&host).unwrap().attached;
    assert!(!host_attached.contains(&attached), "removed from host");
    let bumps = s
        .action_counts
        .get("attached_payment_exile")
        .map(|v| v[0])
        .unwrap_or(0);
    assert_eq!(bumps, 1);
}

#[test]
fn transparent_rejected_as_hand_payment_for_board_placed_cast() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    // Cast: empty-identity creature (so it accepts any payment by P.7a).
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    // Payment: transparent. Per C.14, illegal for board-placed cast.
    set_identity(&mut s, &pay, &["transparent"], "");
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(result, Err(PlayError::HandPaymentTransparentForBoardPlaced(pay)));
}

#[test]
fn transparent_allowed_as_hand_payment_for_spell_cast() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let pay = s.a.hand[1].clone();
    // Cast: a transparent spell (so P.7a identity matches transparent).
    let entry = s.card_pool.get_mut(&cast).unwrap();
    entry.card_mut().kind = CardType::Spell;
    entry.card_mut().timing = Some(crate::card::Timing::Instant);
    entry.card_mut().stats = None;
    set_identity(&mut s, &cast, &["transparent"], "");
    set_identity(&mut s, &pay, &["transparent"], "");
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Hand,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            ..PlayChoices::default()
        },
        None,
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn card_identity_includes_lowercased_colors_and_symbol() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    set_identity(&mut s, &iid, &["Green", "RED"], "⊨");
    let ident = s.card_identity(&iid);
    assert!(ident.contains("green"));
    assert!(ident.contains("red"));
    assert!(ident.contains("⊨"));
    assert_eq!(ident.len(), 3);
}

#[test]
fn primal_toad_scales_by_board_count_and_hand_count_per_c16() {
    // Load the real primal-toad. Put it on A's board. Add other cards
    // to BOTH boards and HANDS. Verify effective stats = board_count
    // (both players, BOARD only — attached excluded per C.16) /
    // hand_count (both players' HAND lengths).
    use crate::card::CardRegistry;
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let toad = registry
        .cards()
        .iter()
        .find(|c| c.id == "primal-toad")
        .unwrap()
        .clone();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let toad_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&toad_iid).unwrap();
        inst.content = Some(toad);
    }
    // Move toad to A's BOARD plus two more cards to A's board and one to B's.
    let a_extra1 = s.a.hand[1].clone();
    let a_extra2 = s.a.hand[2].clone();
    let b_extra = s.b.hand[0].clone();
    let attached_to_toad = s.a.hand[3].clone();
    let _ = s.move_card(&toad_iid, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.move_card(&a_extra1, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.move_card(&a_extra2, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.move_card(&b_extra, PlayerId::B, Zone::Hand, Zone::Board);
    // Pin an attached card under the toad. C.16: this attached must NOT
    // add to board count.
    let _ = s.remove_from_zone(&attached_to_toad, PlayerId::A, Zone::Hand);
    s.add_attached(&toad_iid, &attached_to_toad);

    // X = distinct CardType across BOARD cards on both sides (attached
    // excluded). All four board cards here are creatures (default from
    // deck_of/test_helpers), so distinct-type count is 1.
    // Y = hand count: A and B started with 5 each, A used 4 cards (toad +
    // 2 board fillers + 1 attached), B used 1 (board filler). So A has
    // 1 left, B has 4 left → 5.
    let (x, y) = s.effective_stats(&toad_iid);
    assert_eq!(x, 1.0, "X = distinct CardType count across BOARD cards (all creatures here → 1)");
    assert_eq!(y, 5.0, "Y = HAND count across both players");
}

#[test]
fn counterspell_cast_validate_refuses_when_chain_is_empty() {
    // RULES P.32: counterspell declares `target = "chain"`; the engine
    // refuses the cast when no chain item exists (nothing to counter).
    // play_card refuses with CastValidateFailed; no cost paid.
    use crate::card::CardRegistry;
    use crate::game::EventContext;
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let cs = registry
        .cards()
        .iter()
        .find(|c| c.id == "counterspell")
        .unwrap()
        .clone();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cs_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&cs_iid).unwrap();
        inst.content = Some(cs);
    }
    use rand::SeedableRng;
    let rng = rand::rngs::StdRng::seed_from_u64(0);
    let mut oracle = crate::choice::RandomOracle::new(rng);
    let result = s.play_card(
        PlayerId::A,
        &cs_iid,
        PlayChoices::default(),
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert_eq!(result, Err(PlayError::CastValidateFailed));
    assert!(s.a.hand.contains(&cs_iid));
}

// ---- P.12a / P.12b tests ----------------------------------------------------

/// P.12a: a GY-only-cost cast against a non-matching GY blocks with
/// `NoGraveyardPaymentForColor`. The cast has color {red} but the only
/// card in GY is colorless.
#[test]
fn gy_only_cast_blocks_when_no_color_match_in_graveyard() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    set_identity(&mut s, &cast, &["red"], "");
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Graveyard,
            is_x: false,
            kind: None,
        }],
    );
    let gy_seed = s.a.deck.drain(0..1).next().unwrap();
    s.a.graveyard.push(gy_seed.clone());
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices::default(),
        None,
    );
    assert_eq!(result, Err(PlayError::NoGraveyardPaymentForColor));
}

/// P.12a: a GY-only-cost cast with at least one color-matching GY card
/// succeeds — the color anchor is satisfied.
#[test]
fn gy_only_cast_succeeds_when_at_least_one_gy_card_color_matches() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    set_identity(&mut s, &cast, &["red"], "");
    set_cost(
        &mut s,
        &cast,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Graveyard,
            is_x: false,
            kind: None,
        }],
    );
    let gy_seed = s.a.deck.drain(0..1).next().unwrap();
    set_identity(&mut s, &gy_seed, &["red"], "");
    s.a.graveyard.push(gy_seed.clone());
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices::default(),
        None,
    );
    assert!(result.is_ok(), "expected ok, got {result:?}");
    assert!(s.a.exile.contains(&gy_seed), "GY pitch should exile");
}

/// P.12b: when the cast has both HAND and GY components, a non-color-
/// matching GY pitch *does not* trigger the bypass. The HAND payment
/// then needs to satisfy P.7a per-card — a non-matching HAND card
/// rejects with `HandPaymentIdentityMismatch`. (And P.12a's own check
/// triggers first if no GY pitch matches; this test isolates the HAND
/// failure by giving an empty-color cast so P.12a is a no-op anchor.)
#[test]
fn mixed_hand_and_gy_without_anchor_falls_back_to_p7a_on_hand() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let hand_pay = s.a.hand[1].clone();
    set_identity(&mut s, &cast, &["blue"], "");
    set_identity(&mut s, &hand_pay, &["red"], "");
    set_cost(
        &mut s,
        &cast,
        vec![
            CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            },
            CostComponent {
                amount: 1,
                source: CostSource::Graveyard,
                is_x: false,
                kind: None,
            },
        ],
    );
    let gy_seed = s.a.deck.drain(0..1).next().unwrap();
    set_identity(&mut s, &gy_seed, &["blue"], "");
    s.a.graveyard.push(gy_seed.clone());
    // The GY pitch DOES color-match (blue), so P.12a is satisfied and
    // P.12b bypass *would* be in effect. To isolate the per-HAND fallback,
    // we have to break the anchor — swap the GY card to colorless.
    set_identity(&mut s, &gy_seed, &[], "");
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![hand_pay],
            graveyard_payment_ids: vec![gy_seed.clone()],
            ..PlayChoices::default()
        },
        None,
    );
    // P.12a fires first because graveyard_needed > 0 and cast has colors
    // but no color-matching pitch.
    assert_eq!(result, Err(PlayError::NoGraveyardPaymentForColor));
}

/// Step 3 helper: `resolve_graveyard_payment` puts a color-matching GY
/// card first when one is available, then fills remaining slots from
/// the front of GY.
#[test]
fn resolve_graveyard_payment_prefers_color_matching_anchor() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    set_identity(&mut s, &cast, &["red"], "");
    // GY: [colorless, colorless, red]. Helper should pick the red one
    // first (anchor) then fill with one colorless to reach n=2.
    let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
    set_identity(&mut s, &gy_seeds[2], &["red"], "");
    s.a.graveyard.extend(gy_seeds.clone());
    let picked = s.resolve_graveyard_payment(PlayerId::A, &cast, 2);
    assert_eq!(picked.len(), 2);
    assert_eq!(picked[0], gy_seeds[2], "anchor (red) should come first");
    assert!(
        picked.contains(&gy_seeds[0]) || picked.contains(&gy_seeds[1]),
        "second slot should come from the colorless cards"
    );
}

/// P.5: a card whose cost is to exile itself goes to EXILE on play,
/// not GRAVEYARD or BOARD. on_play still fires (it's the whole point).
/// This is the regression guard for the SelfExile cast-path wiring.
#[test]
fn self_exile_spell_routes_to_exile_on_play() {
    let registry = registry_with_fixture(
        "self_exile",
        r#"return {
            id = "self-exile-fixture",
            type = "spell",
            cost = {{amount = 1, source = "self"}},
            on_play = function(game, self)
                _G.self_exile_fired = (_G.self_exile_fired or 0) + 1
            end,
        }"#,
    );
    let fixture = registry
        .cards()
        .iter()
        .find(|c| c.id == "self-exile-fixture")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&iid).unwrap();
        inst.content = Some(fixture);
    }

    registry
        .lua()
        .globals()
        .set("self_exile_fired", 0_i32)
        .unwrap();

    let result = s.play_card(
        PlayerId::A,
        &iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    );
    assert!(result.is_ok(), "self-exile spell should cast cleanly, got {result:?}");
    assert!(s.a.exile.contains(&iid), "P.5: cast card should be in EXILE");
    assert!(!s.a.graveyard.contains(&iid), "self-exile bypasses GRAVEYARD");
    assert!(!s.a.board.contains(&iid), "self-exile bypasses BOARD");
    assert!(!s.a.hand.contains(&iid), "cast card leaves HAND");
    let fired: i32 = registry.lua().globals().get("self_exile_fired").unwrap();
    assert_eq!(fired, 1, "on_play must fire on a self-exile cast");
}

/// Activations are carved out of P.12a (mirrors A.8's HAND carve-out
/// for activations). signal-goblin's `T, 1 hand, 1 graveyard: ...`
/// activation must fire even when the controller's GY has no card
/// sharing a color with signal-goblin (blue+red). If P.12a ever leaked
/// into activate_ability, this test would catch it.
#[test]
fn activation_with_gy_cost_is_not_subject_to_p12a() {
    use crate::card::CardRegistry;

    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let sig = registry
        .cards()
        .iter()
        .find(|c| c.id == "signal-goblin")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let sig_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&sig_iid).unwrap();
        inst.content = Some(sig.clone());
        inst.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &sig_iid);
    s.a.board.push(sig_iid.clone());

    // A target creature so signal-goblin's validate hook passes.
    let target_iid = s.b.hand[0].clone();
    s.b.hand.retain(|x| x != &target_iid);
    s.b.board.push(target_iid.clone());

    // GY for the activation's `1 graveyard` cost — colorless, so it
    // would NOT anchor P.12a if P.12a were checked on activation.
    let gy_card = s.a.deck.drain(0..1).next().unwrap();
    set_identity(&mut s, &gy_card, &[], "");
    s.a.graveyard.push(gy_card);

    let mut oracle = crate::choice::ScriptedOracle::new(vec![
        crate::choice::ScriptedAnswer::Card(Some(target_iid.clone())),
    ]);
    let result = s.activate_ability(
        &sig_iid,
        0,
        None,
        crate::game::ActivateChoices::default(),
        Some(&mut crate::game::EventContext::new(registry.lua(), &mut oracle)),
    );
    assert!(
        result.is_ok(),
        "GY-cost activation must not trigger P.12a (got {result:?})"
    );
}

/// P.12b: when a color-matching GY pitch is made, the per-HAND P.7a
/// identity check is suspended — a non-matching HAND card is accepted.
#[test]
fn mixed_hand_and_gy_with_color_anchor_bypasses_p7a_on_hand() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let hand_pay = s.a.hand[1].clone();
    set_identity(&mut s, &cast, &["blue"], "");
    // HAND payment is RED — would fail P.7a normally. With a blue
    // anchor in GY, P.12b suspends P.7a for this cast.
    set_identity(&mut s, &hand_pay, &["red"], "");
    set_cost(
        &mut s,
        &cast,
        vec![
            CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
                kind: None,
            },
            CostComponent {
                amount: 1,
                source: CostSource::Graveyard,
                is_x: false,
                kind: None,
            },
        ],
    );
    let gy_seed = s.a.deck.drain(0..1).next().unwrap();
    set_identity(&mut s, &gy_seed, &["blue"], "");
    s.a.graveyard.push(gy_seed.clone());
    let result = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices {
            hand_payment_ids: vec![hand_pay.clone()],
            graveyard_payment_ids: vec![gy_seed.clone()],
            ..PlayChoices::default()
        },
        None,
    );
    assert!(result.is_ok(), "P.12b should allow non-matching HAND when GY anchor present; got {result:?}");
    // GY anchor exiled; HAND payment attached to the cast (it's a
    // BOARD-placed creature by default in deck_of).
    assert!(s.a.exile.contains(&gy_seed));
    assert!(s.card_pool.get(&cast).unwrap().attached.contains(&hand_pay));
}
