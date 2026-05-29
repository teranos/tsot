//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Each fire-site builds a per-call `game` userdata via the `build_game_table!`
//! macro inside `Lua::scope`, so closures can borrow `&mut GameState` for the
//! duration of the handler call only.

use super::state::{CombatState, GameState, InstanceId, PlayerId, Zone};
use crate::card::{CardType, EventName};
use mlua::{Lua, Result, Value};
use std::cell::RefCell;

pub(crate) fn pid_to_str(pid: PlayerId) -> &'static str {
    match pid {
        PlayerId::A => "a",
        PlayerId::B => "b",
    }
}

fn card_type_str(t: CardType) -> &'static str {
    match t {
        CardType::Unspecified => "unspecified",
        CardType::Creature => "creature",
        CardType::Instant => "instant",
        CardType::Spell => "spell",
        CardType::Artifact => "artifact",
        CardType::Environment => "environment",
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
    let owner = s.card_pool.get(iid).map(|i| i.owner);
    if let Some(inst) = s.card_pool.get_mut(iid) {
        inst.damage += n;
    }
    if let Some(owner) = owner {
        s.bump_action("damage", owner);
    }
    Ok(())
}

fn do_mill(s: &mut GameState, pid_str: &str, n: i32, dest_str: &str) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    let dest = parse_mill_zone(dest_str)?;
    let take = (n.max(0) as usize).min(s.player(pid).deck.len());
    let drained: Vec<InstanceId> = s.player_mut(pid).deck.drain(0..take).collect();
    let actually_milled = drained.len() as u32;
    match dest {
        Zone::Graveyard => s.player_mut(pid).graveyard.extend(drained),
        Zone::Exile => s.player_mut(pid).exile.extend(drained),
        _ => unreachable!("parse_mill_zone restricts this"),
    }
    for _ in 0..actually_milled {
        s.bump_action("mill", pid);
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
        s.bump_action("draw", pid);
    }
    Ok(())
}

fn do_set_tapped(s: &mut GameState, iid: &str, tapped: bool) -> Result<()> {
    let owner = s.card_pool.get(iid).map(|i| i.owner);
    if let Some(inst) = s.card_pool.get_mut(iid) {
        inst.tapped = tapped;
    }
    if let Some(o) = owner {
        s.bump_action(if tapped { "tap" } else { "untap" }, o);
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
    s.bump_action("move", controller);
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

        let cell_top = &$cell;
        game.set(
            "deck_top",
            $scope.create_function_mut(move |_, pid_str: String| -> Result<Option<String>> {
                let pid = parse_pid(&pid_str)?;
                let s = cell_top.borrow();
                Ok(s.player(pid).deck.first().cloned())
            })?,
        )?;

        let cell_tap = &$cell;
        game.set(
            "tap",
            $scope.create_function_mut(move |_, iid: String| {
                do_set_tapped(&mut *cell_tap.borrow_mut(), &iid, true)
            })?,
        )?;

        let cell_untap = &$cell;
        game.set(
            "untap",
            $scope.create_function_mut(move |_, iid: String| {
                do_set_tapped(&mut *cell_untap.borrow_mut(), &iid, false)
            })?,
        )?;

        let cell_atk = &$cell;
        game.set(
            "attackers",
            $scope.create_function_mut(move |lua, _: ()| -> Result<mlua::Table> {
                let s = cell_atk.borrow();
                let list: Vec<InstanceId> = match &s.combat {
                    Some(CombatState::AwaitingBlockers { attacks }) => {
                        attacks.iter().map(|a| a.attacker.clone()).collect()
                    }
                    _ => Vec::new(),
                };
                lua.create_sequence_from(list)
            })?,
        )?;

        let cell_zones = &$cell;
        game.set(
            "zones",
            $scope.create_function_mut(move |lua, pid_str: String| -> Result<mlua::Table> {
                let pid = parse_pid(&pid_str)?;
                let s = cell_zones.borrow();
                let p = s.player(pid);
                let t = lua.create_table()?;
                t.set("hand", lua.create_sequence_from(p.hand.clone())?)?;
                t.set("deck", lua.create_sequence_from(p.deck.clone())?)?;
                t.set("graveyard", lua.create_sequence_from(p.graveyard.clone())?)?;
                t.set("exile", lua.create_sequence_from(p.exile.clone())?)?;
                t.set("board", lua.create_sequence_from(p.board.clone())?)?;
                Ok(t)
            })?,
        )?;

        let cell_card = &$cell;
        game.set(
            "card",
            $scope.create_function_mut(
                move |lua, iid: String| -> Result<Option<mlua::Table>> {
                    let s = cell_card.borrow();
                    let Some(inst) = s.card_pool.get(&iid) else {
                        return Ok(None);
                    };
                    let t = lua.create_table()?;
                    t.set("id", inst.card.id.clone())?;
                    t.set("instance_id", iid.clone())?;
                    t.set("type", card_type_str(inst.card.kind))?;
                    t.set(
                        "subtypes",
                        lua.create_sequence_from(inst.card.subtypes.clone())?,
                    )?;
                    t.set(
                        "colors",
                        lua.create_sequence_from(inst.card.colors.clone())?,
                    )?;
                    t.set("symbol", inst.card.symbol.clone())?;
                    t.set("tapped", inst.tapped)?;
                    t.set("face_down", inst.face_down)?;
                    t.set("owner", pid_to_str(inst.owner))?;
                    t.set("controller", pid_to_str(inst.controller))?;
                    let (x, y) = s.effective_stats(&iid);
                    t.set("x", x)?;
                    t.set("y", y)?;
                    Ok(Some(t))
                },
            )?,
        )?;

        game
    }};
}

