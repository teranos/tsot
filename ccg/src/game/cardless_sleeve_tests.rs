//! Cardless sleeve (Z.8) behaviour tests.
//!
//! A cardless sleeve is a sleeve-unit with no card inside (`content: None`).
//! Z.8b: it does not satisfy "draw a card" — on top of the deck it is
//! collected into hand for free and the draw continues, cascading through
//! consecutive empties until one card-bearing unit is drawn.

use super::*;
use crate::card::{CostComponent, CostSource};
use crate::game::test_helpers::*;

fn hand_cost(n: i32) -> CostComponent {
    CostComponent { amount: n, source: CostSource::Hand, is_x: false, kind: None }
}
fn cost(source: CostSource, n: i32) -> CostComponent {
    CostComponent { amount: n, source, is_x: false, kind: None }
}

/// Turn an existing deck sleeve into a cardless sleeve. Stand-in for the
/// real creation primitive until slice 8's deckbuilding / search lands.
fn make_cardless(s: &mut GameState, iid: &InstanceId) {
    s.card_pool.get_mut(iid).unwrap().content = None;
}

#[test]
fn z8b_draw_collects_cardless_sleeves_free_then_draws_one_card() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let c0 = s.a.deck[0].clone();
    let c1 = s.a.deck[1].clone();
    let c2 = s.a.deck[2].clone();
    let real = s.a.deck[3].clone();
    for iid in [&c0, &c1, &c2] {
        make_cardless(&mut s, iid);
    }
    let hand_before = s.a.hand.len();

    let drew = s.draw_one(PlayerId::A);

    assert!(drew, "a card-bearing sleeve was drawn");
    // Z.8b: 3 empties collected for free + 1 real card = 4 units to hand.
    assert_eq!(s.a.hand.len(), hand_before + 4);
    assert!(s.a.hand.contains(&real), "the card-bearing sleeve was drawn");
    for iid in [&c0, &c1, &c2] {
        assert!(s.a.hand.contains(iid), "cardless sleeve collected for free");
        assert!(!s.a.deck.contains(iid), "cardless sleeve left the deck");
    }
}

#[test]
fn z8b_draw_reports_deckout_when_only_cardless_remain() {
    // A deck of nothing but cardless sleeves: all get collected for free,
    // no card is ever drawn, and draw_one reports false so the caller can
    // resolve the deckout (L.1).
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let all: Vec<InstanceId> = s.a.deck.clone();
    for iid in &all {
        make_cardless(&mut s, iid);
    }
    let hand_before = s.a.hand.len();

    let drew = s.draw_one(PlayerId::A);

    assert!(!drew, "no card-bearing sleeve to draw → false (caller deckouts)");
    assert!(s.a.deck.is_empty(), "every cardless sleeve was collected");
    assert_eq!(
        s.a.hand.len(),
        hand_before + all.len(),
        "all collected empties landed in hand"
    );
}

#[test]
fn z8b_draw_of_a_normal_top_is_an_ordinary_single_draw() {
    // No cardless sleeves: draw_one is exactly the old behaviour.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let top = s.a.deck[0].clone();
    let hand_before = s.a.hand.len();

    let drew = s.draw_one(PlayerId::A);

    assert!(drew);
    assert_eq!(s.a.hand.len(), hand_before + 1);
    assert!(s.a.hand.contains(&top));
}

#[test]
fn z8f_cardless_sleeve_is_transparent_for_top_of_deck_visibility() {
    // A cardless sleeve on top of the deck is see-through (V.8): the
    // reveal walk looks past it to the symbols of the card beneath.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let top = s.a.deck[0].clone();
    let below = s.a.deck[1].clone();
    s.card_pool.get_mut(&below).unwrap().card_mut().symbols = vec!["⊨".to_string()];
    make_cardless(&mut s, &top);

    assert_eq!(
        s.effective_top_of_deck_symbols(PlayerId::A),
        vec!["⊨".to_string()],
        "Z.8f: see through the cardless sleeve to the card below"
    );
}

// ---- Z.8c: cardless sleeve as a generic payment body ----

#[test]
fn z8c_cardless_sleeve_pays_an_attach_cost() {
    // Window Cleaner's case: a cardless sleeve attached to a board card
    // pays an ATTACHED-source cost (no identity gate on attach).
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let host = s.a.hand[1].clone();
    let sleeve = s.a.hand[2].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&sleeve, PlayerId::A, Zone::Hand);
    s.add_attached(&host, &sleeve);
    make_cardless(&mut s, &sleeve);
    set_cost(&mut s, &cast, vec![cost(CostSource::Attached, 1)]);

    let res = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices { attached_payment_ids: vec![sleeve.clone()], ..PlayChoices::default() },
        None,
    );
    assert!(res.is_ok(), "cardless sleeve pays an attach cost: {res:?}");
}

#[test]
fn z8c_cardless_pays_a_hand_cost_for_a_wildcard_cast() {
    // Colorless / no-symbol cast is a P.7a wildcard — a cardless body pays.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let sleeve = s.a.hand[1].clone();
    make_cardless(&mut s, &sleeve);
    set_cost(&mut s, &cast, vec![hand_cost(1)]);

    let res = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices { hand_payment_ids: vec![sleeve.clone()], ..PlayChoices::default() },
        None,
    );
    assert!(res.is_ok(), "cardless pays a wildcard hand cost: {res:?}");
}

#[test]
fn z8c_cardless_cannot_anchor_identity_for_a_hand_cost() {
    // Identity-bearing cast, 1-HAND cost paid only by a cardless sleeve:
    // it fills the slot but carries no identity, so P.7a is unsatisfied.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let sleeve = s.a.hand[1].clone();
    make_cardless(&mut s, &sleeve);
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["green".to_string()];
    set_cost(&mut s, &cast, vec![hand_cost(1)]);

    let res = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices { hand_payment_ids: vec![sleeve.clone()], ..PlayChoices::default() },
        None,
    );
    assert!(res.is_err(), "cardless can't anchor identity for a hand cost: {res:?}");
}

#[test]
fn z8c_cardless_body_plus_real_anchor_pays_an_identity_hand_cost() {
    // 2-HAND identity cost: one real green anchor + one cardless body is OK.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let anchor = s.a.hand[1].clone();
    let sleeve = s.a.hand[2].clone();
    make_cardless(&mut s, &sleeve);
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["green".to_string()];
    s.card_pool.get_mut(&anchor).unwrap().card_mut().colors = vec!["green".to_string()];
    set_cost(&mut s, &cast, vec![hand_cost(2)]);

    let res = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices { hand_payment_ids: vec![anchor.clone(), sleeve.clone()], ..PlayChoices::default() },
        None,
    );
    assert!(res.is_ok(), "real anchor + cardless body pays identity hand cost: {res:?}");
}

#[test]
fn z8c_cardless_sleeve_is_not_millable() {
    // A cardless sleeve never counts for MILL (Z.8c): a mill:1 cost skips
    // the cardless sleeves on top and mills the first card-bearing sleeve.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let d0 = s.a.deck[0].clone();
    let d1 = s.a.deck[1].clone();
    let real = s.a.deck[2].clone();
    make_cardless(&mut s, &d0);
    make_cardless(&mut s, &d1);
    set_cost(&mut s, &cast, vec![cost(CostSource::Mill, 1)]);

    let res = s.play_card(PlayerId::A, &cast, PlayChoices::default(), None);
    assert!(res.is_ok(), "{res:?}");
    assert!(s.a.graveyard.contains(&real), "a card-bearing sleeve was milled, not a cardless one");
}
