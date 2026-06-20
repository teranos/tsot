//! Save/load round-trip: snapshot mid-game, restore, continue playing,
//! assert byte-identical to a control run that never saved.
//!
//! Demonstrates that `SaveFile` preserves enough state to resume execution.
//! The `rebind_handlers` step re-attaches Lua handlers from a live registry
//! (they were skipped during serialization because mlua::Function isn't
//! serializable).

mod common;

use tsot::card::CardRegistry;
use tsot::choice::ScriptedOracle;
use tsot::game::{EventContext, GameState, Phase};
use tsot::replay::SaveFile;

#[test]
fn save_mid_game_then_load_and_continue_matches_uninterrupted_control() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let deck_a = common::fixed_deck(&registry, 50);
    let deck_b = common::fixed_deck(&registry, 50);

    // Control: run two combat cycles uninterrupted.
    let mut control = GameState::new(deck_a.clone(), deck_b.clone());
    let mut control_oracle = ScriptedOracle::new(vec![]);
    common::one_combat_cycle(&mut control, &mut control_oracle, registry.lua());
    common::one_combat_cycle(&mut control, &mut control_oracle, registry.lua());

    // Test: run one cycle, save, deserialize, restore, run second cycle.
    let mut tested = GameState::new(deck_a, deck_b);
    let mut tested_oracle = ScriptedOracle::new(vec![]);
    common::one_combat_cycle(&mut tested, &mut tested_oracle, registry.lua());

    let save = SaveFile::from_state(&tested, 0);
    let json = save.to_json().unwrap();
    assert!(!json.is_empty());

    let restored = SaveFile::from_json(&json).unwrap();
    let mut resumed = restored.restore(&registry).unwrap();
    let mut resumed_oracle = ScriptedOracle::new(vec![]);
    common::one_combat_cycle(&mut resumed, &mut resumed_oracle, registry.lua());

    // Final states should match (modulo any journal that lives differently
    // between the two paths). Compare core game state fields directly.
    assert_eq!(format!("{:?}", control.a), format!("{:?}", resumed.a));
    assert_eq!(format!("{:?}", control.b), format!("{:?}", resumed.b));
    assert_eq!(control.turn, resumed.turn);
    assert_eq!(control.phase, resumed.phase);
    assert_eq!(control.active_player, resumed.active_player);
    assert_eq!(control.winner, resumed.winner);
    assert_eq!(
        format!("{:?}", control.card_pool),
        format!("{:?}", resumed.card_pool),
        "card_pool should match after save/load/continue"
    );
}

#[test]
fn save_load_round_trip_preserves_lua_handler_calls() {
    // Save a state, restore via rebind_handlers, fire a real handler from a
    // restored card and verify it executes (no nil-Function error).
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let bee = registry
        .cards()
        .iter()
        .find(|c| c.id == "mortal-bee")
        .unwrap()
        .clone();

    let deck_a = common::fixed_deck(&registry, 50);
    let deck_b = common::fixed_deck(&registry, 50);

    let mut state = GameState::new(deck_a, deck_b);
    let atk = state.a.hand[0].clone();
    {
        let inst = state.card_pool.get_mut(&atk).unwrap();
        inst.card = bee.clone();
    }
    state.a.hand.remove(0);
    state.a.board.push(atk.clone());

    // Save + restore.
    let json = SaveFile::from_state(&state, 0).to_json().unwrap();
    let restored = SaveFile::from_json(&json).unwrap();
    let mut resumed = restored.restore(&registry).unwrap();

    // After rebind, the bee's handlers should be present.
    let bee_inst = resumed.card_pool.get(&atk).unwrap();
    assert!(
        !bee_inst.card.handlers.is_empty(),
        "handlers should be rebound after restore"
    );

    // Bring resumed into combat and try declaring the bee as attacker.
    common::advance_to(&mut resumed, Phase::Combat);
    // Mark unsick so it can attack.
    if let Some(inst) = resumed.card_pool.get_mut(&atk) {
        inst.summoning_sick = false;
    }
    let mut oracle = ScriptedOracle::new(vec![]);
    let result = resumed.declare_attacker(
        &atk,
        Some(&mut EventContext::new(registry.lua(), &mut oracle)),
    );
    assert!(result.is_ok(), "declare_attacker should succeed: {result:?}");
}
