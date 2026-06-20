use super::*;
use crate::card::CardType;
use crate::game::test_helpers::*;
use crate::game::PlayChoices;

#[test]
fn new_game_deals_5_to_hand() {
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    assert_eq!(s.a.hand.len(), 5);
    assert_eq!(s.a.deck.len(), 45);
    assert_eq!(s.b.hand.len(), 5);
    assert_eq!(s.b.deck.len(), 45);
}

#[test]
fn new_game_initial_state() {
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    assert_eq!(s.active_player, PlayerId::A);
    assert_eq!(s.phase, Phase::Untap);
    assert_eq!(s.turn, 1);
    assert!(s.winner.is_none());
}

#[test]
fn new_game_card_pool_has_all_instances() {
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    assert_eq!(s.card_pool.len(), 100);
}

#[test]
fn instances_carry_owner_and_controller() {
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    for iid in s.a.hand.iter().chain(s.a.deck.iter()) {
        let inst = s.card_pool.get(iid).unwrap();
        assert_eq!(inst.owner, PlayerId::A);
        assert_eq!(inst.controller, PlayerId::A);
    }
    for iid in s.b.hand.iter().chain(s.b.deck.iter()) {
        let inst = s.card_pool.get(iid).unwrap();
        assert_eq!(inst.owner, PlayerId::B);
        assert_eq!(inst.controller, PlayerId::B);
    }
}

#[test]
fn check_loss_detects_empty_deck() {
    let s = GameState::new(deck_of(5, "a"), deck_of(50, "b"));
    assert_eq!(s.check_loss(), Some(PlayerId::A));
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    assert_eq!(s.check_loss(), None);
}

#[test]
fn effective_stats_returns_printed_without_modifiers() {
    let s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = &s.a.hand[0];
    assert_eq!(s.effective_stats(iid), (1.0, 1.0));
}

#[test]
fn effective_stats_sums_stat_boost_modifiers() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    let inst = s.card_pool.get_mut(&iid).unwrap();
    inst.modifiers.push(Modifier::StatBoost { x: 1.0, y: 0.0 });
    inst.modifiers.push(Modifier::StatBoost { x: 2.0, y: 2.0 });
    inst.modifiers.push(Modifier::StatBoost { x: -1.0, y: 1.0 });
    assert_eq!(s.effective_stats(&iid), (3.0, 4.0));
}

#[test]
fn effective_stats_includes_eot_stat_boost() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    let inst = s.card_pool.get_mut(&iid).unwrap();
    inst.modifiers.push(Modifier::StatBoost { x: 1.0, y: 1.0 });
    inst.modifiers.push(Modifier::EotStatBoost { x: 2.0, y: 0.0 });
    // Baseline 1/1 + perm +1/+1 + EOT +2/+0 = (4, 2).
    assert_eq!(s.effective_stats(&iid), (4.0, 2.0));
}

#[test]
fn clear_eot_modifiers_strips_only_eot_variants() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.add_modifier(&iid, Modifier::StatBoost { x: 1.0, y: 1.0 });
    s.add_modifier(&iid, Modifier::EotStatBoost { x: 2.0, y: 0.0 });
    s.add_modifier(&iid, Modifier::EotStatBoost { x: 1.0, y: 1.0 });
    // Before clear: (1,1) base + 1/1 perm + 2/0 eot + 1/1 eot = (5, 3).
    assert_eq!(s.effective_stats(&iid), (5.0, 3.0));
    s.clear_eot_modifiers();
    // After clear: only the permanent +1/+1 remains. (2, 2).
    assert_eq!(s.effective_stats(&iid), (2.0, 2.0));
    let inst = s.card_pool.get(&iid).unwrap();
    assert_eq!(inst.modifiers.len(), 1);
}

#[test]
fn clear_eot_modifiers_rollback_restores_original_state() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.add_modifier(&iid, Modifier::StatBoost { x: 1.0, y: 1.0 });
    s.add_modifier(&iid, Modifier::EotStatBoost { x: 2.0, y: 0.0 });
    let pre = format!("{:?}", s.card_pool.get(&iid).unwrap().modifiers);
    s.journal = Some(crate::game::Journal::new());
    s.clear_eot_modifiers();
    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    let post = format!("{:?}", s.card_pool.get(&iid).unwrap().modifiers);
    assert_eq!(pre, post, "rollback must restore the EOT modifiers in order");
}

#[test]
fn effective_stats_returns_zero_for_card_without_printed_stats() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card = card_no_stats("instant", CardType::Spell);
    assert_eq!(s.effective_stats(&iid), (0.0, 0.0));
}

#[test]
fn player_id_opponent_swaps() {
    assert_eq!(PlayerId::A.opponent(), PlayerId::B);
    assert_eq!(PlayerId::B.opponent(), PlayerId::A);
}

fn dummy_played(s: &GameState) -> StackItem {
    StackItem::PlayedCard {
        card: s.a.hand[0].clone(),
        controller: PlayerId::A,
        choices: PlayChoices::default(),
    }
}

fn dummy_played_for(card: InstanceId, controller: PlayerId) -> StackItem {
    StackItem::PlayedCard {
        card,
        controller,
        choices: PlayChoices::default(),
    }
}

#[test]
fn open_window_sets_priority_and_chain() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item = dummy_played(&s);
    s.open_response_window(item.clone()).unwrap();
    let p = s.priority.as_ref().unwrap();
    assert_eq!(p.chain, vec![item]);
    assert_eq!(p.next_to_act, s.active_player); // R.7
    assert_eq!(p.consecutive_passes, 0);
}

#[test]
fn open_window_twice_errors() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item = dummy_played(&s);
    s.open_response_window(item.clone()).unwrap();
    assert_eq!(
        s.open_response_window(item),
        Err(PriorityError::WindowAlreadyOpen),
    );
}

