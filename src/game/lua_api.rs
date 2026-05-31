//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Each fire-site builds a per-call `game` userdata via the `build_game_table!`
//! macro inside `Lua::scope`, so closures can borrow `&mut GameState` for the
//! duration of the handler call only.

use super::state::{
    CombatState, GameState, InstanceId, Modifier, PlayerId, StackItem, StatusEffect, Zone,
};
use crate::card::{Card, CardType, EventName, Timing};
use crate::choice::{ChoiceOracle, ChooseCardRequest, ChooseIntRequest, ChoosePlayerRequest};
use mlua::{Lua, Result, Value};
use std::cell::RefCell;

pub(crate) fn pid_to_str(pid: PlayerId) -> &'static str {
    match pid {
        PlayerId::A => "a",
        PlayerId::B => "b",
    }
}

/// Lua-visible type string. Surfaces the timing distinction for Spell cards
/// so handlers can branch on "instant" vs "sorcery" without checking a
/// separate timing field (`game.card(iid).type` matches what authors write).
fn card_type_str(c: &Card) -> &'static str {
    match c.kind {
        CardType::Unspecified => "unspecified",
        CardType::Creature => "creature",
        CardType::Spell => match c.timing {
            Some(Timing::Instant) => "instant",
            Some(Timing::Sorcery) => "sorcery",
            None => "spell",
        },
        CardType::Artifact => "artifact",
        CardType::Environment => "environment",
        CardType::Mutation => "mutation",
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
            // L.1: effect-draw on an empty deck → drawing player loses.
            // Counted separately from "voluntary suicide" plays caught by
            // preview-rollback (those increment `preview_skip_suicide`).
            // This counter is for handler-driven draws that committed —
            // typically a forced trigger from an opponent's action
            // (e.g., squirrel-overrun being blocked late game).
            s.bump_action("decked_by_handler_draw", pid);
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
    // Smart-discard heuristic: at each slot, score every hand card and
    // pick the highest score (= the most-discardable). See `discard_score`
    // for the signal mix.
    //
    // TODO(smart-discard): this v1 heuristic has known gaps:
    //   - It doesn't consider AFFORDABILITY. A 4-mill card you can't pay
    //     right now is a stronger discard candidate than the same-cost
    //     card when you have 4+ deck. Would need to walk card.cost vs
    //     player state per scoring call — measurable engine cost.
    //   - It treats every payoff handler with a fixed bonus, not weighted
    //     by ACTUAL payoff value. A draw-2 spell scores the same OnPlay
    //     bonus as a coin-flip spell.
    //   - It doesn't model "this card WANTS to be in the graveyard"
    //     (slow-recall-style cards, future graveyard-payoff). Would need
    //     a per-card `discard_payoff` field or `OnDiscarded` event.
    //   - Self-discard only. A future opponent-discard primitive (mantis-
    //     shrimp-style) should pick the worst card from the OPPONENT'S
    //     perspective — same heuristic but applied to their hand from
    //     their angle. Today, mantis-shrimp's "discard" still hits the
    //     opponent's hand[0] because it uses game.discard(opp_player, 1).
    //     Wait — actually mantis-shrimp goes through do_discard with
    //     pid = opponent, so it ALSO uses this heuristic now, picking the
    //     opponent's "least useful" card by the discarder's read. That's
    //     a free side-grade (sees opponent's hand symmetrically) but it's
    //     a kindness, not a strategy.
    //   - No tie-break. If two cards score equal the first hand position
    //     wins (deterministic but arbitrary).
    for _ in 0..take {
        let hand_snapshot: Vec<InstanceId> = s.player(pid).hand.clone();
        let chosen = hand_snapshot
            .iter()
            .max_by_key(|iid| discard_score(s, iid))
            .cloned();
        let Some(iid) = chosen else {
            break;
        };
        // Per-card-id telemetry: bump a prefixed action_counts key. This
        // piggybacks on the existing journaled bump_action plumbing so a
        // preview-and-rollback correctly undoes the count too.
        let card_id = s.card_pool.get(&iid).map(|c| c.card.id.clone());
        let _ = s.move_card(&iid, pid, Zone::Hand, Zone::Graveyard);
        s.bump_action("discard", pid);
        if let Some(cid) = card_id {
            s.bump_action(&format!("discarded:{cid}"), pid);
        }
    }
    Ok(())
}

