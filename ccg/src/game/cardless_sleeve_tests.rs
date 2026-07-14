//! Cardless sleeve (Z.8) behaviour tests.
//!
//! A cardless sleeve is a sleeve-unit with no card inside (`content: None`).
//! Z.8b: it does not satisfy "draw a card" — on top of the deck it is
//! collected into hand for free and the draw continues, cascading through
//! consecutive empties until one card-bearing unit is drawn.

use super::*;
use crate::game::test_helpers::*;

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
