//! Rebuke — red self-exile spell, mills 2× per graveyard-add this turn.
//!
//! Exercises:
//!   - `graveyard_added_this_turn` counter bumps on every move → GRAVEYARD.
//!   - Counter resets on End → Untap.
//!   - `game.graveyard_added_this_turn()` Lua binding reads the counter.
//!   - Rebuke's on_play scales the mill by `2 * n`.

use super::*;
use crate::card::EventName;
use crate::choice::{ScriptedAnswer, ScriptedOracle};
use crate::game::lua_api::fire_self_only;
use crate::game::test_helpers::*;

/// The counter bumps once per move into a graveyard.
#[test]
fn graveyard_add_bumps_the_per_turn_counter() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    assert_eq!(s.graveyard_added_this_turn, 0);

    let iid1 = s.a.hand[0].clone();
    let iid2 = s.a.hand[1].clone();
    let _ = s.move_card_or_emit(
        &iid1,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "test-bump",
        None,
    );
    assert_eq!(s.graveyard_added_this_turn, 1);
    let _ = s.move_card_or_emit(
        &iid2,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "test-bump",
        None,
    );
    assert_eq!(s.graveyard_added_this_turn, 2);
}

/// Non-graveyard destinations do NOT bump.
#[test]
fn move_to_non_graveyard_does_not_bump_the_counter() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    let _ = s.move_card_or_emit(
        &iid,
        PlayerId::A,
        Zone::Hand,
        Zone::Exile,
        "test-no-bump",
        None,
    );
    assert_eq!(s.graveyard_added_this_turn, 0);
}

/// Rebuke reads the counter and mills 2 × N on target.
#[test]
fn rebuke_mills_two_per_graveyard_add_this_turn() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let rebuke = registry
        .cards()
        .iter()
        .find(|c| c.id == "rebuke")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Rebuke on A's board (skip play_card setup — this test exercises the
    // on_play handler in isolation, not the cast machinery).
    let rebuke_iid = s.a.hand[0].clone();
    s.a.hand.retain(|x| x != &rebuke_iid);
    s.a.board.push(rebuke_iid.clone());
    {
        let inst = s.card_pool.get_mut(&rebuke_iid).unwrap();
        inst.card_mut().handlers = rebuke.handlers.clone();
        inst.card_mut().id = rebuke.id.clone();
    }

    // Prime the counter: 2 graveyard-adds via the folded path.
    let d1 = s.a.hand[0].clone();
    let d2 = s.a.hand[1].clone();
    let _ = s.move_card_or_emit(&d1, PlayerId::A, Zone::Hand, Zone::Graveyard, "prime", None);
    let _ = s.move_card_or_emit(&d2, PlayerId::A, Zone::Hand, Zone::Graveyard, "prime", None);
    assert_eq!(s.graveyard_added_this_turn, 2);

    let b_deck_before = s.b.deck.len();
    let b_gy_before = s.b.graveyard.len();

    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Player(Some(PlayerId::B))]);
    fire_self_only(
        registry.lua(),
        &mut s,
        &mut oracle,
        EventName::OnPlay,
        &rebuke_iid,
    )
    .expect("rebuke on_play");

    assert_eq!(
        s.b.deck.len(),
        b_deck_before - 4,
        "milled 2 × 2 from B's deck"
    );
    assert_eq!(
        s.b.graveyard.len(),
        b_gy_before + 4,
        "the 4 milled cards landed in B's graveyard"
    );
}

/// End → Untap transition resets the counter to 0.
#[test]
fn end_untap_transition_resets_the_counter() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let iid = s.a.hand[0].clone();
    let _ = s.move_card_or_emit(
        &iid,
        PlayerId::A,
        Zone::Hand,
        Zone::Graveyard,
        "prime",
        None,
    );
    assert_eq!(s.graveyard_added_this_turn, 1);

    // Advance to End, then End → Untap.
    s.set_phase(Phase::End);
    s.next_phase(None).expect("End → next turn's Untap");

    assert_eq!(
        s.graveyard_added_this_turn, 0,
        "End → Untap must reset the counter"
    );
}
