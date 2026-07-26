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

/// End-to-end: Rebuke is cast from HAND via `play_card`, pays its
/// self-exile cost (P.5), fires OnPlay through the real cast machinery,
/// mills the target, and lands in EXILE. Same shape a real game uses.
#[test]
fn rebuke_e2e_cast_from_hand_pays_self_exile_and_mills_target() {
    let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let rebuke_card = registry
        .cards()
        .iter()
        .find(|c| c.id == "rebuke")
        .unwrap()
        .clone();

    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Rebuke in A's hand — real cast source zone.
    let rebuke_iid = s.a.hand[0].clone();
    {
        let inst = s.card_pool.get_mut(&rebuke_iid).unwrap();
        inst.card_mut().handlers = rebuke_card.handlers.clone();
        inst.card_mut().id = rebuke_card.id.clone();
        inst.card_mut().kind = rebuke_card.kind;
        inst.card_mut().timing = rebuke_card.timing;
        inst.card_mut().colors = rebuke_card.colors.clone();
        inst.card_mut().cost = rebuke_card.cost.clone();
    }

    // Prime the graveyard-add counter with 3 real graveyard-adds — one
    // A hand card + two B hand cards, so the count spans both players
    // (locks the "global across both graveyards" semantic).
    let d_a = s.a.hand[1].clone();
    let d_b1 = s.b.hand[0].clone();
    let d_b2 = s.b.hand[1].clone();
    let _ = s.move_card_or_emit(&d_a, PlayerId::A, Zone::Hand, Zone::Graveyard, "prime", None);
    let _ = s.move_card_or_emit(&d_b1, PlayerId::B, Zone::Hand, Zone::Graveyard, "prime", None);
    let _ = s.move_card_or_emit(&d_b2, PlayerId::B, Zone::Hand, Zone::Graveyard, "prime", None);
    assert_eq!(s.graveyard_added_this_turn, 3, "primed 3 graveyard-adds");

    let b_deck_before = s.b.deck.len();
    let b_gy_before = s.b.graveyard.len();

    // Cast via real play_card. Rebuke's cost is self-exile only, no
    // hand/mill/graveyard payments — PlayChoices::default() suffices.
    let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Player(Some(PlayerId::B))]);
    let mut ctx = crate::game::EventContext::new(registry.lua(), &mut oracle);
    s.play_card(
        PlayerId::A,
        &rebuke_iid,
        crate::game::PlayChoices::default(),
        Some(&mut ctx),
    )
    .expect("rebuke cast succeeds");

    // P.5: Rebuke self-exiles on resolution — must be in A's EXILE, not
    // HAND or GRAVEYARD.
    assert!(!s.a.hand.contains(&rebuke_iid), "Rebuke left HAND");
    assert!(!s.a.graveyard.contains(&rebuke_iid), "Rebuke did NOT go to GY");
    assert!(s.a.exile.contains(&rebuke_iid), "Rebuke self-exiled to EXILE");

    // Effect: 2 × 3 = 6 mill on B.
    assert_eq!(
        s.b.deck.len(),
        b_deck_before - 6,
        "milled 2 × 3 from B's deck"
    );
    assert_eq!(
        s.b.graveyard.len(),
        b_gy_before + 6,
        "the 6 milled cards landed in B's graveyard"
    );
    // The mill itself added 6 to the counter (bumped on every → GY).
    assert_eq!(
        s.graveyard_added_this_turn,
        3 + 6,
        "counter reflects prime + mill adds"
    );
}
