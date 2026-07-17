//! Window Cleaner (slice 9.3) — behaviour tests.
//!
//! An azure human whose two triggers exercise the cardless-sleeve
//! primitives from 9.1 (`attach_cardless_from_deck`) and 9.2
//! (`OnTapped`):
//!   - ETB: search the deck for 2 cardless sleeves and attach them.
//!   - On becoming tapped: *may* move an attached cardless sleeve to
//!     the graveyard and draw a card.

use super::*;
use crate::card::EventName;
use crate::choice::{ScriptedAnswer, ScriptedOracle};
use crate::game::lua_api::fire_self_only;
use crate::game::test_helpers::*;
use std::path::Path;

/// Load the real Window Cleaner card, compiled in `lua` so its handlers
/// fire under the same interpreter.
fn window_cleaner(lua: &mlua::Lua) -> crate::card::Card {
    crate::card::load_card(lua, Path::new("cards/window-cleaner.lua"))
        .expect("window-cleaner.lua loads")
        .into_iter()
        .find(|c| c.id == "window-cleaner")
        .expect("window-cleaner present")
}

/// Put a Window Cleaner instance on player A's board and return its iid.
fn window_cleaner_on_board(s: &mut GameState, lua: &mlua::Lua) -> InstanceId {
    let host = s.a.hand[0].clone();
    s.card_pool.get_mut(&host).unwrap().content = Some(window_cleaner(lua));
    s.a.hand.retain(|i| i != &host);
    s.a.board.push(host.clone());
    host
}

#[test]
fn window_cleaner_etb_attaches_two_cardless_sleeves() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = window_cleaner_on_board(&mut s, &lua);

    // Two cardless sleeves scattered in the deck for the ETB to find.
    let d0 = s.a.deck[3].clone();
    let d1 = s.a.deck[9].clone();
    for c in [&d0, &d1] {
        s.card_pool.get_mut(c).unwrap().content = None;
    }

    let mut oracle = ScriptedOracle::new(vec![]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnEnterBoard, &host)
        .expect("ETB answers locally");

    let attached = &s.card_pool.get(&host).unwrap().attached;
    assert_eq!(attached.len(), 2, "ETB attaches exactly 2 cardless sleeves");
    assert!(
        attached.iter().all(|iid| s.is_cardless(iid)),
        "only cardless sleeves were attached",
    );
    assert!(
        !s.a.deck.contains(&d0) && !s.a.deck.contains(&d1),
        "the attached cardless sleeves left the deck",
    );
}

#[test]
fn window_cleaner_on_tap_moves_a_cardless_to_gy_and_draws() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = window_cleaner_on_board(&mut s, &lua);

    // Give it two attached cardless sleeves to spend.
    let c0 = s.a.deck[3].clone();
    let c1 = s.a.deck[9].clone();
    for c in [&c0, &c1] {
        s.card_pool.get_mut(c).unwrap().content = None;
    }
    let attached = s.attach_cardless_from_deck(&host, PlayerId::A, 2);
    assert_eq!(attached, 2, "precondition: two cardless sleeves attached");

    let gy_before = s.a.graveyard.len();
    let hand_before = s.a.hand.len();

    // The trigger is a "may" — the oracle confirms.
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Confirm(true)]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnTapped, &host)
        .expect("on_tapped answers locally");

    assert_eq!(
        s.card_pool.get(&host).unwrap().attached.len(),
        1,
        "one attached cardless sleeve was spent",
    );
    assert_eq!(
        s.a.graveyard.len(),
        gy_before + 1,
        "the spent cardless sleeve went to the graveyard",
    );
    assert_eq!(s.a.hand.len(), hand_before + 1, "Window Cleaner drew a card");
}

#[test]
fn window_cleaner_on_tap_declined_does_nothing() {
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = window_cleaner_on_board(&mut s, &lua);
    let c0 = s.a.deck[3].clone();
    let c1 = s.a.deck[9].clone();
    for c in [&c0, &c1] {
        s.card_pool.get_mut(c).unwrap().content = None;
    }
    s.attach_cardless_from_deck(&host, PlayerId::A, 2);

    let gy_before = s.a.graveyard.len();
    let hand_before = s.a.hand.len();

    // Decline the "may".
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Confirm(false)]);
    fire_self_only(&lua, &mut s, &mut oracle, EventName::OnTapped, &host)
        .expect("on_tapped answers locally");

    assert_eq!(
        s.card_pool.get(&host).unwrap().attached.len(),
        2,
        "declining leaves both cardless sleeves attached",
    );
    assert_eq!(s.a.graveyard.len(), gy_before, "nothing moved to the graveyard");
    assert_eq!(s.a.hand.len(), hand_before, "no card drawn");
}