fn credit_fire(state: &mut GameState, event: EventName, owner: PlayerId) {
    state.bump_event_fire(event, owner);
}

// --- Fire helpers per event. -------------------------------------------------

fn build_self_table(
    lua: &Lua,
    state: &GameState,
    iid: &InstanceId,
) -> Result<mlua::Table> {
    let inst = state
        .card_pool
        .get(iid)
        .ok_or_else(|| mlua::Error::runtime(format!("build_self_table: not in pool: {iid}")))?;
    let t = lua.create_table()?;
    t.set("instance_id", iid.clone())?;
    t.set("owner", pid_to_str(inst.owner))?;
    t.set("controller", pid_to_str(inst.controller))?;
    let attached_list = lua.create_sequence_from(inst.attached.clone())?;
    t.set("attached", attached_list)?;
    Ok(t)
}

/// Fire an event whose handler takes `(game, self)`. Used for `on_die`,
/// `on_enter_board`, `on_attack`, `on_play`. Errors log and continue.
pub(crate) fn fire_self_only(
    lua: &Lua,
    state: &mut GameState,
    event: EventName,
    source: &InstanceId,
) {
    let Some(inst) = state.card_pool.get(source) else {
        return;
    };
    let Some(handler) = inst.card.handlers.get(&event).cloned() else {
        return;
    };
    let owner = inst.owner;
    let card_id = inst.card.id.clone();

    let state_cell = RefCell::new(&mut *state);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        handler.call::<()>((game, self_table))?;
        let _ = Value::Nil; // keep import warm
        Ok(())
    });

    let _ = state_cell;
    match result {
        Ok(()) => credit_fire(state, event, owner),
        Err(e) => eprintln!("[lua] {} handler for {card_id} failed: {e}", event.lua_key()),
    }
}

/// Fire an event whose handler takes `(game, self, partner)`. Used for
/// `on_blocked_by` (self=attacker, partner=blocker) and `on_block`
/// (self=blocker, partner=attacker). Errors log and continue.
pub(crate) fn fire_with_partner(
    lua: &Lua,
    state: &mut GameState,
    event: EventName,
    source: &InstanceId,
    partner: &InstanceId,
) {
    let Some(inst) = state.card_pool.get(source) else {
        return;
    };
    let Some(handler) = inst.card.handlers.get(&event).cloned() else {
        return;
    };
    let owner = inst.owner;
    let card_id = inst.card.id.clone();

    let state_cell = RefCell::new(&mut *state);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        let partner_table = build_self_table(lua, &state_cell.borrow(), partner)?;
        handler.call::<()>((game, self_table, partner_table))?;
        Ok(())
    });

    let _ = state_cell;
    match result {
        Ok(()) => credit_fire(state, event, owner),
        Err(e) => eprintln!("[lua] {} handler for {card_id} failed: {e}", event.lua_key()),
    }
}