#[test]
fn pass_priority_without_window_errors() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    assert_eq!(s.pass_priority(), Err(PriorityError::NoWindowOpen));
}

#[test]
fn one_pass_hands_priority_to_opponent() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item = dummy_played(&s);
    s.open_response_window(item).unwrap();
    // Opens with active (A); one pass hands to B.
    assert_eq!(s.pass_priority().unwrap(), None);
    let p = s.priority.as_ref().unwrap();
    assert_eq!(p.next_to_act, PlayerId::B);
    assert_eq!(p.consecutive_passes, 1);
}

#[test]
fn two_passes_pop_and_close_when_chain_empties() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item = dummy_played(&s);
    s.open_response_window(item.clone()).unwrap();
    assert_eq!(s.pass_priority().unwrap(), None);
    assert_eq!(s.pass_priority().unwrap(), Some(item));
    assert!(s.priority.is_none());
}

#[test]
fn respond_pushes_and_flips_priority() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item_a = dummy_played(&s);
    let item_b = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
    s.open_response_window(item_a.clone()).unwrap();
    s.pass_priority().unwrap(); // A → B
    s.respond_with(item_b.clone()).unwrap(); // B responds → A
    let p = s.priority.as_ref().unwrap();
    assert_eq!(p.chain, vec![item_a, item_b]);
    assert_eq!(p.next_to_act, PlayerId::A);
    assert_eq!(p.consecutive_passes, 0);
}

#[test]
fn two_passes_with_two_items_pop_top_and_continue() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item_a = dummy_played(&s);
    let item_b = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
    s.open_response_window(item_a.clone()).unwrap();
    s.pass_priority().unwrap();
    s.respond_with(item_b.clone()).unwrap();
    // Two passes → item_b resolves; window stays open with item_a as new top.
    s.pass_priority().unwrap();
    let popped = s.pass_priority().unwrap();
    assert_eq!(popped, Some(item_b));
    let p = s.priority.as_ref().unwrap();
    assert_eq!(p.chain, vec![item_a]);
    assert_eq!(p.next_to_act, s.active_player);
    assert_eq!(p.consecutive_passes, 0);
}

#[test]
fn priority_state_round_trips_through_journal() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    s.journal = Some(crate::game::Journal::new());
    let snapshot = s.clone();
    let item = dummy_played(&s);
    let response = dummy_played_for(s.b.hand[0].clone(), PlayerId::B);
    s.open_response_window(item.clone()).unwrap();
    s.pass_priority().unwrap();
    s.respond_with(response).unwrap();
    s.journal.take().unwrap().rollback(&mut s);
    assert_eq!(s.priority, snapshot.priority);
}

#[test]
fn counter_target_removes_specific_chain_item() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item_a = dummy_played(&s);
    let b_card = s.b.hand[0].clone();
    let item_b = dummy_played_for(b_card.clone(), PlayerId::B);
    s.open_response_window(item_a.clone()).unwrap();
    s.pass_priority().unwrap();
    s.respond_with(item_b.clone()).unwrap();
    // Chain: [item_a, item_b]. Target item_a (the bottom) by its card id.
    let a_card = match &item_a {
        StackItem::PlayedCard { card, .. } => card.clone(),
    };
    let removed = s.counter_target(&a_card).unwrap();
    assert_eq!(removed, item_a);
    // item_b should still be on the chain.
    let p = s.priority.as_ref().unwrap();
    assert_eq!(p.chain.len(), 1);
    assert_eq!(p.chain[0], item_b);
    assert_eq!(p.next_to_act, s.active_player);
    assert_eq!(p.consecutive_passes, 0);
}

#[test]
fn counter_target_returns_none_for_missing_target() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let item = dummy_played(&s);
    s.open_response_window(item).unwrap();
    assert_eq!(s.counter_target(&"nonexistent".to_string()), None);
}

#[test]
fn legal_counter_targets_returns_chain_cards_in_order() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let a_card = s.a.hand[0].clone();
    let b_card = s.b.hand[0].clone();
    let item_a = dummy_played_for(a_card.clone(), PlayerId::A);
    let item_b = dummy_played_for(b_card.clone(), PlayerId::B);
    assert_eq!(s.legal_counter_targets(), Vec::<InstanceId>::new());
    s.open_response_window(item_a).unwrap();
    s.pass_priority().unwrap();
    s.respond_with(item_b).unwrap();
    assert_eq!(s.legal_counter_targets(), vec![a_card, b_card]);
}

fn make_anthem_source(s: &mut GameState, iid: &InstanceId, subtype: &str, dx: f32, dy: f32) {
    let inst = s.card_pool.get_mut(iid).unwrap();
    inst.card.subtypes.push(subtype.to_string());
    inst.card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![subtype.to_ascii_lowercase()],
            colors: vec![],
            controller: Some(crate::card::StaticController::Owner),
            exclude_self: true,
            scope: crate::card::StaticScope::Board,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::StatBoost {
            x: crate::card::ModifierValue::Fixed(dx),
            y: crate::card::ModifierValue::Fixed(dy),
        }],
    });
}

