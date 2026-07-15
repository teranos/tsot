//! Shatter Expectations (slice 10) — behaviour tests.
//!
//! Three novel pieces, all resolved in on_play:
//!   - composition-derived X (clears + cardless add 1, ordinaries
//!     subtract 1);
//!   - counter-with-alternative-cost (an opponent-side may-pay via
//!     confirm_for);
//!   - multi-zone exile (X from HAND, GY, BOARD, DECK).

use super::*;
use crate::card::{CardRegistry, EventName};
use crate::choice::{ScriptedAnswer, ScriptedOracle};
use crate::game::lua_api::fire_self_only;
use crate::game::test_helpers::*;
use std::path::Path;

fn shatter(registry: &CardRegistry) -> crate::card::Card {
    registry
        .cards()
        .iter()
        .find(|c| c.id == "shatter-expectations")
        .expect("shatter-expectations present")
        .clone()
}

/// Move `iid` into `player`'s graveyard from wherever it is.
fn to_gy(s: &mut GameState, player: PlayerId, iid: &InstanceId) {
    let from = if s.player(player).hand.contains(iid) {
        Zone::Hand
    } else {
        Zone::Deck
    };
    let _ = s.move_card(iid, player, from, Zone::Graveyard);
}

fn make_clear(s: &mut GameState, iid: &InstanceId) {
    s.card_pool.get_mut(iid).unwrap().card_mut().frame = Some("transparent".to_string());
}
fn make_empty(s: &mut GameState, iid: &InstanceId) {
    s.card_pool.get_mut(iid).unwrap().content = None;
}

#[test]
fn shatter_opponent_pays_the_ransom_and_the_spell_is_not_countered() {
    let registry = CardRegistry::load(Path::new("cards")).unwrap();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));

    // A's Shatter.
    let shatter_iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&shatter_iid).unwrap().content = Some(shatter(&registry));

    // A's graveyard: one clear + one cardless sleeve (X += 2) and one
    // ordinary card the caster chooses to LEAVE (so the exile loop
    // actually reaches its stop-prompt). X = 2.
    let clear = s.a.deck[0].clone();
    let empty = s.a.deck[1].clone();
    let leave = s.a.deck[2].clone();
    to_gy(&mut s, PlayerId::A, &clear);
    to_gy(&mut s, PlayerId::A, &empty);
    to_gy(&mut s, PlayerId::A, &leave);
    make_clear(&mut s, &clear);
    make_empty(&mut s, &empty);

    // B can pay X=2 from every zone: seed 2 into board + graveyard
    // (hand already has 5, deck has plenty).
    for i in 0..2 {
        let b = s.b.deck[i].clone();
        let _ = s.move_card(&b, PlayerId::B, Zone::Deck, Zone::Board);
    }
    for i in 0..2 {
        let b = s.b.deck[i].clone();
        let _ = s.move_card(&b, PlayerId::B, Zone::Deck, Zone::Graveyard);
    }
    let (bh, bg, bb, bd) = (
        s.b.hand.len(),
        s.b.graveyard.len(),
        s.b.board.len(),
        s.b.deck.len(),
    );

    // Caster exiles clear, then empty, then stops; opponent pays.
    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(Some(clear.clone())),
        ScriptedAnswer::Card(Some(empty.clone())),
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(true),
    ]);
    fire_self_only(registry.lua(), &mut s, &mut oracle, EventName::OnPlay, &shatter_iid)
        .expect("on_play answers locally");

    // Caster exiled its two composition cards; the left card stayed.
    assert!(s.a.exile.contains(&clear) && s.a.exile.contains(&empty), "caster exiled clear + empty");
    assert!(s.a.graveyard.contains(&leave), "the left-behind ordinary stayed in the graveyard");

    // X=2 ransom paid across all four of B's zones.
    assert_eq!(s.b.hand.len(), bh - 2, "B exiled 2 from hand");
    assert_eq!(s.b.graveyard.len(), bg - 2, "B exiled 2 from graveyard");
    assert_eq!(s.b.board.len(), bb - 2, "B exiled 2 from board");
    assert_eq!(s.b.deck.len(), bd - 2, "B exiled 2 from deck");
    assert_eq!(s.b.exile.len(), 8, "the ransom is 4X = 8 cards");

    // Paid → not countered.
    assert_eq!(s.action_counts.get("counter_top").map(|v| v[0] + v[1]).unwrap_or(0), 0, "no counter");
}

