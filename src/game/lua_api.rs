//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Each fire-site builds a per-call `game` userdata via the `build_game_table!`
//! macro inside `Lua::scope`, so closures can borrow `&mut GameState` for the
//! duration of the handler call only.

use super::state::{CombatState, GameState, InstanceId, PlayerId, StatusEffect, Zone};
use crate::card::{CardType, EventName};
use crate::choice::{ChoiceOracle, ChooseCardRequest, ChooseIntRequest, ChoosePlayerRequest};
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

/// Search the owner's zones for the iid; returns the zone if found.
fn find_zone_of(s: &GameState, owner: PlayerId, iid: &str) -> Option<Zone> {
    let p = s.player(owner);
    if p.board.iter().any(|x| x == iid) {
        return Some(Zone::Board);
    }
    if p.hand.iter().any(|x| x == iid) {
        return Some(Zone::Hand);
    }
    if p.deck.iter().any(|x| x == iid) {
        return Some(Zone::Deck);
    }
    if p.graveyard.iter().any(|x| x == iid) {
        return Some(Zone::Graveyard);
    }
    if p.exile.iter().any(|x| x == iid) {
        return Some(Zone::Exile);
    }
    None
}

/// Search all card_pool instances for an `attached` containing `iid`.
/// Returns the host's iid if found.
fn find_host_of_attached(s: &GameState, iid: &str) -> Option<InstanceId> {
    for (host_iid, host) in &s.card_pool {
        if host.attached.iter().any(|x| x == iid) {
            return Some(host_iid.clone());
        }
    }
    None
}

// --- Pure logic for each API method. All mutations go through journaled
// helpers on GameState so handler effects are rollback-safe.

fn do_damage(s: &mut GameState, iid: &str, n: i32) -> Result<()> {
    let (owner, current) = match s.card_pool.get(iid) {
        Some(inst) => (Some(inst.owner), inst.damage),
        None => (None, 0),
    };
    if owner.is_some() {
        s.set_damage(&iid.to_string(), current + n);
    }
    if let Some(o) = owner {
        s.bump_action("damage", o);
    }
    Ok(())
}

fn do_mill(s: &mut GameState, pid_str: &str, n: i32, dest_str: &str) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    let dest = parse_mill_zone(dest_str)?;
    let take = (n.max(0) as usize).min(s.player(pid).deck.len());
    for _ in 0..take {
        let Some(top) = s.player(pid).deck.first().cloned() else {
            break;
        };
        let _ = s.move_card(&top, pid, Zone::Deck, dest);
        s.bump_action("mill", pid);
    }
    Ok(())
}

fn do_draw(s: &mut GameState, pid_str: &str, n: i32) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    for _ in 0..n.max(0) {
        if s.player(pid).deck.is_empty() {
            // L.1 (effect-draw, same as the draw step's empty-deck rule).
            s.bump_action("self_deckout_by_choice", pid);
            s.set_winner(Some(pid.opponent()));
            break;
        }
        let Some(top) = s.player(pid).deck.first().cloned() else {
            break;
        };
        let _ = s.move_card(&top, pid, Zone::Deck, Zone::Hand);
        s.bump_action("draw", pid);
    }
    Ok(())
}

fn do_discard(s: &mut GameState, pid_str: &str, n: i32) -> Result<()> {
    let pid = parse_pid(pid_str)?;
    let take = (n.max(0) as usize).min(s.player(pid).hand.len());
    // Deterministic front-of-hand pending choice API surface (Phase 2+).
    for _ in 0..take {
        let Some(front) = s.player(pid).hand.first().cloned() else {
            break;
        };
        let _ = s.move_card(&front, pid, Zone::Hand, Zone::Graveyard);
        s.bump_action("discard", pid);
    }
    Ok(())
}

fn do_add_status(s: &mut GameState, iid: &str, kind: &str, duration: i32) -> Result<()> {
    let (owner, current_effects) = match s.card_pool.get(iid) {
        Some(inst) => (Some(inst.owner), inst.status_effects.clone()),
        None => return Ok(()),
    };
    match kind.to_ascii_lowercase().as_str() {
        "skip_untap" => {
            let n = duration.max(0) as u32;
            let mut new_effects = current_effects;
            let existing = new_effects
                .iter()
                .position(|s| matches!(s, StatusEffect::SkipUntap(_)));
            if let Some(idx) = existing {
                let StatusEffect::SkipUntap(old) = new_effects[idx];
                new_effects[idx] = StatusEffect::SkipUntap(old + n);
            } else {
                new_effects.push(StatusEffect::SkipUntap(n));
            }
            s.set_status_effects(&iid.to_string(), new_effects);
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "game.add_status: unknown kind {other:?} (known: \"skip_untap\")"
            )))
        }
    }
    if let Some(o) = owner {
        s.bump_action("add_status", o);
    }
    Ok(())
}

fn do_set_tapped(s: &mut GameState, iid: &str, tapped: bool) -> Result<()> {
    let owner = s.card_pool.get(iid).map(|i| i.owner);
    if owner.is_some() {
        s.set_tapped(&iid.to_string(), tapped);
    }
    if let Some(o) = owner {
        s.bump_action(if tapped { "tap" } else { "untap" }, o);
    }
    Ok(())
}

fn do_move(s: &mut GameState, iid: &str, dest_str: &str) -> Result<()> {
    let dest = parse_zone(dest_str)?;
    let iid_owned = iid.to_string();
    let controller = s
        .card_pool
        .get(iid)
        .ok_or_else(|| mlua::Error::runtime(format!("game.move: card not in pool: {iid}")))?
        .controller;

    if let Some(from) = find_zone_of(s, controller, iid) {
        let _ = s.move_card(&iid_owned, controller, from, dest);
    } else if let Some(host) = find_host_of_attached(s, iid) {
        s.remove_attached(&host, &iid_owned);
        s.set_face_down(&iid_owned, false);
        s.add_to_zone(&iid_owned, controller, dest);
    } else {
        return Err(mlua::Error::runtime(format!(
            "game.move: card not found in any zone or attached list: {iid}"
        )));
    }
    s.bump_action("move", controller);
    Ok(())
}

