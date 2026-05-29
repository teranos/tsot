//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Each fire-site builds a per-call `game` userdata via the `build_game_table!`
//! macro inside `Lua::scope`, so closures can borrow `&mut GameState` for the
//! duration of the handler call only.

use super::state::{GameState, InstanceId, PlayerId, Zone};
use crate::card::EventName;
use mlua::{Lua, Result, Value};
use std::cell::RefCell;

pub(crate) fn pid_to_str(pid: PlayerId) -> &'static str {
    match pid {
        PlayerId::A => "a",
        PlayerId::B => "b",
    }
}

fn parse_pid(s: &str) -> Result<PlayerId> {
    match s.to_ascii_lowercase().as_str() {
        "a" => Ok(PlayerId::A),
        "b" => Ok(PlayerId::B),
        other => Err(mlua::Error::runtime(format!(
            "invalid player id (expected \"a\" or \"b\"): {other:?}"
        ))),
    }
}

fn parse_mill_zone(s: &str) -> Result<Zone> {
    match s.to_ascii_lowercase().as_str() {
        "graveyard" => Ok(Zone::Graveyard),
        "exile" => Ok(Zone::Exile),
        other => Err(mlua::Error::runtime(format!(
            "mill destination must be \"graveyard\" or \"exile\": {other:?}"
        ))),
    }
}

fn parse_zone(s: &str) -> Result<Zone> {
    match s.to_ascii_lowercase().as_str() {
        "board" => Ok(Zone::Board),
        "hand" => Ok(Zone::Hand),
        "deck" => Ok(Zone::Deck),
        "graveyard" => Ok(Zone::Graveyard),
        "exile" => Ok(Zone::Exile),
        other => Err(mlua::Error::runtime(format!(
            "unknown zone: {other:?} (expected board|hand|deck|graveyard|exile)"
        ))),
    }
}

fn remove_from_zones(s: &mut GameState, controller: PlayerId, iid: &str) -> bool {
    let p = s.player_mut(controller);
    for vec in [
        &mut p.board,
        &mut p.hand,
        &mut p.deck,
        &mut p.graveyard,
        &mut p.exile,
    ] {
        if let Some(pos) = vec.iter().position(|x| x == iid) {
            vec.remove(pos);
            return true;
        }
    }
    false
}

fn remove_from_attached(s: &mut GameState, iid: &str) -> bool {
    for host in s.card_pool.values_mut() {
        if let Some(pos) = host.attached.iter().position(|x| x == iid) {
            host.attached.remove(pos);
            return true;
        }
    }
    false
}

fn push_to_zone(s: &mut GameState, controller: PlayerId, zone: Zone, iid: String) {
    let p = s.player_mut(controller);
    match zone {
        Zone::Board => p.board.push(iid),
        Zone::Hand => p.hand.push(iid),
        Zone::Deck => p.deck.push(iid),
        Zone::Graveyard => p.graveyard.push(iid),
        Zone::Exile => p.exile.push(iid),
    }
}

// --- Pure logic for each API method. -----------------------------------------

fn do_damage(s: &mut GameState, iid: &str, n: i32) -> Result<()> {
    if let Some(inst) = s.card_pool.get_mut(iid) {
        inst.damage += n;
    }
    Ok(())
}

fn do_mill(s: &mut GameState, pid_str: &str, n: i32, dest_str: &str) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    let dest = parse_mill_zone(dest_str)?;
    let take = (n.max(0) as usize).min(s.player(pid).deck.len());
    let drained: Vec<InstanceId> = s.player_mut(pid).deck.drain(0..take).collect();
    match dest {
        Zone::Graveyard => s.player_mut(pid).graveyard.extend(drained),
        Zone::Exile => s.player_mut(pid).exile.extend(drained),
        _ => unreachable!("parse_mill_zone restricts this"),
    }
    Ok(())
}

fn do_draw(s: &mut GameState, pid_str: &str, n: i32) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    for _ in 0..n.max(0) {
        if s.player(pid).deck.is_empty() {
            // L.1 (effect-draw, same as the draw step's empty-deck rule).
            s.winner = Some(pid.opponent());
            break;
        }
        let top = s.player_mut(pid).deck.remove(0);
        s.player_mut(pid).hand.push(top);
    }
    Ok(())
}

fn do_move(s: &mut GameState, iid: &str, dest_str: &str) -> Result<()> {
    let dest = parse_zone(dest_str)?;
    let controller = s
        .card_pool
        .get(iid)
        .ok_or_else(|| mlua::Error::runtime(format!("game.move: card not in pool: {iid}")))?
        .controller;
    if !remove_from_zones(s, controller, iid) {
        if !remove_from_attached(s, iid) {
            return Err(mlua::Error::runtime(format!(
                "game.move: card not found in any zone or attached list: {iid}"
            )));
        }
        if let Some(inst) = s.card_pool.get_mut(iid) {
            inst.face_down = false;
        }
    }
    push_to_zone(s, controller, dest, iid.to_string());
    Ok(())
}

// --- Userdata builder. -------------------------------------------------------

