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

    // Non-transparent host — proves C.14 doesn't gate the cardless body.
    assert!(!s.is_transparent(&cast), "the cast is a non-transparent host");

    let res = s.play_card(
        PlayerId::A,
        &cast,
        PlayChoices { hand_payment_ids: vec![sleeve.clone()], ..PlayChoices::default() },
        None,
    );
    assert!(res.is_ok(), "cardless pays a wildcard hand cost: {res:?}");
    // Z.8d / C.14: a cardless sleeve has no frame, so it is a
    // non-transparent attachee and attaches to ANY host — here it pitched
    // (P.6) onto the non-transparent cast.
    assert!(
        s.card_pool.get(&cast).unwrap().attached.contains(&sleeve),
        "the cardless sleeve attached to the non-transparent host it paid for",
    );
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
fn z8c_cardless_sleeve_is_milled_uncounted_not_skipped() {
    // Z.8c MILL: a cardless sleeve CAN be milled — it is skimmed into the
    // graveyard along the way — it just doesn't COUNT toward the cost. So
    // a mill:1 cost with two cardless sleeves on top of the deck is still
    // castable: both empties go uncounted into the graveyard and the mill
    // is paid by the first card-bearing sleeve underneath.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let d0 = s.a.deck[0].clone();
    let d1 = s.a.deck[1].clone();
    let real = s.a.deck[2].clone();
    make_cardless(&mut s, &d0);
    make_cardless(&mut s, &d1);
    set_cost(&mut s, &cast, vec![cost(CostSource::Mill, 1)]);

    let res = s.play_card(PlayerId::A, &cast, PlayChoices::default(), None);
    assert!(res.is_ok(), "mill cost is castable with cardless on top: {res:?}");

    // The counted mill: the card-bearing sleeve was milled.
    assert!(s.a.graveyard.contains(&real), "the real card paid the mill and is in the graveyard");
    // The empties went UNCOUNTED INTO the graveyard — not skipped, not left
    // on the deck.
    assert!(s.a.graveyard.contains(&d0) && s.a.graveyard.contains(&d1), "cardless sleeves milled into the graveyard");
    assert!(!s.a.deck.contains(&d0) && !s.a.deck.contains(&d1), "cardless sleeves left the deck");
    assert!(!s.a.deck.contains(&real), "the milled real card left the deck");
}

// ---- 8.2: AI affordability agrees with the resolver on cardless ----

#[test]
fn z8_can_pay_mill_counts_real_cards_not_cardless() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    set_cost(&mut s, &cast, vec![cost(CostSource::Mill, 1)]);
    let a_card = s.card_pool.get(&cast).unwrap().card().clone();

    // Whole deck cardless → 0 millable cards → mill:1 unaffordable.
    let deck: Vec<InstanceId> = s.a.deck.clone();
    for iid in &deck {
        make_cardless(&mut s, iid);
    }
    assert!(
        !crate::sim::ai::can_pay_instant_cost(&s, PlayerId::A, &cast),
        "mill:1 unaffordable when the deck is all cardless"
    );

    // Restore one real card on top → 1 millable → affordable.
    s.card_pool.get_mut(&deck[0]).unwrap().content = Some(a_card);
    assert!(
        crate::sim::ai::can_pay_instant_cost(&s, PlayerId::A, &cast),
        "mill:1 affordable once one real card is millable"
    );
}

#[test]
fn z8_can_pay_wildcard_hand_cost_with_a_cardless_body() {
    // Colorless (wildcard) cast: a cardless sleeve in hand pays the slot.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    let others: Vec<InstanceId> = s.a.hand.iter().filter(|h| **h != cast).cloned().collect();
    for iid in &others {
        make_cardless(&mut s, iid);
    }
    set_cost(&mut s, &cast, vec![hand_cost(1)]);

    assert!(
        crate::sim::ai::can_pay_instant_cost(&s, PlayerId::A, &cast),
        "wildcard hand:1 affordable via a cardless body"
    );
}