/// Heuristic: "how much do I want to throw this card away?" Higher score =
/// better candidate for discard. Used by `do_discard` to pick the lowest-
/// value card in hand. Same shape as `choice::pitch_score` but for discard.
///
/// Signals (v1):
/// - Body value: bigger creatures are worth keeping (negative score).
/// - Handler-bearing cards have play-value worth holding (negative bonus
///   per handler kind).
/// - Pitch-payoff cards (OnAttachedAsCost handlers, e.g. jewels) are tools
///   for cost-substitution and have outsized loss-on-discard — big penalty.
fn discard_score(state: &GameState, iid: &InstanceId) -> i32 {
    let Some(c) = state.card_pool.get(iid) else {
        return 0;
    };
    let mut s = 0i32;
    let (x, y) = state.effective_stats(iid);
    s -= x + y;
    let h = &c.card.handlers;
    if h.contains_key(&crate::card::EventName::OnPlay) {
        s -= 10;
    }
    if h.contains_key(&crate::card::EventName::OnEnterBoard) {
        s -= 10;
    }
    if h.contains_key(&crate::card::EventName::OnDie) {
        s -= 5;
    }
    if h.contains_key(&crate::card::EventName::OnAttack) {
        s -= 5;
    }
    if h.contains_key(&crate::card::EventName::OnAttachedAsCost) {
        s -= 50;
    }
    s
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

fn do_add_modifier(
    s: &mut GameState,
    iid: &str,
    kind: &str,
    x: i32,
    y: i32,
    duration: Option<&str>,
) -> Result<()> {
    let owner = s.card_pool.get(iid).map(|i| i.owner);
    // duration == Some("end_of_turn") promotes a stat_boost to an
    // EotStatBoost variant that the engine clears at end-of-turn.
    let eot = matches!(duration.map(str::to_ascii_lowercase).as_deref(), Some("end_of_turn"));
    let modifier = match kind.to_ascii_lowercase().as_str() {
        "stat_boost" if eot => Modifier::EotStatBoost { x, y },
        "stat_boost" => Modifier::StatBoost { x, y },
        "gains_flying" => Modifier::GainsFlying,
        "cant_attack" => Modifier::CantAttack,
        other => {
            return Err(mlua::Error::runtime(format!(
                "game.add_modifier: unknown kind {other:?} (known: \"stat_boost\", \"gains_flying\", \"cant_attack\")"
            )))
        }
    };
    if owner.is_some() {
        s.add_modifier(&iid.to_string(), modifier);
    }
    if let Some(o) = owner {
        s.bump_action("add_modifier", o);
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

/// Cross-player move: takes `iid` (wherever it is — its current owner's
/// zone OR attached to a host) and places it in `target_player`'s `dest`
/// zone, updating controller to `target_player`. Owner stays put per T.2.
/// Used by theft effects like opponent-draw's literal "take cards from
/// opponent's deck into your hand."
///
/// Returns true iff this move is an "entry" (non-board origin → BOARD
/// destination): the caller should fire `OnEnterBoard` on `iid`. The
/// board→board case (e.g., beguile's controller-swap) returns false —
/// that's a relocation, not an entry, and shouldn't re-trigger ETB.
/// Attached → board returns true: the attached zone is a non-board
/// origin, so reanimation-from-attached behaves like reanimation-from-
/// graveyard. Hand → board likewise returns true (cards that "bypass
/// the cast" and place a creature directly).
fn do_move_to(
    s: &mut GameState,
    iid: &str,
    target_player_str: &str,
    dest_str: &str,
) -> Result<bool> {
    let dest = parse_zone(dest_str)?;
    let target = parse_pid(target_player_str)?;
    let iid_owned = iid.to_string();
    let inst = s
        .card_pool
        .get(iid)
        .ok_or_else(|| mlua::Error::runtime(format!("game.move_to: card not in pool: {iid}")))?;
    let owner = inst.owner;
    // Try owner-side zones first, then controller-side, then attached.
    // Track whether the origin was BOARD so the caller knows whether to
    // fire ETB after the move.
    let mut origin_was_board = false;
    if let Some(from) = find_zone_of(s, owner, iid) {
        origin_was_board = from == Zone::Board;
        s.remove_from_zone(&iid_owned, owner, from);
    } else if let Some(from) = find_zone_of(s, inst.controller, iid) {
        origin_was_board = from == Zone::Board;
        let ctrl = inst.controller;
        s.remove_from_zone(&iid_owned, ctrl, from);
    } else if let Some(host) = find_host_of_attached(s, iid) {
        s.remove_attached(&host, &iid_owned);
        s.set_face_down(&iid_owned, false);
        // attached origin is a non-board zone; leave origin_was_board = false.
    } else {
        return Err(mlua::Error::runtime(format!(
            "game.move_to: card not found in any zone or attached list: {iid}"
        )));
    }
    s.add_to_zone(&iid_owned, target, dest);
    s.set_controller(&iid_owned, target);
    s.bump_action("move_to", target);
    // B.3: a creature entering the BOARD from a non-board zone gets
    // summoning sickness, same as the play_card path sets after a hard
    // cast. Reanimation, hand→board placements, exile→board returns all
    // behave consistently. Handlers that want haste-on-entry (e.g., a
    // Reanimate variant) explicitly clear it via game.set_summoning_sick
    // after the move_to call. Artifacts skip per the play.rs convention
    // (B.3 is creature-specific).
    let etb = dest == Zone::Board && !origin_was_board;
    if etb {
        let is_creature = s
            .card_pool
            .get(iid)
            .map(|inst| inst.card.kind == CardType::Creature)
            .unwrap_or(false);
        if is_creature {
            s.set_summoning_sick(&iid_owned, true);
        }
    }
    Ok(etb)
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
                    // Handler-side game.choose_card has no payment context
                    // (host = None). State is passed through to the oracle
                    // which now reads controllers / stats directly.
                    let req = ChooseCardRequest {
                        pool,
                        asker: Some(choose_owner),
                        host: None,
                        optional,
                        prompt,
                    };
                    let answer = {
                        let s = cell_choose_s.borrow();
                        let mut o = cell_choose_o.borrow_mut();
                        o.choose_card(&s, req)
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
                    let s = cell_confirm_s.borrow();
                    let mut o = cell_confirm_o.borrow_mut();
                    o.confirm(&s, confirm_owner, &prompt)
                };
                cell_confirm_s.borrow_mut().bump_action("confirm", confirm_owner);
                Ok(answer)
            })?,
        )?;

        // Ask a SPECIFIC player a yes/no question. Used by cards like
        // BCI MegaFly where the trigger asks the OPPONENT (not the card's
        // owner) to decide. The asker plumbs to the oracle for recording /
        // future biased heuristics; RandomOracle still answers via gen_bool.
        let cell_confirm_for_o = &$oracle_cell;
        let cell_confirm_for_s = &$cell;
        game.set(
            "confirm_for",
            $scope.create_function_mut(
                move |_, (pid_str, prompt): (String, String)| -> Result<bool> {
                    let pid = parse_pid(&pid_str)?;
                    let answer = {
                        let s = cell_confirm_for_s.borrow();
                        let mut o = cell_confirm_for_o.borrow_mut();
                        o.confirm(&s, pid, &prompt)
                    };
                    cell_confirm_for_s.borrow_mut().bump_action("confirm", pid);
                    Ok(answer)
                },
            )?,
        )?;

        // Same as `choose_card` but the asker is an explicit player id
        // rather than the card's owner. For opponent-side picks like BCI
        // MegaFly's "opponent picks an artifact to sacrifice."
        let cell_cc_for_o = &$oracle_cell;
        let cell_cc_for_s = &$cell;
        game.set(
            "choose_card_for",
            $scope.create_function_mut(
                move |_, (pid_str, pool, opts): (String, Vec<String>, Option<mlua::Table>)| -> Result<Option<String>> {
                    let pid = parse_pid(&pid_str)?;
                    let (optional, prompt) = match opts {
                        Some(t) => (
                            t.get::<Option<bool>>("optional")?.unwrap_or(false),
                            t.get::<Option<String>>("prompt")?.unwrap_or_default(),
                        ),
                        None => (false, String::new()),
                    };
                    let req = ChooseCardRequest {
                        pool,
                        asker: Some(pid),
                        host: None,
                        optional,
                        prompt,
                    };
                    let answer = {
                        let s = cell_cc_for_s.borrow();
                        let mut o = cell_cc_for_o.borrow_mut();
                        o.choose_card(&s, req)
                    };
                    cell_cc_for_s.borrow_mut().bump_action("choose_card", pid);
                    Ok(answer)
                },
            )?,
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
                        let s = cell_player_s.borrow();
                        let mut o = cell_player_o.borrow_mut();
                        o.choose_player(&s, req)
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
                        let s = cell_int_s.borrow();
                        let mut o = cell_int_o.borrow_mut();
                        o.choose_int(&s, req)
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

        let cell_move_to = &$cell;
        let cell_move_to_o = &$oracle_cell;
        let move_to_lua = $lua;
        game.set(
            "move_to",
            $scope.create_function_mut(
                move |_, (iid, target_player, dest): (String, String, String)| {
                    let fire_etb = do_move_to(
                        &mut *cell_move_to.borrow_mut(),
                        &iid,
                        &target_player,
                        &dest,
                    )?;
                    if fire_etb {
                        let mut s = cell_move_to.borrow_mut();
                        let mut o = cell_move_to_o.borrow_mut();
                        fire_self_only(
                            move_to_lua,
                            &mut *s,
                            &mut **o,
                            EventName::OnEnterBoard,
                            &iid,
                        );
                    }
                    Ok(())
                },
            )?,
        )?;

        game.set(
            "opponent",
            $lua.create_function(|_, pid_str: String| -> Result<String> {
                let pid = parse_pid(&pid_str)?;
                Ok(pid_to_str(pid.opponent()).to_string())
            })?,
        )?;

        let cell_atk_q = &$cell;
        game.set(
            "creature_attacked_this_turn",
            $scope.create_function_mut(move |_, ()| -> Result<bool> {
                Ok(cell_atk_q.borrow().creature_attacked_this_turn)
            })?,
        )?;

        // Counter-the-top: convenience for "counter the spell directly
        // underneath me." Used by counterspell.
        let cell_counter_top = &$cell;
        let counter_top_owner = $owner;
        game.set(
            "counter_top",
            $scope.create_function_mut(move |_, ()| -> Result<bool> {
                let mut s = cell_counter_top.borrow_mut();
                let removed = s.counter_top();
                if removed.is_some() {
                    s.bump_action("counter_top", counter_top_owner);
                }
                Ok(removed.is_some())
            })?,
        )?;

        // Counter-target: removes a specific chain item by InstanceId.
        // Returns true if the target was on the chain and got countered,
        // false otherwise. For cards like DTST-creature's "counter target
        // card on the stack" where the controller picks which item.
        let cell_counter_t = &$cell;
        let counter_t_owner = $owner;
        game.set(
            "counter",
            $scope.create_function_mut(move |_, target: String| -> Result<bool> {
                let mut s = cell_counter_t.borrow_mut();
                let removed = s.counter_target(&target);
                if removed.is_some() {
                    s.bump_action("counter", counter_t_owner);
                }
                Ok(removed.is_some())
            })?,
        )?;

        // Chain inspector: returns the response chain as a Lua array of
        // tables [{card, controller, kind}, ...]. Bottom of chain at
        // index 1, top at #chain. Empty array if no window is open.
        let cell_chain = &$cell;
        game.set(
            "chain",
            $scope.create_function_mut(move |lua, ()| -> Result<mlua::Table> {
                let s = cell_chain.borrow();
                let arr = lua.create_table()?;
                if let Some(p) = s.priority.as_ref() {
                    for (i, item) in p.chain.iter().enumerate() {
                        let t = lua.create_table()?;
                        let StackItem::PlayedCard { card, controller, .. } = item;
                        t.set("card", card.clone())?;
                        t.set("controller", pid_to_str(*controller).to_string())?;
                        t.set("kind", "played_card")?;
                        arr.set(i + 1, t)?;
                    }
                }
                Ok(arr)
            })?,
        )?;

        // Legal targets for a counter effect: the InstanceIds of chain
        // items right now. Used by handlers to populate `choose_card`
        // pools for "counter target spell" choices.
        let cell_ct = &$cell;
        game.set(
            "legal_counter_targets",
            $scope.create_function_mut(move |lua, ()| -> Result<mlua::Table> {
                let s = cell_ct.borrow();
                let targets = s.legal_counter_targets();
                let arr = lua.create_table()?;
                for (i, t) in targets.iter().enumerate() {
                    arr.set(i + 1, t.clone())?;
                }
                Ok(arr)
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

        // game.set_summoning_sick(iid, sick) — used by reanimate-style
        // handlers that want haste-on-entry (call after move_to with false
        // to clear the sickness move_to automatically applied). Idempotent
        // no-op if the iid isn't in the pool.
        let cell_ss = &$cell;
        game.set(
            "set_summoning_sick",
            $scope.create_function_mut(move |_, (iid, sick): (String, bool)| -> Result<()> {
                cell_ss.borrow_mut().set_summoning_sick(&iid, sick);
                Ok(())
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

        let cell_mod = &$cell;
        game.set(
            "add_modifier",
            $scope.create_function_mut(
                move |_,
                      (iid, kind, x, y, duration): (
                    String,
                    String,
                    i32,
                    i32,
                    Option<String>,
                )| {
                    do_add_modifier(
                        &mut *cell_mod.borrow_mut(),
                        &iid,
                        &kind,
                        x,
                        y,
                        duration.as_deref(),
                    )
                },
            )?,
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
                    t.set("type", card_type_str(&inst.card))?;
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
                    t.set("attacked_this_turn", inst.attacked_this_turn)?;
                    t.set("owner", pid_to_str(inst.owner))?;
                    t.set("controller", pid_to_str(inst.controller))?;
                    let (x, y) = s.effective_stats(&iid);
                    t.set("x", x)?;
                    t.set("y", y)?;
                    t.set(
                        "attached",
                        lua.create_sequence_from(inst.attached.clone())?,
                    )?;
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
// Design (2026-05-30): consequential triggers do NOT go on the stack. ETB
// (`OnEnterBoard`), `OnPlay`, `OnAttack`, `OnBlock`, `OnBlockedBy`, `OnDie`
// all fire inline as part of resolving the action that caused them. The
// stack only carries the cast / declaration itself, plus instants cast in
// response. This kills the MTG "kill-with-priority-on-the-trigger" two-shot
// but keeps the cleaner "counter the spell / kill the attacker before its
// effect fires" windows. No "queue trigger" rework needed here.
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

/// Fire an activated-ability handler. Same shape as fire_self_only
/// (handler takes `(game, self)`), but the handler is passed in by
/// reference rather than looked up by event name. Used by
/// `GameState::activate_ability` after cost has been paid. Per RULES
/// A.5 the effect resolves immediately and no response window opens.
pub(crate) fn fire_activated(
    lua: &Lua,
    state: &mut GameState,
    oracle: &mut dyn ChoiceOracle,
    source: &InstanceId,
    handler: mlua::Function,
) {
    let Some(inst) = state.card_pool.get(source) else {
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
        let _ = Value::Nil;
        Ok(())
    });

    let _ = state_cell;
    let _ = oracle_cell;
    if let Err(e) = result {
        eprintln!("[lua] activated handler for {card_id} failed: {e}");
    }
}

/// Fire an event whose handler takes `(game, self, partner)`. Used for
/// `on_blocked_by` (self=attacker, partner=blocker) and `on_block`
/// (self=blocker, partner=attacker). Errors log and continue.
// Same design as fire_self_only: `OnBlock` / `OnBlockedBy` fire inline as
// part of resolving the block declaration. Stack carries the declaration
// itself (R.1.c), not the trigger.
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
