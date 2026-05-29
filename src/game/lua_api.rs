//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Wires the minimum `game` userdata that handlers need to actually do work:
//! `game.damage`, `game.mill`, `game.opponent`. Built per fire-site as a
//! scoped table so the closures can borrow `&mut GameState` for the duration
//! of the handler call only.

use super::state::{GameState, InstanceId, PlayerId, Zone};
use crate::card::EventName;
use mlua::{Lua, Result};
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
        let game = lua.create_table()?;

        let cell_dmg = &state_cell;
        game.set(
            "damage",
            scope.create_function_mut(move |_, (iid, n): (String, i32)| {
                let mut s = cell_dmg.borrow_mut();
                if let Some(inst) = s.card_pool.get_mut(&iid) {
                    inst.damage += n;
                }
                Ok(())
            })?,
        )?;

        let cell_mill = &state_cell;
        game.set(
            "mill",
            scope.create_function_mut(
                move |_, (pid_str, n, dest_str): (String, i32, String)| {
                    let pid = parse_pid(&pid_str)?;
                    let dest = parse_mill_zone(&dest_str)?;
                    let mut s = cell_mill.borrow_mut();
                    let take = (n.max(0) as usize).min(s.player(pid).deck.len());
                    let drained: Vec<InstanceId> =
                        s.player_mut(pid).deck.drain(0..take).collect();
                    match dest {
                        Zone::Graveyard => s.player_mut(pid).graveyard.extend(drained),
                        Zone::Exile => s.player_mut(pid).exile.extend(drained),
                        _ => unreachable!("parse_mill_zone restricts this"),
                    }
                    Ok(())
                },
            )?,
        )?;

        let cell_draw = &state_cell;
        game.set(
            "draw",
            scope.create_function_mut(move |_, (pid_str, n): (String, i32)| {
                let pid = parse_pid(&pid_str)?;
                let mut s = cell_draw.borrow_mut();
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
            })?,
        )?;

        game.set(
            "opponent",
            lua.create_function(|_, pid_str: String| -> Result<String> {
                let pid = parse_pid(&pid_str)?;
                Ok(pid_to_str(pid.opponent()).to_string())
            })?,
        )?;

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
        Ok(()) => match attacker_owner {
            PlayerId::A => state.triggered_fires_a += 1,
            PlayerId::B => state.triggered_fires_b += 1,
        },
        Err(e) => {
            eprintln!("[lua] on_blocked_by handler for {card_id} failed: {e}");
        }
    }
}