/// Builds the per-fire-site `game` table inside an `mlua::Scope`. Macro because
/// the scoped closures need to capture borrows whose lifetimes are tied to the
/// scope; a generic fn would need explicit `'scope` lifetime gymnastics.
macro_rules! build_game_table {
    ($lua:expr, $scope:expr, $cell:expr) => {{
        let game = $lua.create_table()?;

        let cell_dmg = &$cell;
        game.set(
            "damage",
            $scope.create_function_mut(move |_, (iid, n): (String, i32)| {
                do_damage(&mut *cell_dmg.borrow_mut(), &iid, n)
            })?,
        )?;

        let cell_mill = &$cell;
        game.set(
            "mill",
            $scope.create_function_mut(move |_, (pid, n, dest): (String, i32, String)| {
                do_mill(&mut *cell_mill.borrow_mut(), &pid, n, &dest)
            })?,
        )?;

        let cell_draw = &$cell;
        game.set(
            "draw",
            $scope.create_function_mut(move |_, (pid, n): (String, i32)| {
                do_draw(&mut *cell_draw.borrow_mut(), &pid, n)
            })?,
        )?;

        let cell_move = &$cell;
        game.set(
            "move",
            $scope.create_function_mut(move |_, (iid, dest): (String, String)| {
                do_move(&mut *cell_move.borrow_mut(), &iid, &dest)
            })?,
        )?;

        game.set(
            "opponent",
            $lua.create_function(|_, pid_str: String| -> Result<String> {
                let pid = parse_pid(&pid_str)?;
                Ok(pid_to_str(pid.opponent()).to_string())
            })?,
        )?;

        game
    }};
}

fn credit_fire(state: &mut GameState, owner: PlayerId) {
    match owner {
        PlayerId::A => state.triggered_fires_a += 1,
        PlayerId::B => state.triggered_fires_b += 1,
    }
}

// --- Fire helpers per event. -------------------------------------------------

/// Fire the attacker's `on_blocked_by` handler, if any. Called from
/// `declare_blocker` once per blocker assigned. Errors log and continue.
pub(crate) fn fire_on_blocked_by(
    lua: &Lua,
    state: &mut GameState,
    attacker: &InstanceId,
    blocker: &InstanceId,
) {
    let Some(attacker_inst) = state.card_pool.get(attacker) else {
        return;
    };
    let Some(handler) = attacker_inst
        .card
        .handlers
        .get(&EventName::OnBlockedBy)
        .cloned()
    else {
        return;
    };
    let attacker_owner = attacker_inst.owner;
    let card_id = attacker_inst.card.id.clone();
    let self_iid = attacker.clone();
    let blocker_iid = blocker.clone();
    let blocker_owner = state.card_pool.get(blocker).map(|i| i.owner);

    let state_cell = RefCell::new(&mut *state);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell);

        let self_table = lua.create_table()?;
        self_table.set("instance_id", self_iid.clone())?;
        self_table.set("owner", pid_to_str(attacker_owner))?;

        let blocker_table = lua.create_table()?;
        blocker_table.set("instance_id", blocker_iid)?;
        if let Some(o) = blocker_owner {
            blocker_table.set("owner", pid_to_str(o))?;
        }

        handler.call::<()>((game, self_table, blocker_table))?;
        Ok(())
    });

    let _ = state_cell;
    match result {
        Ok(()) => credit_fire(state, attacker_owner),
        Err(e) => eprintln!("[lua] on_blocked_by handler for {card_id} failed: {e}"),
    }
}

/// Fire the dying card's `on_die` handler, if any. Called from `resolve_combat`'s
/// death loop after the Board → Graveyard move (so handlers observe the post-move
/// zone state). Errors log and continue.
pub(crate) fn fire_on_die(lua: &Lua, state: &mut GameState, dying: &InstanceId) {
    let Some(inst) = state.card_pool.get(dying) else {
        return;
    };
    let Some(handler) = inst.card.handlers.get(&EventName::OnDie).cloned() else {
        return;
    };
    let owner = inst.owner;
    let controller = inst.controller;
    let card_id = inst.card.id.clone();
    let self_iid = dying.clone();
    let attached_snapshot: Vec<InstanceId> = inst.attached.clone();

    let state_cell = RefCell::new(&mut *state);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell);

        let self_table = lua.create_table()?;
        self_table.set("instance_id", self_iid.clone())?;
        self_table.set("owner", pid_to_str(owner))?;
        self_table.set("controller", pid_to_str(controller))?;
        // Attached snapshot at the moment of death. If the handler returns
        // them via game.move, the host's live `attached` field is updated
        // by do_move; this snapshot lets handlers iterate without mutation
        // hazard.
        let attached_list = lua.create_sequence_from(attached_snapshot.clone())?;
        self_table.set("attached", attached_list)?;

        let _ = Value::Nil; // silence unused warning in case macro changes
        handler.call::<()>((game, self_table))?;
        Ok(())
    });

    let _ = state_cell;
    match result {
        Ok(()) => credit_fire(state, owner),
        Err(e) => eprintln!("[lua] on_die handler for {card_id} failed: {e}"),
    }
}