#[test]
fn z8_can_pay_identity_hand_needs_a_real_anchor_not_just_cardless() {
    // Identity cast with only cardless in hand: no real anchor, so the AI
    // stays conservative and does NOT offer it — matching the resolver's
    // all-cardless rejection, so no pick/resolve loop.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["green".to_string()];
    let others: Vec<InstanceId> = s.a.hand.iter().filter(|h| **h != cast).cloned().collect();
    for iid in &others {
        make_cardless(&mut s, iid);
    }
    set_cost(&mut s, &cast, vec![hand_cost(1)]);

    assert!(
        !crate::sim::ai::can_pay_instant_cost(&s, PlayerId::A, &cast),
        "identity hand:1 not affordable with only cardless (no anchor)"
    );
}

#[test]
fn z8_can_pay_identity_hand2_with_one_real_anchor_plus_a_cardless_body() {
    // Identity cast, hand:2 — one real identity-matching card anchors
    // (P.7a) and a cardless sleeve fills the second, non-anchor slot
    // (Z.8c). The engine already accepts this bundle; the sim must now
    // fund it (previously it stayed conservative and refused).
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let cast = s.a.hand[0].clone();
    s.card_pool.get_mut(&cast).unwrap().card_mut().colors = vec!["green".to_string()];
    // One real green anchor.
    let anchor = s.a.hand[1].clone();
    s.card_pool.get_mut(&anchor).unwrap().card_mut().colors = vec!["green".to_string()];
    // The rest are cardless bodies.
    let bodies: Vec<InstanceId> = s
        .a
        .hand
        .iter()
        .filter(|h| **h != cast && **h != anchor)
        .cloned()
        .collect();
    for iid in &bodies {
        make_cardless(&mut s, iid);
    }
    set_cost(&mut s, &cast, vec![hand_cost(2)]);

    assert!(
        crate::sim::ai::can_pay_instant_cost(&s, PlayerId::A, &cast),
        "identity hand:2 affordable via one real anchor + a cardless body"
    );
}

// ---- 9.1: search library for cardless sleeves ----

#[test]
fn attach_cardless_from_deck_finds_scattered_cardless_and_attaches_n() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    // Three cardless sleeves scattered through the deck.
    let c1 = s.a.deck[2].clone();
    let c2 = s.a.deck[10].clone();
    let c3 = s.a.deck[20].clone();
    for c in [&c1, &c2, &c3] {
        make_cardless(&mut s, c);
    }

    let n = s.attach_cardless_from_deck(&host, PlayerId::A, 2);

    assert_eq!(n, 2, "attaches up to 2 cardless sleeves");
    let attached = &s.card_pool.get(&host).unwrap().attached;
    assert!(attached.contains(&c1) && attached.contains(&c2), "first two (deck order) attached");
    assert!(!attached.contains(&c3), "the third is left in the deck");
    assert!(!s.a.deck.contains(&c1) && !s.a.deck.contains(&c2), "attached ones left the deck");
    assert!(s.a.deck.contains(&c3), "the third stays in the deck");
    // Only cardless sleeves are taken — no real card is attached.
    assert!(attached.iter().all(|iid| s.is_cardless(iid)), "only cardless sleeves attached");
}

#[test]
fn attach_cardless_from_hand_takes_empty_sleeves_out_of_hand() {
    // Angry Glassblower's on-attack attaches an empty sleeve that comes
    // out of HAND (not the deck like Window Cleaner).
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    // Two cardless sleeves in hand, one real card between them.
    let h0 = s.a.hand[0].clone();
    let real = s.a.hand[1].clone();
    let h1 = s.a.hand[2].clone();
    make_cardless(&mut s, &h0);
    make_cardless(&mut s, &h1);

    let n = s.attach_cardless_from_hand(&host, PlayerId::A, 1);

    assert_eq!(n, 1, "attaches one empty sleeve from hand");
    let attached = &s.card_pool.get(&host).unwrap().attached;
    assert!(attached.contains(&h0), "the first hand cardless (hand order) attached");
    assert!(attached.iter().all(|iid| s.is_cardless(iid)), "only a cardless sleeve was attached");
    assert!(!s.a.hand.contains(&h0), "the attached sleeve left the hand");
    assert!(s.a.hand.contains(&h1), "the second sleeve stays in hand");
    assert!(s.a.hand.contains(&real), "the real card is untouched");
}