#[test]
fn anthem_applies_to_matching_subtype_on_board() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let anthem = s.a.hand[0].clone();
    let target = s.a.hand[1].clone();
    let unrelated = s.a.hand[2].clone();
    // Make target a human, unrelated a goblin.
    s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
    s.card_pool.get_mut(&unrelated).unwrap().card.subtypes = vec!["goblin".into()];
    // anthem source is a human anthem.
    make_anthem_source(&mut s, &anthem, "human", 1.0, 1.0);
    // Put all three on A's board.
    s.a.hand.retain(|i| i != &anthem && i != &target && i != &unrelated);
    s.a.board.push(anthem.clone());
    s.a.board.push(target.clone());
    s.a.board.push(unrelated.clone());

    // Target (human) gets boosted; unrelated (goblin) does not; source
    // doesn't self-boost.
    assert_eq!(s.effective_stats(&target), (2.0, 2.0));
    assert_eq!(s.effective_stats(&unrelated), (1.0, 1.0));
    assert_eq!(s.effective_stats(&anthem), (1.0, 1.0));
}

#[test]
fn anthem_removed_when_source_leaves_board() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let anthem = s.a.hand[0].clone();
    let target = s.a.hand[1].clone();
    s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
    make_anthem_source(&mut s, &anthem, "human", 1.0, 1.0);
    s.a.hand.retain(|i| i != &anthem && i != &target);
    s.a.board.push(anthem.clone());
    s.a.board.push(target.clone());
    assert_eq!(s.effective_stats(&target), (2.0, 2.0));
    // Move anthem to graveyard — boost evaporates.
    s.a.board.retain(|i| i != &anthem);
    s.a.graveyard.push(anthem);
    assert_eq!(s.effective_stats(&target), (1.0, 1.0));
}

#[test]
fn attached_host_scope_grants_keyword_to_host() {
    // Companion-bird shape: a card with `scope = AttachedHost` +
    // `modifier_keyword = "flying"` grants flying to whatever host it's
    // attached to, and to nothing else.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let bird = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let bystander = s.a.hand[2].clone();
    // Bird = attached-host flying-granter.
    s.card_pool.get_mut(&bird).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::KeywordGrant("flying".into())],
    });
    // Move host + bystander to board.
    s.a.hand.retain(|i| i != &bird && i != &host && i != &bystander);
    s.a.board.push(host.clone());
    s.a.board.push(bystander.clone());
    // Attach bird to host (companion-bird arrives as a HAND payment).
    s.add_attached(&host, &bird);
    // Host gains flying via the AttachedHost static. Bystander does not.
    assert!(s.has_keyword(&host, "flying"));
    assert!(!s.has_keyword(&bystander, "flying"));
}

#[test]
fn attached_host_scope_does_not_grant_when_unattached() {
    // Same source card, but the bird is on the BOARD (not attached) —
    // the AttachedHost predicate has no host to point at.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let bird = s.a.hand[0].clone();
    let target = s.a.hand[1].clone();
    s.card_pool.get_mut(&bird).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::KeywordGrant("flying".into())],
    });
    s.a.hand.retain(|i| i != &bird && i != &target);
    s.a.board.push(bird);
    s.a.board.push(target.clone());
    assert!(!s.has_keyword(&target, "flying"));
}

#[test]
fn condition_gate_blocks_static_until_graveyard_threshold() {
    // Ossuary-shape: static fires only when owner's graveyard has 5+ cards.
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let source = s.a.hand[0].clone();
    let target = s.a.hand[1].clone();
    s.card_pool.get_mut(&target).unwrap().card.kind = crate::card::CardType::Creature;
    s.card_pool.get_mut(&source).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: Some(crate::card::StaticController::Owner),
            exclude_self: true,
            scope: crate::card::StaticScope::Board,
            kind: Some(crate::card::CardType::Creature),
            has_keyword: None,
        },
        condition: Some(crate::card::StaticCondition::OwnerGraveyardSize { min: 5 }),
        effects: vec![
            crate::card::StaticEffect::StatBoost {
                x: crate::card::ModifierValue::Fixed(1.0),
                y: crate::card::ModifierValue::Fixed(1.0),
            },
            crate::card::StaticEffect::KeywordGrant("flying".into()),
        ],
    });
    s.a.hand.retain(|i| i != &source && i != &target);
    s.a.board.push(source);
    s.a.board.push(target.clone());

    // Empty graveyard: condition fails, no boost, no flying.
    assert_eq!(s.effective_stats(&target), (1.0, 1.0));
    assert!(!s.has_keyword(&target, "flying"));

    // Move 5 cards from A's deck to graveyard.
    let to_mill: Vec<_> = s.a.deck.iter().take(5).cloned().collect();
    for iid in to_mill {
        s.a.deck.retain(|x| x != &iid);
        s.a.graveyard.push(iid);
    }
    assert_eq!(s.a.graveyard.len(), 5);

    // Now the condition is met: +1/+1 + flying applies.
    assert_eq!(s.effective_stats(&target), (2.0, 2.0));
    assert!(s.has_keyword(&target, "flying"));
}

#[test]
fn condition_non_creatures_counts_only_non_creature_kinds() {
    // Wandering-wizard-shape: the static counts NON-creature cards in
    // graveyard. A graveyard full of creatures should NOT trigger it.
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let wizard = s.a.hand[0].clone();
    s.card_pool.get_mut(&wizard).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::SourceOnly,
            kind: None,
            has_keyword: None,
        },
        condition: Some(crate::card::StaticCondition::OwnerGraveyardNonCreatures { min: 4 }),
        effects: vec![crate::card::StaticEffect::KeywordGrant("flying".into())],
    });
    s.a.hand.retain(|i| i != &wizard);
    s.a.board.push(wizard.clone());

    // Fill graveyard with creatures: deck_of() makes every card a creature.
    let to_mill: Vec<_> = s.a.deck.iter().take(6).cloned().collect();
    for iid in to_mill {
        s.a.deck.retain(|x| x != &iid);
        s.a.graveyard.push(iid);
    }
    // Graveyard has 6 cards but they're all creatures → non-creature count
    // is 0 → flying NOT granted.
    assert_eq!(s.a.graveyard.len(), 6);
    assert!(!s.has_keyword(&wizard, "flying"));

    // Flip 4 of them to Spell — non-creature count hits 4 → flying ON.
    let gy = s.a.graveyard.clone();
    for iid in gy.iter().take(4) {
        s.card_pool.get_mut(iid).unwrap().card.kind = crate::card::CardType::Spell;
    }
    assert!(s.has_keyword(&wizard, "flying"));
}