// --- Userdata builder. -------------------------------------------------------

/// Builds the per-fire-site `game` table inside an `mlua::Scope`. Macro because
/// the scoped closures need to capture borrows whose lifetimes are tied to the
/// scope; a generic fn would need explicit `'scope` lifetime gymnastics.
macro_rules! build_game_table {
    ($lua:expr, $scope:expr, $cell:expr, $oracle_cell:expr, $owner:expr) => {{
        let game = $lua.create_table()?;

        let cell_choose_o = &$oracle_cell;
        let cell_choose_s = &$cell;
        let choose_owner = $owner;
        game.set(
            "choose_card",
            $scope.create_function_mut(
                move |_, (pool, opts): (Vec<String>, Option<mlua::Table>)| -> Result<Option<String>> {
                    let (optional, prompt) = match opts {
                        Some(t) => (
                            t.get::<Option<bool>>("optional")?.unwrap_or(false),
                            t.get::<Option<String>>("prompt")?.unwrap_or_default(),
                        ),
                        None => (false, String::new()),
                    };
                    let req = ChooseCardRequest { pool, optional, prompt };
                    let answer = {
                        let mut o = cell_choose_o.borrow_mut();
                        o.choose_card(req)
                    };
                    cell_choose_s.borrow_mut().bump_action("choose_card", choose_owner);
                    Ok(answer)
                },
            )?,
        )?;

        let cell_confirm_o = &$oracle_cell;
        let cell_confirm_s = &$cell;
        let confirm_owner = $owner;
        game.set(
            "confirm",
            $scope.create_function_mut(move |_, prompt: String| -> Result<bool> {
                let answer = {
                    let mut o = cell_confirm_o.borrow_mut();
                    o.confirm(&prompt)
                };
                cell_confirm_s.borrow_mut().bump_action("confirm", confirm_owner);
                Ok(answer)
            })?,
        )?;

        let cell_player_o = &$oracle_cell;
        let cell_player_s = &$cell;
        let player_owner = $owner;
        game.set(
            "choose_player",
            $scope.create_function_mut(
                move |_, opts: Option<mlua::Table>| -> Result<Option<String>> {
                    let (exclude_str, optional, prompt) = match opts {
                        Some(t) => (
                            t.get::<Option<Vec<String>>>("exclude")?.unwrap_or_default(),
                            t.get::<Option<bool>>("optional")?.unwrap_or(false),
                            t.get::<Option<String>>("prompt")?.unwrap_or_default(),
                        ),
                        None => (Vec::new(), false, String::new()),
                    };
                    let mut exclude: Vec<PlayerId> = Vec::new();
                    for s in &exclude_str {
                        exclude.push(parse_pid(s)?);
                    }
                    let req = ChoosePlayerRequest {
                        exclude,
                        optional,
                        prompt,
                    };
                    let answer = {
                        let mut o = cell_player_o.borrow_mut();
                        o.choose_player(req)
                    };
                    cell_player_s
                        .borrow_mut()
                        .bump_action("choose_player", player_owner);
                    Ok(answer.map(|p| pid_to_str(p).to_string()))
                },
            )?,
        )?;

        let cell_int_o = &$oracle_cell;
        let cell_int_s = &$cell;
        let int_owner = $owner;
        game.set(
            "choose_int",
            $scope.create_function_mut(
                move |_, (min, max, prompt): (i32, i32, Option<String>)| -> Result<i32> {
                    let req = ChooseIntRequest {
                        min,
                        max,
                        prompt: prompt.unwrap_or_default(),
                    };
                    let answer = {
                        let mut o = cell_int_o.borrow_mut();
                        o.choose_int(req)
                    };
                    cell_int_s.borrow_mut().bump_action("choose_int", int_owner);
                    Ok(answer)
                },
            )?,
        )?;

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

        game.set(
            "print",
            $lua.create_function(|_, msg: String| -> Result<()> {
                eprintln!("[card] {msg}");
                Ok(())
            })?,
        )?;

        let cell_status = &$cell;
        game.set(
            "add_status",
            $scope.create_function_mut(
                move |_, (iid, kind, duration): (String, String, i32)| {
                    do_add_status(&mut *cell_status.borrow_mut(), &iid, &kind, duration)
                },
            )?,
        )?;

        let cell_discard = &$cell;
        game.set(
            "discard",
            $scope.create_function_mut(move |_, (pid, n): (String, i32)| {
                do_discard(&mut *cell_discard.borrow_mut(), &pid, n)
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
    oracle: &mut dyn ChoiceOracle,
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
    let oracle_cell = RefCell::new(&mut *oracle);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        handler.call::<()>((game, self_table))?;
        let _ = Value::Nil; // keep import warm
        Ok(())
    });

    let _ = state_cell;
    let _ = oracle_cell;
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
    oracle: &mut dyn ChoiceOracle,
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
    let oracle_cell = RefCell::new(&mut *oracle);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        let partner_table = build_self_table(lua, &state_cell.borrow(), partner)?;
        handler.call::<()>((game, self_table, partner_table))?;
        Ok(())
    });

    let _ = state_cell;
    let _ = oracle_cell;
    match result {
        Ok(()) => credit_fire(state, event, owner),
        Err(e) => eprintln!("[lua] {} handler for {card_id} failed: {e}", event.lua_key()),
    }
}