#[test]
fn z8_a_card_sheds_its_own_sleeve_and_becomes_sleeveless() {
    // Sleeveless card (Z.8) — the mirror of the cardless sleeve. A card
    // pops out of its own sleeve: the card stays put (same id, content
    // intact) but is now sleeveless, and its vacated sleeve attaches to it
    // as a cardless sleeve (Z.6). This is the primitive a "survive by
    // shedding your sleeve" death replacement drives.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let card = s.a.hand[0].clone();
    let _ = s.move_card(&card, PlayerId::A, Zone::Hand, Zone::Board);

    assert!(!s.card_pool.get(&card).unwrap().sleeveless, "starts sleeved");

    let shed = s.shed_own_sleeve(&card);
    assert!(shed, "a sleeved card can shed its sleeve");

    let inst = s.card_pool.get(&card).unwrap();
    assert!(inst.sleeveless, "the card is now sleeveless");
    assert!(inst.content.is_some(), "the card itself is intact — only the sleeve left");
    assert_eq!(inst.attached.len(), 1, "the vacated sleeve attaches to the card");
    let sleeve = inst.attached[0].clone();
    assert!(s.is_cardless(&sleeve), "the shed sleeve is cardless");

    // Second shed is a no-op: already sleeveless, no sleeve left to shed.
    let again = s.shed_own_sleeve(&card);
    assert!(!again, "a sleeveless card has no sleeve to shed");
    assert_eq!(
        s.card_pool.get(&card).unwrap().attached.len(),
        1,
        "no second sleeve minted on the no-op shed"
    );
}

#[test]
fn z8_shed_own_sleeve_round_trips_through_journal() {
    // Shedding is three journaled effects — the sleeveless flip, the minted
    // cardless sleeve, and the self-attach. All three must invert together
    // or full-game rollback diverges once a sleeveless card exists.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let card = s.a.hand[0].clone();
    let _ = s.move_card(&card, PlayerId::A, Zone::Hand, Zone::Board);

    let pool_len = s.card_pool.len();
    let before = format!("{s:?}");
    s.journal = Some(crate::game::Journal::new());
    assert!(s.shed_own_sleeve(&card), "shed happened");
    assert!(s.card_pool.get(&card).unwrap().sleeveless);
    assert_eq!(s.card_pool.len(), pool_len + 1, "shed sleeve minted into the pool");

    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    assert_eq!(before, format!("{s:?}"), "shed rolls back: flag, mint, and attach");
}

#[test]
fn z8_a_cardless_sleeve_cannot_become_sleeveless_no_null_unit() {
    // The fourth quadrant — content:None AND sleeveless:true — is the null
    // object: neither a card nor a sleeve. It must be unrepresentable.
    // Making a cardless sleeve sleeveless is refused with a sacred error,
    // and the unit stays a plain cardless sleeve.
    crate::error::reset();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    make_cardless(&mut s, &iid);
    assert!(s.is_cardless(&iid), "it is a cardless sleeve");

    s.set_sleeveless(&iid, true);

    let inst = s.card_pool.get(&iid).unwrap();
    assert!(
        !inst.sleeveless,
        "a cardless sleeve was refused sleeveless — the null unit was not constructed"
    );
    assert!(inst.content.is_none(), "it is still a plain cardless sleeve");
    assert!(
        !crate::error::drain().is_empty(),
        "a sacred error surfaced the refusal"
    );
}
