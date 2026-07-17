//! Death-replacement hook (12.3) — OnWouldDie intercepts a lethal
//! Board→GY move so a card can substitute an alternative.
//!
//! The engine fires `OnWouldDie` self-only on the dying creature before
//! any move. The handler may call one of two primitives:
//!   - `game.prevent_death(self)` — the creature survives on the BOARD and
//!     the engine clears its accumulated damage (so B.8 doesn't re-kill).
//!     No on_die / no broadcast / no cascade — it didn't die.
//!   - `game.redirect_death(self, zone)` — the creature moves BOARD→zone
//!     instead of GRAVEYARD, quietly: no on_die, no watcher broadcast, no
//!     P.8 attached-cascade.
//!
//! No call → normal death (Board→GY + on_die + broadcast + cascade).
//!
//! Driven end-to-end through the White Elephant, whose printed rule is
//! exactly this replacement (sheds its sleeve to survive once, then is
//! exiled when sleeveless).

use super::*;
use crate::card::CardType;
use crate::game::context::EventContext;
use crate::game::test_helpers::*;
use std::path::Path;

fn white_elephant(lua: &mlua::Lua) -> crate::card::Card {
    crate::card::load_card(lua, Path::new("cards/white-elephant.lua"))
        .expect("white-elephant.lua loads")
        .into_iter()
        .find(|c| c.id == "white-elephant")
        .expect("white-elephant present")
}

fn elephant_on_board(s: &mut GameState, lua: &mlua::Lua) -> InstanceId {
    let host = s.a.hand[0].clone();
    s.card_pool.get_mut(&host).unwrap().content = Some(white_elephant(lua));
    s.a.hand.retain(|i| i != &host);
    s.a.board.push(host.clone());
    host
}

#[test]
fn sleeved_elephant_sheds_and_survives_a_lethal_death() {
    // First lethal event, still sleeved: OnWouldDie sheds its sleeve and
    // prevents the death. It stays on the BOARD, sleeveless, damage
    // cleared, with the shed sleeve attached to it.
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = elephant_on_board(&mut s, &lua);

    // Pile on lethal damage (4/4).
    s.set_damage(&host, 5.0);
    assert_eq!(s.effective_stats(&host).1, 4.0, "toughness is 4");

    let mut ctx = EventContext::lua_only(&lua);
    let died = s
        .resolve_board_deaths(vec![host.clone()], Some(&mut ctx))
        .expect("OnWouldDie resolves locally");

    assert!(died.is_empty(), "the elephant did not die — it was saved");
    assert!(s.a.board.contains(&host), "it stays on the board");
    assert!(!s.a.graveyard.contains(&host), "it did not go to the graveyard");

    let inst = s.card_pool.get(&host).unwrap();
    assert!(inst.sleeveless, "it shed its sleeve to survive");
    assert_eq!(inst.damage, 0.0, "prevent_death cleared its accumulated damage");
    assert_eq!(inst.attached.len(), 1, "the shed sleeve attached to it");
    assert!(s.is_cardless(&inst.attached[0].clone()), "the shed sleeve is cardless");
}

#[test]
fn sleeveless_elephant_is_exiled_instead_of_dying() {
    // Second lethal event, now sleeveless: OnWouldDie redirects to EXILE.
    // Quiet relocation — it leaves the board to exile, not the graveyard.
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = elephant_on_board(&mut s, &lua);
    // Already spent its sleeve (as if it survived once before).
    s.set_sleeveless(&host, true);
    s.set_damage(&host, 5.0);

    let mut ctx = EventContext::lua_only(&lua);
    let died = s
        .resolve_board_deaths(vec![host.clone()], Some(&mut ctx))
        .expect("OnWouldDie resolves locally");

    assert!(died.is_empty(), "a redirect is not a death");
    assert!(!s.a.board.contains(&host), "it left the board");
    assert!(s.a.exile.contains(&host), "it was exiled instead");
    assert!(!s.a.graveyard.contains(&host), "it did not go to the graveyard");
}

#[test]
fn an_ordinary_creature_with_no_ward_dies_normally() {
    // Baseline: a creature with no OnWouldDie handler dies to the graveyard
    // exactly as before — the hook is transparent when nothing replaces.
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    s.card_pool.get_mut(&host).unwrap().card_mut().kind = CardType::Creature;
    s.a.hand.retain(|i| i != &host);
    s.a.board.push(host.clone());

    let mut ctx = EventContext::lua_only(&lua);
    let died = s
        .resolve_board_deaths(vec![host.clone()], Some(&mut ctx))
        .expect("no handler, resolves");

    assert_eq!(died, vec![host.clone()], "it died");
    assert!(s.a.graveyard.contains(&host), "to the graveyard");
    assert!(!s.a.board.contains(&host), "off the board");
}

#[test]
fn elephant_survives_lethal_combat_through_the_real_combat_path() {
    // The wiring proof: the ward fires through confirm_blocks (the actual
    // combat call site), not just a direct resolve_board_deaths call. A
    // 4-power attacker deals lethal damage to the blocking 4/4 elephant; it
    // sheds and survives, and is absent from outcome.deaths.
    let lua = mlua::Lua::new();
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    let atk = s.a.hand[0].clone();
    s.card_pool.get_mut(&atk).unwrap().card_mut().stats.as_mut().unwrap().x = 4.0; // 4 power → lethal to Y=4
    s.a.hand.retain(|i| i != &atk);
    s.a.board.push(atk.clone());
    s.card_pool.get_mut(&atk).unwrap().card_mut().abilities.push("haste".to_string());

    let ele = s.b.hand[0].clone();
    s.card_pool.get_mut(&ele).unwrap().content = Some(white_elephant(&lua));
    s.b.hand.retain(|i| i != &ele);
    s.b.board.push(ele.clone());

    while s.phase != Phase::Combat {
        s.next_phase(None).expect("None ctx never yields");
    }

    s.declare_attacker(&atk, None).unwrap();
    s.confirm_attacks().unwrap();
    s.declare_blocker(&ele, &atk, None).unwrap();

    let mut ctx = EventContext::lua_only(&lua);
    let outcome = s.confirm_blocks(Some(&mut ctx)).expect("combat resolves");

    assert!(!outcome.deaths.contains(&ele), "the elephant did not die in combat");
    assert!(s.b.board.contains(&ele), "it is still on the board");
    let inst = s.card_pool.get(&ele).unwrap();
    assert!(inst.sleeveless, "it shed its sleeve to survive the combat death");
    assert_eq!(inst.damage, 0.0, "its combat damage was cleared on survival");
}
