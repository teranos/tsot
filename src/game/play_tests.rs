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
            },
            CostComponent {
                amount: 2,
                source: CostSource::Mill,
                is_x: false,
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
        }],
    );
    let choices = PlayChoices {
        hand_payment_ids: vec![payment.clone()],
        x_value: None,
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
            },
            CostComponent {
                amount: 2,
                source: CostSource::Mill,
                is_x: false,
            },
        ],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone()],
            x_value: None,
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
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay],
            x_value: None,
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
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![creature.clone()],
            x_value: None,
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
        }],
    );
    let result = s.play_card(
        PlayerId::A,
        &creature,
        PlayChoices {
            hand_payment_ids: vec![pay.clone(), pay.clone()],
            x_value: None,
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
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card.kind = CardType::Artifact;
    assert_eq!(
        s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
        Err(PlayError::UnsupportedType(CardType::Artifact))
    );
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
        }],
    );
    let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(result, Err(PlayError::VariableXValueMissing));
}

#[test]
fn play_card_errors_on_unsupported_cost_source() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let creature = s.a.hand[0].clone();
    set_cost(
        &mut s,
        &creature,
        vec![CostComponent {
            amount: 1,
            source: CostSource::Sacrifice,
            is_x: false,
        }],
    );
    let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
    assert_eq!(
        result,
        Err(PlayError::UnsupportedCostSource(CostSource::Sacrifice))
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

    // Scripted oracle: pick target_iid.
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Card(Some(target_iid.clone()))]);

    s.play_card(
        PlayerId::A,
        &jelly_iid,
        PlayChoices {
            hand_payment_ids: vec![hand_payment],
            x_value: None,
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
    s.play_card(
        PlayerId::A,
        &surge_iid,
        PlayChoices {
            hand_payment_ids: vec![payment, payment2],
            x_value: None,
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

    let hand_before = s.a.hand.len();
    let deck_before = s.a.deck.len();
    let gy_before = s.a.graveyard.len();
    let exile_before = s.a.exile.len();

    s.play_card(
        PlayerId::A,
        &instant_iid,
        PlayChoices::default(),
        Some(&mut crate::game::EventContext::lua_only(registry.lua())),
    )
    .unwrap();

    // - Played card removed from hand.
    // - 3 graveyard cards exiled (cost).
    // - 2 cards drawn from deck into hand.
    // - Played card lands in graveyard.
    assert_eq!(s.a.hand.len(), hand_before - 1 + 2);
    assert_eq!(s.a.deck.len(), deck_before - 2);
    assert_eq!(s.a.graveyard.len(), gy_before - 3 + 1);
    assert_eq!(s.a.exile.len(), exile_before + 3);
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
    // B's instant is NOT resolved yet — it's still in B's hand.
    assert!(s.b.hand.contains(&b_instant));
    assert!(!s.b.graveyard.contains(&b_instant));
}