#[test]
fn source_only_scope_targets_only_the_source() {
    // SourceOnly scope: the static targets the source card itself, not
    // other on-board cards even if they match other predicates.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let wizard = s.a.hand[0].clone();
    let other = s.a.hand[1].clone();
    s.card_pool.get_mut(&wizard).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::SourceOnly,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::KeywordGrant("flying".into())],
    });
    s.a.hand.retain(|i| i != &wizard && i != &other);
    s.a.board.push(wizard.clone());
    s.a.board.push(other.clone());
    assert!(s.has_keyword(&wizard, "flying"));
    assert!(!s.has_keyword(&other, "flying"));
}

#[test]
fn restriction_cannot_attack_propagates_to_opponent_insects() {
    // Flesh-eating-plant shape: opponent's insects get CannotAttack.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let plant = s.b.hand[0].clone();
    let opp_insect = s.a.hand[0].clone();
    let own_insect = s.b.hand[1].clone();
    s.card_pool.get_mut(&opp_insect).unwrap().card.subtypes = vec!["insect".into()];
    s.card_pool.get_mut(&own_insect).unwrap().card.subtypes = vec!["insect".into()];
    s.card_pool.get_mut(&plant).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec!["insect".into()],
            colors: vec![],
            controller: Some(crate::card::StaticController::Opponent),
            exclude_self: false,
            scope: crate::card::StaticScope::Board,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![
            crate::card::StaticEffect::Restrict(crate::card::Restriction::CannotAttack),
            crate::card::StaticEffect::Restrict(crate::card::Restriction::CannotBeCostPaid),
        ],
    });
    s.b.hand.retain(|i| i != &plant && i != &own_insect);
    s.a.hand.retain(|i| i != &opp_insect);
    s.b.board.push(plant);
    s.b.board.push(own_insect.clone());
    s.a.board.push(opp_insect.clone());

    // Plant is on B's board; A's insect is opponent's insect → restricted.
    // B's own insect is NOT restricted (controller filter = "opponent" of
    // the source = A; B's insect is on the same side as the source).
    assert!(s.has_restriction(&opp_insect, crate::card::Restriction::CannotAttack));
    assert!(s.has_restriction(&opp_insect, crate::card::Restriction::CannotBeCostPaid));
    assert!(!s.has_restriction(&own_insect, crate::card::Restriction::CannotAttack));
    assert!(!s.has_restriction(&own_insect, crate::card::Restriction::CannotBeCostPaid));
}

#[test]
fn restriction_cannot_attack_blocks_declare_attacker() {
    use crate::card::CardType;
    // End-to-end: declare_attacker errors out when the would-be attacker
    // has the CannotAttack restriction.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let plant = s.b.hand[0].clone();
    let attacker = s.a.hand[0].clone();
    s.card_pool.get_mut(&attacker).unwrap().card.subtypes = vec!["insect".into()];
    s.card_pool.get_mut(&attacker).unwrap().card.kind = CardType::Creature;
    s.card_pool.get_mut(&plant).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec!["insect".into()],
            colors: vec![],
            controller: Some(crate::card::StaticController::Opponent),
            exclude_self: false,
            scope: crate::card::StaticScope::Board,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::Restrict(
            crate::card::Restriction::CannotAttack,
        )],
    });
    s.b.hand.retain(|i| i != &plant);
    s.a.hand.retain(|i| i != &attacker);
    s.b.board.push(plant);
    s.a.board.push(attacker.clone());

    // Set up combat phase for player A (the would-be attacker's controller).
    s.active_player = PlayerId::A;
    s.phase = crate::game::Phase::Combat;
    s.card_pool.get_mut(&attacker).unwrap().summoning_sick = false;
    let err = s.declare_attacker(&attacker, None).unwrap_err();
    assert_eq!(err, crate::game::combat::CombatError::AttackerForbiddenByRestriction);
}

#[test]
fn affects_has_keyword_filters_by_intrinsic_or_static_grant() {
    // Scarecrow-shape: a restriction static that only affects opponent
    // creatures with the `flying` keyword (either printed or static-granted).
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let source = s.b.hand[0].clone();
    let flyer = s.a.hand[0].clone();
    let grounder = s.a.hand[1].clone();
    s.card_pool.get_mut(&flyer).unwrap().card.abilities = vec!["flying.".into()];
    s.card_pool.get_mut(&flyer).unwrap().card.kind = crate::card::CardType::Creature;
    s.card_pool.get_mut(&grounder).unwrap().card.kind = crate::card::CardType::Creature;
    s.card_pool.get_mut(&source).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: Some(crate::card::StaticController::Opponent),
            exclude_self: false,
            scope: crate::card::StaticScope::Board,
            kind: Some(crate::card::CardType::Creature),
            has_keyword: Some("flying".into()),
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::Restrict(
            crate::card::Restriction::CannotAttack,
        )],
    });
    s.b.hand.retain(|i| i != &source);
    s.a.hand.retain(|i| i != &flyer && i != &grounder);
    s.b.board.push(source);
    s.a.board.push(flyer.clone());
    s.a.board.push(grounder.clone());

    // Flyer restricted; ground creature unaffected.
    assert!(s.has_restriction(&flyer, crate::card::Restriction::CannotAttack));
    assert!(!s.has_restriction(&grounder, crate::card::Restriction::CannotAttack));
}