#[test]
fn shatter_opponent_declines_and_the_target_spell_is_countered() {
    let registry = CardRegistry::load(Path::new("cards")).unwrap();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));

    // A puts a creature cast on the chain.
    let a_creature = s.a.hand[0].clone();
    let a_cast = crate::game::StackItem::PlayedCard {
        card: a_creature.clone(),
        controller: PlayerId::A,
        choices: PlayChoices::default(),
    };
    s.open_response_window(a_cast).unwrap();

    // B responds with Shatter. B's graveyard: two clears → X = 2.
    let shatter_iid = s.b.hand[0].clone();
    s.card_pool.get_mut(&shatter_iid).unwrap().content = Some(shatter(&registry));
    let c1 = s.b.deck[0].clone();
    let c2 = s.b.deck[1].clone();
    let b_leave = s.b.deck[2].clone();
    to_gy(&mut s, PlayerId::B, &c1);
    to_gy(&mut s, PlayerId::B, &c2);
    to_gy(&mut s, PlayerId::B, &b_leave);
    make_clear(&mut s, &c1);
    make_clear(&mut s, &c2);

    // Make A able to pay (so the may-pay is actually asked), then decline.
    for i in 0..2 {
        let a = s.a.deck[i].clone();
        let _ = s.move_card(&a, PlayerId::A, Zone::Deck, Zone::Board);
    }
    for i in 0..2 {
        let a = s.a.deck[i].clone();
        let _ = s.move_card(&a, PlayerId::A, Zone::Deck, Zone::Graveyard);
    }

    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(Some(c1.clone())),
        ScriptedAnswer::Card(Some(c2.clone())),
        ScriptedAnswer::Card(None),
        ScriptedAnswer::Confirm(false), // A declines the ransom
    ]);
    fire_self_only(registry.lua(), &mut s, &mut oracle, EventName::OnPlay, &shatter_iid)
        .expect("on_play answers locally");

    // A's cast was countered — removed from the chain — and counter_top
    // credited to B.
    let a_still_on_chain = s
        .priority
        .as_ref()
        .map(|p| {
            p.chain.iter().any(|item| {
                let crate::game::StackItem::PlayedCard { card, .. } = item;
                card == &a_creature
            })
        })
        .unwrap_or(false);
    assert!(!a_still_on_chain, "A's spell was countered off the chain");
    assert_eq!(s.action_counts.get("counter_top").map(|v| v[1]).unwrap_or(0), 1, "counter_top by B");
}

#[test]
fn shatter_nonpositive_x_whiffs() {
    let registry = CardRegistry::load(Path::new("cards")).unwrap();
    let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));

    let shatter_iid = s.a.hand[0].clone();
    s.card_pool.get_mut(&shatter_iid).unwrap().content = Some(shatter(&registry));

    // A's graveyard: a single ordinary card. Exiling it gives X = -1.
    let ordinary = s.a.deck[0].clone();
    to_gy(&mut s, PlayerId::A, &ordinary);
    let b_before = (s.b.hand.len(), s.b.deck.len());

    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Card(Some(ordinary.clone())),
        ScriptedAnswer::Card(None),
    ]);
    fire_self_only(registry.lua(), &mut s, &mut oracle, EventName::OnPlay, &shatter_iid)
        .expect("on_play answers locally");

    // The ordinary was still exiled (it was part of the composition), but
    // X <= 0 means no counter and no ransom demanded.
    assert!(s.a.exile.contains(&ordinary), "the exiled ordinary left the graveyard");
    assert_eq!(s.action_counts.get("counter_top").map(|v| v[0] + v[1]).unwrap_or(0), 0, "no counter at X<=0");
    assert_eq!((s.b.hand.len(), s.b.deck.len()), b_before, "opponent untouched at X<=0");
    assert!(s.b.exile.is_empty(), "no ransom exiled at X<=0");
}
