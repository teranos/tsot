//! Angry Glassblower — behaviour tests.
//!
//! The hand-side cardless card: where Window Cleaner pulls clear sleeves
//! out of the DECK, the Glassblower attaches an empty sleeve out of HAND
//! on attack, and shatters one back off (exile) for a rummage when a
//! swing connects.
//!   - OnAttack: *may* attach an empty sleeve from hand + draw.
//!   - OnDealtDamageToPlayer: *may* exile an attached card; if it was an
//!     empty sleeve, draw a card and discard a card.

use super::*;
use crate::card::EventName;
use crate::choice::{ScriptedAnswer, ScriptedOracle};
use crate::game::lua_api::fire_self_only;
use crate::game::test_helpers::*;
use std::path::Path;

fn glassblower(lua: &mlua::Lua) -> crate::card::Card {
    crate::card::load_card(lua, Path::new("cards/angry-glassblower.lua"))
        .expect("angry-glassblower.lua loads")
        .into_iter()
        .find(|c| c.id == "angry-glassblower")
        .expect("angry-glassblower present")
}

fn glassblower_on_board(s: &mut GameState, lua: &mlua::Lua) -> InstanceId {
    let host = s.a.hand[0].clone();
    s.card_pool.get_mut(&host).unwrap().content = Some(glassblower(lua));
    s.a.hand.retain(|i| i != &host);
    s.a.board.push(host.clone());
    host
}

#[test]
fn glassblower_on_attack_attaches_an_empty_sleeve_from_hand_and_draws() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = glassblower_on_board(&mut s, &lua);

    // An empty sleeve sits in hand for him to blow onto himself.
    let sleeve = s.a.hand[0].clone();
    s.card_pool.get_mut(&sleeve).unwrap().content = None;

    let deck_before = s.a.deck.len();

    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Confirm(true)]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnAttack, &host)
        .expect("on_attack answers locally");

    let attached = &s.card_pool.get(&host).unwrap().attached;
    assert_eq!(attached.len(), 1, "one empty sleeve attached from hand");
    assert!(attached.contains(&sleeve), "the hand sleeve is the one attached");
    assert!(s.is_cardless(&sleeve), "the attached card is the empty sleeve");
    assert!(!s.a.hand.contains(&sleeve), "the sleeve left the hand");
    assert_eq!(s.a.deck.len(), deck_before - 1, "he also drew a card");
}

#[test]
fn glassblower_on_attack_without_an_empty_sleeve_does_nothing() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = glassblower_on_board(&mut s, &lua);
    let deck_before = s.a.deck.len();

    // Even a "yes" can't attach what isn't there — no empty sleeve in hand.
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Confirm(true)]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnAttack, &host)
        .expect("on_attack answers locally");

    assert!(s.card_pool.get(&host).unwrap().attached.is_empty(), "nothing attached");
    assert_eq!(s.a.deck.len(), deck_before, "no draw without a sleeve to attach");
}

#[test]
fn glassblower_on_damage_exiles_an_empty_sleeve_then_draws_and_discards() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = glassblower_on_board(&mut s, &lua);

    // Attach an empty sleeve straight onto him (as if a prior swing did).
    let sleeve = s.a.hand[0].clone();
    s.card_pool.get_mut(&sleeve).unwrap().content = None;
    s.attach_cardless_from_hand(&host, PlayerId::A, 1);
    assert_eq!(s.card_pool.get(&host).unwrap().attached, vec![sleeve.clone()]);

    let deck_before = s.a.deck.len();
    let gy_before = s.a.graveyard.len();

    // Confirm the may, then pick the sleeve to exile.
    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Confirm(true),
        ScriptedAnswer::Card(Some(sleeve.clone())),
    ]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnDealtDamageToPlayer, &host)
        .expect("on_dealt_damage answers locally");

    assert!(s.card_pool.get(&host).unwrap().attached.is_empty(), "the sleeve was exiled off him");
    assert!(s.a.exile.contains(&sleeve), "the empty sleeve went to exile");
    // Shattering an empty sleeve draws then discards: deck -1, graveyard +1.
    assert_eq!(s.a.deck.len(), deck_before - 1, "drew a card off the shatter");
    assert_eq!(s.a.graveyard.len(), gy_before + 1, "discarded a card after drawing");
}

#[test]
fn glassblower_on_damage_exiling_a_real_card_gives_no_cantrip() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = glassblower_on_board(&mut s, &lua);

    // Attach a REAL card (not an empty sleeve) onto him.
    let real = s.a.hand[0].clone();
    s.a.hand.retain(|i| i != &real);
    s.add_attached(&host, &real);
    assert!(!s.is_cardless(&real), "precondition: attached card is a real card");

    let deck_before = s.a.deck.len();
    let gy_before = s.a.graveyard.len();

    let mut oracle = ScriptedOracle::new(vec![
        ScriptedAnswer::Confirm(true),
        ScriptedAnswer::Card(Some(real.clone())),
    ]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnDealtDamageToPlayer, &host)
        .expect("on_dealt_damage answers locally");

    assert!(s.card_pool.get(&host).unwrap().attached.is_empty(), "the real card was exiled");
    assert!(s.a.exile.contains(&real), "the real card went to exile");
    // No empty sleeve → no rummage.
    assert_eq!(s.a.deck.len(), deck_before, "no draw when the exiled card wasn't empty");
    assert_eq!(s.a.graveyard.len(), gy_before, "no discard when the exiled card wasn't empty");
}