#[test]
fn two_anthems_stack() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let anthem_a = s.a.hand[0].clone();
    let anthem_b = s.a.hand[1].clone();
    let target = s.a.hand[2].clone();
    s.card_pool.get_mut(&target).unwrap().card.subtypes = vec!["human".into()];
    make_anthem_source(&mut s, &anthem_a, "human", 1.0, 1.0);
    make_anthem_source(&mut s, &anthem_b, "human", 2.0, 0.0);
    s.a.hand.retain(|i| i != &anthem_a && i != &anthem_b && i != &target);
    s.a.board.push(anthem_a);
    s.a.board.push(anthem_b);
    s.a.board.push(target.clone());
    // Both anthems are humans too (via make_anthem_source push), but
    // exclude_self skips self. They DO boost each other though, and the
    // target. Target: 1 + 1 + 2 = 4 / 1 + 1 + 0 = 2.
    assert_eq!(s.effective_stats(&target), (4.0, 2.0));
}

#[test]
fn opponent_controlled_anthem_does_not_affect_owner_filtered() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    // B has an "owner" anthem for humans — only B's humans should be
    // boosted, not A's.
    let b_anthem = s.b.hand[0].clone();
    let a_human = s.a.hand[0].clone();
    s.card_pool.get_mut(&a_human).unwrap().card.subtypes = vec!["human".into()];
    make_anthem_source(&mut s, &b_anthem, "human", 1.0, 1.0);
    s.b.hand.retain(|i| i != &b_anthem);
    s.a.hand.retain(|i| i != &a_human);
    s.b.board.push(b_anthem);
    s.a.board.push(a_human.clone());
    // A's human is on board, B's anthem is on board, but controller
    // filter is "owner" — B's anthem boosts only B's humans.
    assert_eq!(s.effective_stats(&a_human), (1.0, 1.0));
}

// --- effective_colors / granted_colors (Phase: static color grants) ---

fn make_glow_granter(s: &mut GameState, iid: &InstanceId, granted: &[&str]) {
    let inst = s.card_pool.get_mut(iid).unwrap();
    inst.card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: granted
            .iter()
            .map(|s| crate::card::StaticEffect::GrantColor(s.to_string()))
            .collect(),
    });
}

#[test]
fn effective_colors_returns_printed_when_no_grant() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&iid).unwrap().card.colors = vec!["green".into()];
    assert_eq!(s.effective_colors(&iid), vec!["green".to_string()]);
}

#[test]
fn effective_colors_includes_granted_from_attached_source() {
    // GFP shape: a mutation attached to host with granted_colors = ["glow"].
    // Host's effective_colors should include "glow" alongside its printed colors.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let host = s.a.hand[0].clone();
    let mutation = s.a.hand[1].clone();
    s.card_pool.get_mut(&host).unwrap().card.colors = vec!["red".into()];
    make_glow_granter(&mut s, &mutation, &["glow"]);
    s.a.hand.retain(|i| i != &host && i != &mutation);
    s.a.board.push(host.clone());
    s.add_attached(&host, &mutation);
    let mut effective = s.effective_colors(&host);
    effective.sort();
    assert_eq!(effective, vec!["glow".to_string(), "red".to_string()]);
}

#[test]
fn effective_colors_grants_multiple_colors_via_single_static() {
    // GFP grants {"green", "glow"} — both colors land on the host.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let host = s.a.hand[0].clone();
    let mutation = s.a.hand[1].clone();
    s.card_pool.get_mut(&host).unwrap().card.colors = vec!["red".into()];
    make_glow_granter(&mut s, &mutation, &["green", "glow"]);
    s.a.hand.retain(|i| i != &host && i != &mutation);
    s.a.board.push(host.clone());
    s.add_attached(&host, &mutation);
    let mut effective = s.effective_colors(&host);
    effective.sort();
    assert_eq!(
        effective,
        vec!["glow".to_string(), "green".to_string(), "red".to_string()]
    );
}

#[test]
fn effective_colors_deduplicates_grant_already_in_printed() {
    // If the printed color is "green" and the mutation also grants "green",
    // effective_colors should contain "green" once.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let host = s.a.hand[0].clone();
    let mutation = s.a.hand[1].clone();
    s.card_pool.get_mut(&host).unwrap().card.colors = vec!["green".into()];
    make_glow_granter(&mut s, &mutation, &["green", "glow"]);
    s.a.hand.retain(|i| i != &host && i != &mutation);
    s.a.board.push(host.clone());
    s.add_attached(&host, &mutation);
    let mut effective = s.effective_colors(&host);
    effective.sort();
    assert_eq!(effective, vec!["glow".to_string(), "green".to_string()]);
}

#[test]
fn effective_colors_drops_grant_when_mutation_unattached() {
    // Mutation off the board (no host) → no grant, host stays at printed colors.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let host = s.a.hand[0].clone();
    let mutation = s.a.hand[1].clone();
    s.card_pool.get_mut(&host).unwrap().card.colors = vec!["red".into()];
    make_glow_granter(&mut s, &mutation, &["glow"]);
    // Host on board, mutation NOT attached — mutation just lives in hand or wherever.
    s.a.hand.retain(|i| i != &host);
    s.a.board.push(host.clone());
    // Mutation stays in hand (un-attached).
    assert_eq!(s.effective_colors(&host), vec!["red".to_string()]);
}

// --- add_to_zone_top (deck-manipulation primitive for cantrips like Sprout) ---

