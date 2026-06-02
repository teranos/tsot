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
        },
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
    };
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
        },
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
        },
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
        },
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
        },
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
    s.card_pool.get_mut(&iid).unwrap().card.kind = CardType::Environment;
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Err(PlayError::UnsupportedType(CardType::Environment))
    );
}

#[test]
fn play_card_routes_artifact_to_board() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card.kind = CardType::Artifact;
    // No cost — empty payment is valid for an artifact with cost = {}.
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Ok(())
    );
    assert!(s.a.board.contains(&iid));
    assert!(!s.a.hand.contains(&iid));
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
        j.card.kind = CardType::Artifact;
        j.card.subtypes = vec!["jewel".to_string()];
        j.card.colors = vec!["red".to_string()];
    }
    // Move the jewel to BOARD (untapped) so it can be tapped as cost.
    let _ = s.move_card(&jewel, PlayerId::A, Zone::Hand, Zone::Board);
    // Build the cast card: creature, red, 1 hand cost.
    {
        let c = s.card_pool.get_mut(&cast).unwrap();
        c.card.colors = vec!["red".to_string()];
    }
    // P.7a identity match: payment needs a shared color with cast.
    {
        let p = s.card_pool.get_mut(&payment).unwrap();
        p.card.colors = vec!["red".to_string()];
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
        },
        None,
    );
    assert_eq!(result, Ok(()));
    // Jewel is now tapped on BOARD.
    let jewel_inst = s.card_pool.get(&jewel).unwrap();
    assert!(jewel_inst.tapped);
    assert!(s.a.board.contains(&jewel));
    // Cast card on BOARD (it's a creature).
    assert!(s.a.board.contains(&cast));
    // The substituted payment slot wasn't pitched from hand: bump_action
    // for jewel_tap_substitution recorded.
    let bumps = s
        .action_counts
        .get("jewel_tap_substitution")
        .map(|v| v[0])
        .unwrap_or(0);
    assert_eq!(bumps, 1);
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
        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(jewel)));
}

#[test]
fn jewel_tap_rejected_on_color_mismatch() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Recolor the cast card to blue — jewel is red, no overlap.
    s.card_pool.get_mut(&cast).unwrap().card.colors = vec!["blue".to_string()];
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
        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(jewel)));
}

#[test]
fn jewel_tap_rejected_on_non_jewel_artifact() {
    let (mut s, jewel, cast, _payment) = setup_jewel_tap_scenario();
    // Strip the "jewel" subtype — still a red artifact on board, but not a jewel.
    s.card_pool.get_mut(&jewel).unwrap().card.subtypes.clear();
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
        },
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
        },
        None,
    );
    assert_eq!(result, Err(PlayError::JewelTapWithoutHandCost));
}