#[test]
fn add_to_zone_top_inserts_at_index_0() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    // Pull a card out of A's deck and put it back on top.
    let mover = s.a.deck[3].clone();
    s.a.deck.retain(|i| i != &mover);
    assert_eq!(s.a.deck.len(), 4);
    s.add_to_zone_top(&mover, PlayerId::A, Zone::Deck);
    assert_eq!(s.a.deck[0], mover);
    assert_eq!(s.a.deck.len(), 5);
}

#[test]
fn add_to_zone_top_round_trips_through_journal() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let snapshot_deck = s.a.deck.clone();
    let mover = s.a.deck[3].clone();
    s.a.deck.retain(|i| i != &mover);
    s.journal = Some(crate::game::Journal::new());
    s.add_to_zone_top(&mover, PlayerId::A, Zone::Deck);
    s.journal.take().unwrap().rollback(&mut s);
    // After rollback, the top-insert is undone; mover is NOT in deck.
    assert!(!s.a.deck.contains(&mover));
    assert_eq!(s.a.deck.len(), 4);
    // Restore for the assert below — sanity check rollback didn't break order.
    s.a.deck.insert(3, mover.clone());
    assert_eq!(s.a.deck, snapshot_deck);
}

#[test]
fn add_to_zone_top_replay_forward_restores_state() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let mover = s.a.deck[3].clone();
    s.a.deck.retain(|i| i != &mover);
    s.replay_journal = Some(crate::game::Journal::new());
    s.add_to_zone_top(&mover, PlayerId::A, Zone::Deck);
    let recorded = s.replay_journal.take().unwrap();
    // Rebuild from scratch and replay forward.
    let mut fresh = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    fresh.a.deck.retain(|i| i != &mover);
    recorded.replay_forward(&mut fresh);
    assert_eq!(fresh.a.deck[0], mover);
    assert_eq!(fresh.a.deck.len(), 5);
}

#[test]
fn playable_responses_filters_to_zero_cost_instants() {
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    // a_hand[0] is a creature by default — not a response candidate.
    // Mutate a_hand[1] into a zero-cost instant.
    let inst = s.a.hand[1].clone();
    let card = s.card_pool.get_mut(&inst).unwrap();
    card.card.kind = crate::card::CardType::Spell;
    card.card.timing = Some(crate::card::Timing::Instant);
    card.card.cost = vec![];
    // Mutate a_hand[2] into a sorcery — should NOT be returned.
    let sorc = s.a.hand[2].clone();
    let card2 = s.card_pool.get_mut(&sorc).unwrap();
    card2.card.kind = crate::card::CardType::Spell;
    card2.card.timing = Some(crate::card::Timing::Sorcery);
    card2.card.cost = vec![];
    let candidates = s.playable_responses(PlayerId::A);
    assert!(candidates.contains(&inst));
    assert!(!candidates.contains(&sorc));
}

#[test]
fn deck_top_symbol_matches_attached_condition_grants_modifier() {
    // flyer-match shape: creature with a static that grants +3/+0 iff the
    // effective top of deck (V.8 walk) shares a symbol with any of its
    // attached cards.
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    let source = s.a.hand[0].clone();
    let attached = s.a.hand[1].clone();
    s.card_pool.get_mut(&source).unwrap().card.kind = crate::card::CardType::Creature;
    s.card_pool.get_mut(&source).unwrap().card.stats = Some(crate::card::Stats { x: 1.0, y: 1.0 });
    s.card_pool.get_mut(&source).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::SourceOnly,
            kind: None,
            has_keyword: None,
        },
        condition: Some(crate::card::StaticCondition::DeckTopSymbolMatchesAttached),
        effects: vec![crate::card::StaticEffect::StatBoost {
            x: crate::card::ModifierValue::Fixed(3.0),
            y: crate::card::ModifierValue::Fixed(0.0),
        }],
    });
    // Attach a card with symbol "alpha".
    s.card_pool.get_mut(&attached).unwrap().card.symbols = vec!["alpha".to_string()];
    s.a.hand.retain(|i| i != &source && i != &attached);
    s.a.board.push(source.clone());
    s.card_pool.get_mut(&source).unwrap().attached.push(attached);

    // Deck top has no matching symbol → no boost.
    let top0 = s.a.deck[0].clone();
    s.card_pool.get_mut(&top0).unwrap().card.symbols = vec!["beta".to_string()];
    assert_eq!(s.effective_stats(&source), (1.0, 1.0));

    // Set deck top to share "alpha" with attached → boost fires.
    s.card_pool.get_mut(&top0).unwrap().card.symbols = vec!["alpha".to_string()];
    assert_eq!(s.effective_stats(&source), (4.0, 1.0));

    // V.8: insert a transparent-frame card at the top of the deck. The
    // effective top is still the alpha card below — boost still fires.
    let transparent = s.b.hand[0].clone();
    s.card_pool.get_mut(&transparent).unwrap().card.frame = Some("transparent".to_string());
    s.card_pool.get_mut(&transparent).unwrap().card.symbols = vec![];
    s.b.hand.retain(|i| i != &transparent);
    s.a.deck.insert(0, transparent);
    assert_eq!(s.effective_stats(&source), (4.0, 1.0));

    // Swap the alpha card under the transparent for a non-matching one;
    // boost goes away despite the transparent still being on top.
    let underneath = s.a.deck[1].clone();
    s.card_pool.get_mut(&underneath).unwrap().card.symbols = vec!["gamma".to_string()];
    assert_eq!(s.effective_stats(&source), (1.0, 1.0));
}

#[test]
fn effective_top_of_deck_symbols_walks_through_transparent_per_v8() {
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
    // deck[0] transparent, deck[1] also transparent, deck[2] opaque with "alpha".
    let t0 = s.a.deck[0].clone();
    let t1 = s.a.deck[1].clone();
    let opaque = s.a.deck[2].clone();
    s.card_pool.get_mut(&t0).unwrap().card.frame = Some("transparent".to_string());
    s.card_pool.get_mut(&t1).unwrap().card.frame = Some("transparent".to_string());
    s.card_pool.get_mut(&opaque).unwrap().card.symbols = vec!["alpha".to_string()];
    assert_eq!(
        s.effective_top_of_deck_symbols(PlayerId::A),
        vec!["alpha".to_string()],
    );
}

#[test]
fn host_loses_colors_true_when_attached_mutation_declares_colorless() {
    // Nonsense Mutation shape: attached_host-scope static with
    // `makes_host_colorless = true` causes the host to lose its color
    // identity. Bystander on the board (no attached suppressor) is
    // unaffected.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let bystander = s.a.hand[2].clone();
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::MakesHostColorless],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host && i != &bystander);
    s.a.board.push(host.clone());
    s.a.board.push(bystander.clone());
    s.add_attached(&host, &mutation);

    assert!(s.host_loses_colors(&host), "host with attached colorless mutation must lose colors");
    assert!(!s.host_loses_colors(&bystander), "bystander has no attached suppressor");
}

#[test]
fn effective_colors_returns_empty_when_host_loses_colors() {
    // Host with printed `colors = ["red"]` plus an attached colorless
    // mutation → effective_colors evaporates the identity entirely.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    s.card_pool.get_mut(&host).unwrap().card.colors = vec!["red".into()];
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::MakesHostColorless],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host);
    s.a.board.push(host.clone());
    s.add_attached(&host, &mutation);

    assert_eq!(s.effective_colors(&host), Vec::<String>::new());
}

#[test]
fn host_loses_abilities_true_when_attached_mutation_declares_suppression() {
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let bystander = s.a.hand[2].clone();
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::SuppressesHostAbilities],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host && i != &bystander);
    s.a.board.push(host.clone());
    s.a.board.push(bystander.clone());
    s.add_attached(&host, &mutation);

    assert!(s.host_loses_abilities(&host));
    assert!(!s.host_loses_abilities(&bystander));
}

#[test]
fn hosts_own_static_stops_applying_when_suppressed() {
    // Host has a self-boosting static (SourceOnly scope, +2/+2 on
    // itself). Without suppression, effective_stats reflects the boost.
    // With an attached suppressor mutation, the host's own static is
    // dropped from static iteration → stats fall back to printed.
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    // Host self-static: +2/+2 via SourceOnly scope.
    s.card_pool.get_mut(&host).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
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
            x: crate::card::ModifierValue::Fixed(2.0),
            y: crate::card::ModifierValue::Fixed(2.0),
        }],
    });
    // Suppressor mutation: AttachedHost + suppresses_host_abilities.
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::SuppressesHostAbilities],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host);
    s.a.board.push(host.clone());
    // Sanity: with no suppressor attached, the self-static fires → 3/3
    // (printed 1/1 + self-boost 2/2). Attach the suppressor → drops to
    // printed.
    assert_eq!(s.effective_stats(&host), (3.0, 3.0), "pre-attach baseline");
    s.add_attached(&host, &mutation);
    assert_eq!(s.effective_stats(&host), (1.0, 1.0), "host's own static must stop applying when suppressed");
}

#[test]
fn modifier_value_fixed_carries_fractional_value() {
    // Attached-host mutation with y = -0.5. Host's printed stats are
    // 1/1; with the mutation attached, effective_stats must reflect
    // 1.0 / 0.5 — not 1.0 / 1.0 (truncation to 0).
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::StatBoost {
            x: crate::card::ModifierValue::Fixed(0.0),
            y: crate::card::ModifierValue::Fixed(-0.5),
        }],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host);
    s.a.board.push(host.clone());
    s.add_attached(&host, &mutation);
    assert_eq!(s.effective_stats(&host), (1.0, 0.5));
}

#[test]
fn board_count_by_face_modifier_counts_shiny_board_cards() {
    // Host on board with an attached-host mutation whose modifier_x =
    // BoardCountByFace("shiny"). Add three shiny cards to the board
    // (mix of own + opponent); assert host gets +3 to X.
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let shiny_a1 = s.a.hand[2].clone();
    let shiny_a2 = s.a.hand[3].clone();
    let shiny_b = s.b.hand[0].clone();
    // Tag three cards as shiny on `face`.
    for iid in [&shiny_a1, &shiny_a2, &shiny_b] {
        s.card_pool.get_mut(iid).unwrap().card.face = vec!["shiny".into()];
    }
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::StatBoost {
            x: crate::card::ModifierValue::BoardCountByFace("shiny".into()),
            y: crate::card::ModifierValue::Fixed(0.0),
        }],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host && i != &shiny_a1 && i != &shiny_a2);
    s.b.hand.retain(|i| i != &shiny_b);
    s.a.board.push(host.clone());
    s.a.board.push(shiny_a1);
    s.a.board.push(shiny_a2);
    s.b.board.push(shiny_b);
    s.add_attached(&host, &mutation);
    // Printed 1/1 + (BoardCountByFace("shiny") = 3) → 4/1.
    assert_eq!(s.effective_stats(&host), (4.0, 1.0));
}