#[test]
fn jewel_tap_plus_hand_payment_splits_cost_correctly() {
    // 2 HAND cost, 1 jewel-tap + 1 hand payment.
    let (mut s, jewel, cast, payment) = setup_jewel_tap_scenario();
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
            hand_payment_ids: vec![payment.clone()],
            x_value: None,
            jewel_tap: Some(jewel.clone()),
            sacrifice_ids: vec![],
            mutation_target: None,
            gy_hand_payment_ids: vec![],
            attached_payment_ids: vec![],
        },
        None,
    );
    assert_eq!(result, Ok(()));
    assert!(s.card_pool.get(&jewel).unwrap().tapped);
    // The payment card is now attached to the cast (creature ETB path).
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
        c.card.kind = CardType::Artifact;
        c.card.subtypes = vec!["crystal".into()];
        c.card.colors = vec![
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
    s.card_pool.get_mut(&attached).unwrap().card.colors = vec!["red".into()];
    let _ = s.move_card(&attached, PlayerId::A, Zone::Hand, Zone::Exile); // remove from hand
    s.a.exile.retain(|x| x != &attached);
    s.add_attached(&crystal, &attached);
    // Cast card: red, 1 hand cost.
    s.card_pool.get_mut(&cast).unwrap().card.colors = vec!["red".into()];
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
        },
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
        c.card.kind = CardType::Artifact;
        c.card.subtypes = vec!["crystal".into()];
        c.card.colors = vec!["black".into(), "red".into(), "blue".into()];
    }
    let _ = s.move_card(&crystal, PlayerId::A, Zone::Hand, Zone::Board);
    // Attached card is GREEN — does not match the BLUE cast card.
    s.card_pool.get_mut(&attached).unwrap().card.colors = vec!["green".into()];
    let _ = s.move_card(&attached, PlayerId::A, Zone::Hand, Zone::Exile);
    s.a.exile.retain(|x| x != &attached);
    s.add_attached(&crystal, &attached);
    s.card_pool.get_mut(&cast).unwrap().card.colors = vec!["blue".into()];
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
        },
        None,
    );
    assert_eq!(result, Err(PlayError::InvalidJewelTap(crystal)));
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
        },
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
        },
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
        },
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
        },
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
fn play_card_errors_on_unsupported_cost_source() {
    // SelfExile (P.5) is the last unsupported source; Hand / Mill /
    // Graveyard / Sacrifice are all routable.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 1,
            source: CostSource::SelfExile,
            is_x: false,
            kind: None,
        }],
    );
    let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(
        result,
        Err(PlayError::UnsupportedCostSource(CostSource::SelfExile))
    );
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
        inst.card = jelly.clone();
    }
    // Put an opposing creature on B's board to be the target.
    s.b.hand.retain(|x| x != &target_iid);
    s.b.board.push(target_iid.clone());

    // Seed graveyard for the 3-graveyard cost.
    let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
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
        },
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
        inst.card.handlers = fixture.handlers.clone();
        inst.card.id = fixture.id.clone();
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
        inst.card = surge.clone();
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
        },
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
        inst.card = draw_two.clone();
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
        inst.card.handlers = scribe.handlers.clone();
        inst.card.id = scribe.id.clone();
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
        inst.card.handlers = fixture.handlers.clone();
        inst.card.id = fixture.id.clone();
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
    s.card_pool.get_mut(&cs_iid).unwrap().card = counterspell;

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
        },
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
    s.card_pool.get_mut(&b_instant).unwrap().card.kind = crate::card::CardType::Spell;
    s.card_pool.get_mut(&b_instant).unwrap().card.timing = Some(crate::card::Timing::Instant);
    s.card_pool.get_mut(&b_instant).unwrap().card.cost = vec![]; // free

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
    s.card_pool.get_mut(&cs_iid).unwrap().card = card;

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
        },
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
        },
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
        },
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
    s.card_pool.get_mut(&creature).unwrap().card.colors = vec!["green".into()];
    s.card_pool.get_mut(&creature).unwrap().card.symbols =
        vec!["⊨".into(), "⨳".into()];
    s.card_pool.get_mut(&pay).unwrap().card.colors = vec!["red".into()];
    s.card_pool.get_mut(&pay).unwrap().card.symbols = vec!["⨳".into()];
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
        },
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
    s.card_pool.get_mut(&creature).unwrap().card.colors = vec!["green".into()];
    s.card_pool.get_mut(&creature).unwrap().card.symbols =
        vec!["⊨".into(), "⨳".into()];
    s.card_pool.get_mut(&pay).unwrap().card.colors = vec!["red".into()];
    s.card_pool.get_mut(&pay).unwrap().card.symbols = vec!["꩜".into()];
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
        },
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
        },
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
        },
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
        },
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
        inst.card = jewel;
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
    let second = s.activate_ability(&iid, 0, None, None);
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
        inst.card = vh;
        inst.summoning_sick = true; // freshly played, no haste
    }
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    assert!(!s.can_activate(&iid, 0));
    let result = s.activate_ability(&iid, 0, None, None);
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
    s.card_pool.get_mut(&iid).unwrap().card = jewel;
    s.a.hand.retain(|x| x != &iid);
    s.a.board.push(iid.clone());

    // red-jewel has exactly one activated ability.
    assert!(!s.can_activate(&iid, 1));
    assert_eq!(s.activate_ability(&iid, 1, None, None), Err(ActivateError::NoSuchAbility));
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
        inst.card = vh;
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
        inst.card = vh;
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
        inst.card = monkey;
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
        inst.card = monkey;
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
    let result = s.activate_ability(&iid, 0, None, None);
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
        m.card = monkey;
        m.summoning_sick = false;
    }
    {
        // Give buddy a baseline 1/1 stat line.
        let b = s.card_pool.get_mut(&buddy_iid).unwrap();
        b.card.stats = Some(crate::card::Stats { x: 1, y: 1 });
        b.card.kind = crate::card::CardType::Creature;
        b.summoning_sick = false;
    }
    s.a.hand.retain(|x| x != &monkey_iid && x != &buddy_iid);
    s.a.board.push(monkey_iid.clone());
    s.a.board.push(buddy_iid.clone());

    // Pre: buddy is 1/1, no vigilance.
    let (bx0, by0) = s.effective_stats(&buddy_iid);
    assert_eq!((bx0, by0), (1, 1));
    assert!(!s.has_keyword(&buddy_iid, "vigilance"));

    // Activate.
    let mut oracle = crate::choice::NoopOracle;
    s.activate_ability(
        &monkey_iid,
        0,
        None,
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    )
    .unwrap();

    // Post: buddy got +2/+2 EOT and vigilance EOT.
    let (bx1, by1) = s.effective_stats(&buddy_iid);
    assert_eq!((bx1, by1), (3, 3));
    assert!(s.has_keyword(&buddy_iid, "vigilance"));

    // Self-pump: monkey itself also gained +2/+2 and vigilance.
    let (mx, my) = s.effective_stats(&monkey_iid);
    assert_eq!((mx, my), (4, 4));
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
        inst.card = pink;
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
        inst.card = pink;
        inst.summoning_sick = false;
    }
    {
        // Put an opposing creature on B's board so validate passes.
        let t = s.card_pool.get_mut(&target).unwrap();
        t.card.kind = crate::card::CardType::Creature;
        t.card.stats = Some(crate::card::Stats { x: 1, y: 1 });
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
        inst.card = sala;
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
        host_inst.card.kind = crate::card::CardType::Creature;
        host_inst.card.stats = Some(crate::card::Stats { x: 2, y: 2 });
        host_inst.summoning_sick = false;
    }
    {
        let jewel_inst = s.card_pool.get_mut(&jewel_iid).unwrap();
        jewel_inst.card = jewel;
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
    s.card_pool.get_mut(&cv).unwrap().card = clear_view;
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
        },
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
    s.card_pool.get_mut(&cv).unwrap().card = clear_view;
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
        },
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
    entry.card.kind = CardType::Spell;
    entry.card.timing = Some(crate::card::Timing::Instant);
    entry.card.stats = None;
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
            ..PlayChoices::default()
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
            ..PlayChoices::default()
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
            ..PlayChoices::default()
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
            ..PlayChoices::default()
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
    use crate::card::{ModifierValue, Restriction, StaticAffects, StaticDef};
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host_cast = s.a.hand[0].clone();
    let attached = s.a.hand[1].clone();
    let spell = s.a.hand[2].clone();
    let _ = s.move_card(&host_cast, PlayerId::A, Zone::Hand, Zone::Board);
    // Make host_cast a Hollow-style 0/0 +attached/+attached creature.
    {
        let inst = s.card_pool.get_mut(&host_cast).unwrap();
        inst.card.kind = CardType::Creature;
        inst.card.stats = Some(crate::card::Stats { x: 0, y: 0 });
        inst.card.static_def = Some(StaticDef {
            affects: StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: crate::card::StaticScope::SourceOnly,
                kind: None,
                has_keyword: None,
            },
            modifier_x: ModifierValue::AttachedCount,
            modifier_y: ModifierValue::AttachedCount,
            modifier_keyword: None,
            condition: None,
            restrictions: Vec::<Restriction>::new(),
            cost_modifiers: vec![],
            granted_activated: None,
            granted_colors: vec![],
        });
    }
    let _ = s.remove_from_zone(&attached, PlayerId::A, Zone::Hand);
    s.add_attached(&host_cast, &attached);
    // Sanity: host_cast has effective Y = 1 right now.
    let (_, y_before) = s.effective_stats(&host_cast);
    assert_eq!(y_before, 1, "precondition: hollow's y should be 1");
    // Set up the spell: 1 attached cost. Spell type so attached → EXILE.
    {
        let inst = s.card_pool.get_mut(&spell).unwrap();
        inst.card.kind = CardType::Spell;
        inst.card.timing = Some(crate::card::Timing::Instant);
        inst.card.stats = None;
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
            ..PlayChoices::default()
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
        inst.card.stats = Some(crate::card::Stats { x: 3, y: 3 });
    }
    // Apply -3/-3 EOT directly via the engine (skip the spell layer).
    s.add_modifier(
        &victim,
        Modifier::EotStatBoost { x: -3, y: -3 },
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
        inst.card = bring_down;
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
            ..PlayChoices::default()
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
    entry.card.kind = CardType::Spell;
    entry.card.timing = Some(crate::card::Timing::Instant);
    entry.card.stats = None;
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
        inst.card = toad;
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

    // Board count: 3 on A + 1 on B = 4.
    // Hand count: A and B started with 5 each, A used 4 cards (toad +
    // 2 board fillers + 1 attached), B used 1 (board filler). So A has
    // 1 left, B has 4 left → 5.
    let (x, y) = s.effective_stats(&toad_iid);
    assert_eq!(x, 4, "X = BOARD count across both players (attached excluded per C.16)");
    assert_eq!(y, 5, "Y = HAND count across both players");
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
        inst.card = cs;
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