#[test]
fn sum_and_scaled_modifier_compose_offset_plus_per_face_contribution() {
    // Missense Mutation Y-axis shape: -0.5 base + (-0.25 × shiny_count).
    // With 2 shiny board cards: -0.5 + -0.5 = -1.0. Host's printed Y is
    // 1.0, so effective Y = 0.0.
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let mutation = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let shiny1 = s.a.hand[2].clone();
    let shiny2 = s.a.hand[3].clone();
    for iid in [&shiny1, &shiny2] {
        s.card_pool.get_mut(iid).unwrap().card.face = vec!["shiny".into()];
    }
    use crate::card::ModifierValue;
    s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(crate::card::StaticDef {
        affects: crate::card::StaticAffects {
            subtypes: vec![],
            colors: vec![],
            controller: None,
            exclude_self: false,
            scope: crate::card::StaticScope::AttachedHost,
            kind: None,
            has_keyword: None,
        },
        condition: None,
        effects: vec![crate::card::StaticEffect::StatBoost {
            x: ModifierValue::Fixed(0.0),
            y: ModifierValue::Sum(vec![
                ModifierValue::Fixed(-0.5),
                ModifierValue::Scaled(-0.25, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ]),
        }],
    });
    s.a.hand.retain(|i| i != &mutation && i != &host && i != &shiny1 && i != &shiny2);
    s.a.board.push(host.clone());
    s.a.board.push(shiny1);
    s.a.board.push(shiny2);
    s.add_attached(&host, &mutation);
    // Printed 1/1 + modifier (0.0, -1.0) → 1.0/0.0.
    assert_eq!(s.effective_stats(&host), (1.0, 0.0));
}

#[test]
fn pending_main_phase_returns_flush_on_main1_entry() {
    // Card sits in A's exile; queue it for next-main-phase return.
    // Stepping into Main1 must move it from exile to A's board and
    // clear the queue.
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let iid = s.a.hand[0].clone();
    s.a.hand.retain(|i| i != &iid);
    s.a.exile.push(iid.clone());
    s.pending_main_phase_returns.push(iid.clone());
    assert_eq!(s.phase, Phase::Untap);
    s.next_phase(None).expect("None ctx never yields"); // Untap → Draw
    s.next_phase(None).expect("None ctx never yields"); // Draw  → Main1
    assert_eq!(s.phase, Phase::Main1);
    assert!(s.a.board.contains(&iid), "queued card must return to owner's board");
    assert!(!s.a.exile.contains(&iid), "queued card must leave exile");
    assert!(s.pending_main_phase_returns.is_empty(), "queue must clear after flush");
}

#[test]
fn pending_main_phase_returns_flush_on_main2_entry() {
    // Same but starting after Main1 — phase advances through Main1
    // (no queued items) → Combat → Main2 (flushes).
    let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
    let iid = s.a.hand[0].clone();
    s.a.hand.retain(|i| i != &iid);
    s.a.exile.push(iid.clone());
    s.next_phase(None).expect("None ctx never yields"); // Untap → Draw
    s.next_phase(None).expect("None ctx never yields"); // Draw  → Main1
    // Queue AFTER Main1 entry so the first flush doesn't catch it.
    s.pending_main_phase_returns.push(iid.clone());
    s.next_phase(None).expect("None ctx never yields"); // Main1 → Combat
    s.next_phase(None).expect("None ctx never yields"); // Combat → Main2
    assert_eq!(s.phase, Phase::Main2);
    assert!(s.a.board.contains(&iid), "queued card must return on Main2 entry");
    assert!(!s.a.exile.contains(&iid));
    assert!(s.pending_main_phase_returns.is_empty());
}

/// Pins A.12: `effective_combined_cost` walks per-source cost components,
/// applies every on-board cost-reduction static via `cost_reduction`, clamps
/// per-source to 0 (P.20), and sums. Reincubator-style setup: -1 hand, -1
/// graveyard for any creature whose colors include green or black.
#[test]
fn effective_combined_cost_applies_color_gated_reduction() {
    use crate::card::{CardType, CostComponent, CostSource};
    let mut s = GameState::new(deck_of(5, "a"), deck_of(5, "b"));
    let source_iid = s.a.hand[0].clone();
    let green_target = s.a.hand[1].clone();
    let black_target = s.a.hand[2].clone();
    let blue_target = s.a.hand[3].clone();

    // Shape three target creatures: same printed cost (2 hand + 2 graveyard
    // = combined 4), different colors. Green and black should match
    // Reincubator's affects predicate; blue should not.
    for (iid, colors) in [
        (&green_target, vec!["green".to_string()]),
        (&black_target, vec!["black".to_string()]),
        (&blue_target, vec!["blue".to_string()]),
    ] {
        let c = &mut s.card_pool.get_mut(iid).unwrap().card;
        c.colors = colors;
        c.kind = CardType::Creature;
        c.cost = vec![
            CostComponent { amount: 2, source: CostSource::Hand, is_x: false, kind: None },
            CostComponent { amount: 2, source: CostSource::Graveyard, is_x: false, kind: None },
        ];
    }

    // Install the Reincubator-style source on A's board.
    {
        let inst = s.card_pool.get_mut(&source_iid).unwrap();
        inst.card.static_def = Some(crate::card::StaticDef {
            affects: crate::card::StaticAffects {
                subtypes: vec![],
                colors: vec!["green".into(), "black".into()],
                controller: None, // both players
                exclude_self: false,
                scope: crate::card::StaticScope::Board,
                kind: Some(CardType::Creature),
                has_keyword: None,
            },
            condition: None,
            effects: vec![
                crate::card::StaticEffect::CostModify { source: CostSource::Hand, amount: 1 },
                crate::card::StaticEffect::CostModify { source: CostSource::Graveyard, amount: 1 },
            ],
        });
    }
    s.a.hand.retain(|i| i != &source_iid);
    s.a.board.push(source_iid);

    // Green and black qualify → printed 4, effective 2 (each source -1).
    assert_eq!(s.effective_combined_cost(&green_target), 2);
    assert_eq!(s.effective_combined_cost(&black_target), 2);
    // Blue does not qualify → printed 4, effective 4 (no reduction matches).
    assert_eq!(s.effective_combined_cost(&blue_target), 4);
}
