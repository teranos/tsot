//! Lua-facing API surface for card event handlers (LUA.md Phase 1).
//!
//! Each fire-site builds a per-call `game` userdata via the `build_game_table!`
//! macro inside `Lua::scope`, so closures can borrow `&mut GameState` for the
//! duration of the handler call only.

use super::state::{
    CombatState, GameState, InstanceId, Modifier, PlayerId, StackItem, StatusEffect, Zone,
};
use crate::card::{Card, CardType, EventName, Timing};
use crate::choice::{
    ChoiceOracle, ChoicePending, ChooseCardRequest, ChooseIntRequest, ChoosePlayerRequest,
    TargetIntent,
};
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
        CardType::Symbol => "symbol",
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

pub(crate) fn do_damage(s: &mut GameState, target: &str, n: f32) -> Result<()> {
    // Two targeting modes per RULES (cards say "deal N damage to any
    // target" — creature or player):
    //
    //   1. Target is a player id ("a"/"b") → player damage. No life
    //      total exists (X.1: no mana; X.2: no lands; L.1: deck-out
    //      loses), so "damage to player" mills N cards from their
    //      DECK. Closest analog to MTG-shape "lose N life."
    //   2. Target is a creature iid → existing behavior: accumulate
    //      damage on the creature, run B.8 death check.
    //
    // Disambiguation: a player id is exactly 1 char, lowercase a or
    // b. Card iids look like "A:0007" / "B:0014" — distinct shape.
    if matches!(target, "a" | "b") {
        let pid = match target {
            "a" => crate::game::PlayerId::A,
            "b" => crate::game::PlayerId::B,
            _ => unreachable!(),
        };
        let take = (n.max(0.0) as usize).min(s.player(pid).deck.len());
        for _ in 0..take {
            if let Some(top) = s.player(pid).deck.first().cloned() {
                let _ = s.move_card(&top, pid, crate::game::Zone::Deck, crate::game::Zone::Exile);
            }
        }
        s.bump_action("damage", pid);
        // Deck-out loss check (RULES L.1).
        if s.player(pid).deck.is_empty() && s.winner.is_none() {
            s.winner = Some(pid.opponent());
        }
        return Ok(());
    }
    let (owner, current) = match s.card_pool.get(target) {
        Some(inst) => (Some(inst.owner), inst.damage),
        None => (None, 0.0),
    };
    if owner.is_some() {
        s.set_damage(&target.to_string(), current + n);
    }
    if let Some(o) = owner {
        s.bump_action("damage", o);
    }
    // RULES B.8: a creature with accumulated damage ≥ effective Y
    // dies. Combat damage already runs this check inside
    // confirm_blocks; Lua-driven damage (game.damage from on_play /
    // on_attack / etc) used to skip it, leaving creatures with
    // damage ≥ Y standing on the board (Read the Embers + Ember Bat
    // bug, 2026-06-16). Reuse the shared sweep so both paths share
    // semantics.
    s.cleanup_b8_damage_deaths();
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
            s.set_winner(Some(pid.opponent()), "deckout_handler_draw");
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
    do_smart_discard(s, pid, n.max(0) as usize);
    Ok(())
}

/// Same smart-discard loop `game.discard` uses, but takes a typed
/// `PlayerId` and a clamped count — callable from engine code that's
/// already parsed the player. Used by activated-ability cost payment
/// in `play.rs::activate_ability` for HAND-source components.
pub(crate) fn do_smart_discard(s: &mut GameState, pid: PlayerId, n: usize) {
    let take = n.min(s.player(pid).hand.len());
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
    s -= (x + y).round() as i32;
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
    x: f32,
    y: f32,
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
        "gains_vigilance" if eot => Modifier::EotGainsVigilance,
        "gains_vigilance" => Modifier::GainsVigilance,
        "gains_haste" if eot => Modifier::EotGainsHaste,
        "gains_haste" => Modifier::GainsHaste,
        "cant_attack" => Modifier::CantAttack,
        other => {
            return Err(mlua::Error::runtime(format!(
                "game.add_modifier: unknown kind {other:?} (known: \"stat_boost\", \"gains_flying\", \"gains_vigilance\", \"gains_haste\", \"cant_attack\")"
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
                        // Pending is carried up to fire_*'s caller as a
                        // typed external error (`Error::external(p)`).
                        // The wrapper there downcasts and re-emits it as
                        // Err(ChoicePending), letting the engine surface a
                        // HumanPrompt + rollback-and-replay (see LIMITATIONS.md
                        // ## lua for the resolved design — coroutine.yield
                        // is blocked across C-call boundaries by Lua itself,
                        // so the replay path is the only viable approach).
                        o.choose_card(&s, req).map_err(mlua::Error::external)?
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
                        .map_err(mlua::Error::external)?
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
                        o.confirm(&s, pid, &prompt).map_err(mlua::Error::external)?
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
                        o.choose_card(&s, req).map_err(mlua::Error::external)?
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
                        o.choose_player(&s, req).map_err(mlua::Error::external)?
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
                        o.choose_int(&s, req).map_err(mlua::Error::external)?
                    };
                    cell_int_s.borrow_mut().bump_action("choose_int", int_owner);
                    Ok(answer)
                },
            )?,
        )?;

        let cell_dmg = &$cell;
        game.set(
            "damage",
            $scope.create_function_mut(move |_, (iid, n): (String, f32)| {
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

        // game.grant_extra_turn(pid) — push `pid` onto the extra-turn
        // queue. Consumed at end-of-turn instead of the default opponent
        // swap. Multiple grants stack in FIFO order.
        let cell_ext = &$cell;
        game.set(
            "grant_extra_turn",
            $scope.create_function_mut(move |_, pid_str: String| -> Result<()> {
                let pid = parse_pid(&pid_str)?;
                cell_ext.borrow_mut().extra_turns_pending.push(pid);
                Ok(())
            })?,
        )?;

        // game.schedule_return_at_next_main(iid) — queue `iid` for
        // return to its owner's BOARD at the start of the next main
        // phase (Main1 OR Main2 of any player's turn, whichever comes
        // first). Used by Cryogenic Chamber's on_die to thaw the held
        // creature later in the game. Silent no-op if `iid` isn't in
        // the card pool.
        let cell_ret = &$cell;
        game.set(
            "schedule_return_at_next_main",
            $scope.create_function_mut(move |_, iid: String| -> Result<()> {
                let mut s = cell_ret.borrow_mut();
                if !s.card_pool.contains_key(&iid) {
                    return Ok(());
                }
                s.pending_main_phase_returns.push(iid);
                Ok(())
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

        let cell_bot = &$cell;
        game.set(
            "deck_bottom",
            $scope.create_function_mut(move |_, pid_str: String| -> Result<Option<String>> {
                let pid = parse_pid(&pid_str)?;
                let s = cell_bot.borrow();
                Ok(s.player(pid).deck.last().cloned())
            })?,
        )?;

        // Move an iid from wherever it currently is onto the TOP of its
        // owner's deck (index 0 per V.1). Used by cantrips like Sprout
        // that pull from the bottom of the deck up to the top.
        let cell_mtd = &$cell;
        game.set(
            "move_to_deck_top",
            $scope.create_function_mut(move |_, iid: String| -> Result<()> {
                let mut s = cell_mtd.borrow_mut();
                let iid_owned = iid.clone();
                let inst = s.card_pool.get(&iid).ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "game.move_to_deck_top: card not in pool: {iid}"
                    ))
                })?;
                let owner = inst.owner;
                let controller = inst.controller;
                if let Some(from) = find_zone_of(&s, owner, &iid) {
                    s.remove_from_zone(&iid_owned, owner, from);
                } else if let Some(from) = find_zone_of(&s, controller, &iid) {
                    s.remove_from_zone(&iid_owned, controller, from);
                } else if let Some(host) = find_host_of_attached(&s, &iid) {
                    s.remove_attached(&host, &iid_owned);
                    s.set_face_down(&iid_owned, false);
                } else {
                    return Err(mlua::Error::runtime(format!(
                        "game.move_to_deck_top: card not in any zone: {iid}"
                    )));
                }
                s.add_to_zone_top(&iid_owned, owner, Zone::Deck);
                s.bump_action("move_to_deck_top", owner);
                Ok(())
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

        // game.set_intent("steal"|"donate"|"high_value_attached"|nil) —
        // side-channel hint declaring what the next choose_card call is
        // for. Consumed by the very next choose_card on this oracle.
        // Pass nil (or omit the call entirely) to clear/leave unset.
        let cell_intent = &$oracle_cell;
        game.set(
            "set_intent",
            $scope.create_function_mut(
                move |_, intent_str: Option<String>| -> Result<()> {
                    let intent = match intent_str.as_deref() {
                        None => None,
                        Some(s) => match TargetIntent::parse(s) {
                            Some(i) => Some(i),
                            None => {
                                return Err(mlua::Error::runtime(format!(
                                    "game.set_intent: unknown intent {s:?} (known: steal, donate, high_value_attached)"
                                )))
                            }
                        },
                    };
                    cell_intent.borrow_mut().set_next_intent(intent);
                    Ok(())
                },
            )?,
        )?;

        // game.x_value() — read the X value of the activation currently
        // resolving (RULES A.8 X-cost activations). Returns nil outside
        // of an X-cost activation. Used by handlers like dark-salamander
        // that scale their effect with the X paid for the activation.
        let cell_x = &$cell;
        game.set(
            "x_value",
            $scope.create_function_mut(move |_, _: ()| -> Result<Option<i32>> {
                Ok(cell_x.borrow().current_activation_x)
            })?,
        )?;

        // game.payment_ids() — read the iids that paid for the cast
        // currently resolving. Returns a table { hand=[...], attached=[...],
        // graveyard=[...], mill=[...] } when called from OnPlay, empty
        // lists otherwise. Used by handlers like read-the-embers that
        // count "red cards used to pay for this spell" — caller iterates
        // the lists and inspects `game.card(iid).colors`.
        let cell_pay = &$cell;
        game.set(
            "payment_ids",
            $scope.create_function_mut(move |lua, _: ()| -> Result<mlua::Table> {
                let s = cell_pay.borrow();
                let t = lua.create_table()?;
                match &s.current_cast_payments {
                    Some(p) => {
                        t.set("hand", lua.create_sequence_from(p.hand.clone())?)?;
                        t.set("attached", lua.create_sequence_from(p.attached.clone())?)?;
                        t.set("graveyard", lua.create_sequence_from(p.graveyard.clone())?)?;
                        t.set("mill", lua.create_sequence_from(p.mill.clone())?)?;
                        t.set("sacrifice", lua.create_sequence_from(p.sacrifice.clone())?)?;
                    }
                    None => {
                        t.set("hand", lua.create_sequence_from(Vec::<String>::new())?)?;
                        t.set("attached", lua.create_sequence_from(Vec::<String>::new())?)?;
                        t.set("graveyard", lua.create_sequence_from(Vec::<String>::new())?)?;
                        t.set("mill", lua.create_sequence_from(Vec::<String>::new())?)?;
                        t.set("sacrifice", lua.create_sequence_from(Vec::<String>::new())?)?;
                    }
                }
                Ok(t)
            })?,
        )?;

        // game.attached_of(iid) → list of attached iids. Pure read.
        let cell_att = &$cell;
        game.set(
            "attached_of",
            $scope.create_function_mut(
                move |lua, iid: String| -> Result<mlua::Table> {
                    let s = cell_att.borrow();
                    let list = s
                        .card_pool
                        .get(&iid)
                        .map(|inst| inst.attached.clone())
                        .unwrap_or_default();
                    lua.create_sequence_from(list)
                },
            )?,
        )?;

        // game.move_attached(from_host, to_host, iid) — detach `iid`
        // from `from_host`, attach to `to_host`. Powers shift-style
        // attached-card relocation. Both hosts must be in the card
        // pool; `iid` must currently be in `from_host`'s attached
        // list. Silent no-op on any precondition fail.
        let cell_mv = &$cell;
        game.set(
            "move_attached",
            $scope.create_function_mut(
                move |_, (from_host, to_host, iid): (String, String, String)| -> Result<()> {
                    let mut s = cell_mv.borrow_mut();
                    let in_from = s
                        .card_pool
                        .get(&from_host)
                        .map(|h| h.attached.iter().any(|a| a == &iid))
                        .unwrap_or(false);
                    let to_exists = s.card_pool.contains_key(&to_host);
                    if !in_from || !to_exists {
                        return Ok(());
                    }
                    let removed = s.remove_attached(&from_host, &iid);
                    if removed {
                        s.add_attached(&to_host, &iid);
                    }
                    Ok(())
                },
            )?,
        )?;

        // game.host_of(iid) → host iid or nil. Walks every card in the
        // pool and returns the first whose `attached` list contains
        // `iid`. Used by attached cards (mutations) that need to find
        // the host they're pinned to — e.g. an on_turn_begin handler
        // that wants to do something to the host.
        let cell_ho = &$cell;
        game.set(
            "host_of",
            $scope.create_function_mut(move |_, iid: String| -> Result<Option<String>> {
                let s = cell_ho.borrow();
                Ok(find_host_of_attached(&s, &iid))
            })?,
        )?;

        // game.attach_from_deck(host, player, n) — take top n cards of
        // `player`'s DECK and attach each to `host` face-down (P.17).
        // Respects C.14 — a transparent card is skipped if `host`
        // isn't transparent. Stops early if the deck runs out. Used by
        // beginning-of-turn "mill to attached" triggers (MYC, ...).
        let cell_afd = &$cell;
        game.set(
            "attach_from_deck",
            $scope.create_function_mut(
                move |_, (host, player, n): (String, String, i32)| -> Result<()> {
                    let pid = parse_pid(&player)?;
                    let mut s = cell_afd.borrow_mut();
                    if !s.card_pool.contains_key(&host) {
                        return Ok(());
                    }
                    let host_transparent = s.is_transparent(&host);
                    let take = n.max(0) as usize;
                    for _ in 0..take {
                        let Some(top) = s.player(pid).deck.first().cloned() else {
                            break;
                        };
                        if s.is_transparent(&top) && !host_transparent {
                            // C.14 violation — skip this card (it stays
                            // on top of the deck). Caller is responsible
                            // for choosing not to call this when a glass
                            // creature is at the top.
                            break;
                        }
                        let _ = s.remove_from_zone(&top, pid, Zone::Deck);
                        s.add_attached(&host, &top);
                        s.set_face_down(&top, true);
                    }
                    Ok(())
                },
            )?,
        )?;

        // game.attach(host, iid) — take `iid` from whatever zone it's
        // currently in (BOARD search across both players' boards) and
        // attach it to `host`, face-down per P.17. Used by predator
        // cards that destroy a creature "instead of it going to the
        // graveyard" and reroute it to attached. on_die does NOT fire
        // (this isn't a death, it's a redirected move). Silent no-op
        // when either id is missing from the card pool.
        let cell_at = &$cell;
        game.set(
            "attach",
            $scope.create_function_mut(
                move |_, (host, iid): (String, String)| -> Result<()> {
                    let mut s = cell_at.borrow_mut();
                    if !s.card_pool.contains_key(&host)
                        || !s.card_pool.contains_key(&iid)
                    {
                        return Ok(());
                    }
                    // C.14: transparent attachees can only attach to
                    // transparent hosts. Silent no-op when the rule
                    // would be violated (matches the existing
                    // silent-fail convention of this API).
                    if s.is_transparent(&iid) && !s.is_transparent(&host) {
                        return Ok(());
                    }
                    // Use the journaled `remove_from_zone` (instead of
                    // raw `board.retain`) so MCTS rollouts and the full-
                    // game rollback invariant test can reverse this
                    // mutation. The previous raw-retain implementation
                    // left the journal missing a `RemoveFromZone` entry
                    // for the card, which caused subsequent rollbacks
                    // to shrink the zone permanently and panic on later
                    // `insert(was_pos, …)` calls.
                    let owner_a = s.a.board.contains(&iid);
                    let owner_b = s.b.board.contains(&iid);
                    if owner_a {
                        s.remove_from_zone(&iid, PlayerId::A, Zone::Board);
                    }
                    if owner_b {
                        s.remove_from_zone(&iid, PlayerId::B, Zone::Board);
                    }
                    s.add_attached(&host, &iid);
                    s.set_face_down(&iid, true);
                    Ok(())
                },
            )?,
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
                    f32,
                    f32,
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
                        lua.create_sequence_from(s.effective_colors(&iid))?,
                    )?;
                    t.set(
                        "symbols",
                        lua.create_sequence_from(inst.card.symbols.clone())?,
                    )?;
                    t.set(
                        "face",
                        lua.create_sequence_from(s.effective_face(&iid))?,
                    )?;
                    t.set("tapped", inst.tapped)?;
                    t.set("face_down", inst.face_down)?;
                    t.set("attacked_this_turn", inst.attacked_this_turn)?;
                    t.set("owner", pid_to_str(inst.owner))?;
                    t.set("controller", pid_to_str(inst.controller))?;
                    let (x, y) = s.effective_stats(&iid);
                    t.set("x", x)?;
                    t.set("y", y)?;
                    // A.12: effective combined cost — sum across sources
                    // after every on-board cost-reduction static applies,
                    // per-source clamped to 0 (P.20). Routes through
                    // `effective_combined_cost` so handler-gates like
                    // "kill creature with combined cost ≥ N" observe the
                    // reduced value when the target sits under a
                    // cost-reduction static. is_x components contribute 0
                    // (their amount field is not the chosen X).
                    t.set("combined_cost", s.effective_combined_cost(&iid))?;
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
/// Extract a `ChoicePending` from an `mlua::Error` if the wrapper inside
/// `build_game_table!` raised it via `Error::external(pending)`. mlua
/// wraps caller errors in `CallbackError { cause, .. }` and external
/// errors in `ExternalError(Arc<dyn Error>)`; this walks both layers.
fn pending_from_mlua_error(e: &mlua::Error) -> Option<ChoicePending> {
    let mut cur = e;
    loop {
        match cur {
            mlua::Error::CallbackError { cause, .. } => cur = cause.as_ref(),
            mlua::Error::WithContext { cause, .. } => cur = cause.as_ref(),
            mlua::Error::ExternalError(inner) => {
                return inner.downcast_ref::<ChoicePending>().cloned();
            }
            _ => return None,
        }
    }
}

/// Walk an `mlua::Error`'s wrapper chain and render every layer's
/// human-readable message, joined by ` → `. The bare `format!("{e}")`
/// only shows the outermost wrapper (e.g. `"callback error: ..."`)
/// and HIDES the inner Lua line:message — which is exactly the piece
/// a card-author needs to fix their handler. ERROR.md slice 4 calls
/// this out as a wrong-diagnostic lie: outer wrapper text shown,
/// inner message hidden. This walker mirrors the `pending_from_mlua_error`
/// chain traversal but for the developer-visible `why` field.
///
/// For each layer:
///   - `CallbackError`: skipped (wrapper noise; cause has the real info).
///   - `WithContext { context, cause }`: emit `context`, descend into cause.
///   - `ExternalError`: render via `Display` and stop (we can't introspect
///     deeper; ChoicePending is the only `ExternalError` we expect at this
///     layer and it's already intercepted by `pending_from_mlua_error`).
///   - leaf variants (RuntimeError, SyntaxError, …): render via `Display`
///     and stop.
fn mlua_error_chain_why(e: &mlua::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = e;
    loop {
        match cur {
            mlua::Error::CallbackError { cause, .. } => {
                cur = cause.as_ref();
            }
            mlua::Error::WithContext { context, cause } => {
                parts.push(context.to_string());
                cur = cause.as_ref();
            }
            other => {
                // Leaf (or ExternalError): render via Display + stop.
                // `RuntimeError` carries the actual Lua line:message
                // string the card-author is looking for.
                parts.push(other.to_string());
                break;
            }
        }
    }
    if parts.is_empty() {
        // Defensive: shouldn't happen because the loop always pushes at
        // least once before breaking, but render the original error so
        // we never surface an empty `why`.
        e.to_string()
    } else {
        parts.join(" → ")
    }
}

/// Map a Lua handler-call result to a [`crate::trace::OutcomeRepr`]
/// for `TraceEvent::Handler`. ChoicePending is a Suspend (engine
/// catches and yields a HumanPrompt); everything else that isn't Ok
/// is an Err (real Lua crash — typo, nil deref, etc).
fn handler_outcome_from_lua_result(
    r: &mlua::Result<()>,
) -> crate::trace::OutcomeRepr {
    match r {
        Ok(()) => crate::trace::OutcomeRepr::Ok,
        Err(e) => {
            if let Some(pending) = pending_from_mlua_error(e) {
                crate::trace::OutcomeRepr::Suspend(format!("{pending:?}"))
            } else {
                crate::trace::OutcomeRepr::Err(e.to_string())
            }
        }
    }
}

#[allow(unused_must_use)] // build_game_table! macro expansion confuses the
                          // unused-must-use lint on the macro call site
                          // (see lua_api.rs:1330 etc). Macro returns
                          // `mlua::Table` at runtime — the 3 yield TDD
                          // tests + 442 lib tests verify the behavior.
pub(crate) fn fire_self_only(
    lua: &Lua,
    state: &mut GameState,
    oracle: &mut dyn ChoiceOracle,
    event: EventName,
    source: &InstanceId,
) -> std::result::Result<(), ChoicePending> {
    // Subtractive: Nonsense Mutation-style suppression evaporates the
    // host's own handlers. The suppressor itself is a separate iid
    // (in the host's attached list); it isn't its own host, so
    // host_loses_abilities returns false for it and its handlers
    // continue to fire.
    if state.host_loses_abilities(source) {
        return Ok(());
    }
    let Some(inst) = state.card_pool.get(source) else {
        return Ok(());
    };
    let Some(handler) = inst.card.handlers.get(&event).cloned() else {
        return Ok(());
    };
    let owner = inst.owner;
    let card_id = inst.card.id.clone();

    // O9: bracket the Lua scope with `Instant::now()` so the Handler
    // event records the handler's wall-clock cost. Cheap no-op when
    // trace is off.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    let state_cell = RefCell::new(&mut *state);
    let oracle_cell = RefCell::new(&mut *oracle);
    let result: Result<()> = lua.scope(|scope| {
        let game: mlua::Table = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        handler.call::<()>((game, self_table))?;
        Ok(())
    });

    if let Some(t0) = t0 {
        crate::trace::push(crate::trace::TraceEvent::Handler {
            at_us: crate::trace::now_us(),
            event: event.lua_key().to_string(),
            source: source.clone(),
            partner: None,
            duration_us: t0.elapsed().as_micros() as u64,
            outcome: handler_outcome_from_lua_result(&result),
        });
    }
    match result {
        Ok(()) => {
            credit_fire(state, event, owner);
            Ok(())
        }
        Err(e) => {
            if let Some(pending) = pending_from_mlua_error(&e) {
                // Handler suspended on a user-choice request. Engine
                // catches Pending up the stack and surfaces a
                // HumanPrompt; resume happens by replaying the answer
                // through HumanReplayOracle after a journal rollback.
                Err(pending)
            } else {
                let event_key = event.lua_key();
                // ERROR.md sweep: lua handler failure was eprintln-
                // and-continue → invisible from the dev tool. Now
                // also pushes a typed Error so the failure surfaces
                // at the next FFI yield with full context (which
                // card, which event, the Lua error message).
                crate::error::emit_region(
                    crate::error::Severity::Error,
                    "lua-handler",
                    event_key,
                    format!("Lua {event_key} handler for {card_id} failed"),
                    mlua_error_chain_why(&e),
                );
                eprintln!("[lua] {event_key} handler for {card_id} failed: {e}");
                Ok(())
            }
        }
    }
}

/// Fire an activated-ability handler. Same shape as fire_self_only
/// (handler takes `(game, self)`), but the handler is passed in by
/// reference rather than looked up by event name. Used by
/// `GameState::activate_ability` after cost has been paid. Per RULES
/// A.5 the effect resolves immediately and no response window opens.
#[allow(unused_must_use)] // see fire_self_only — build_game_table! lint quirk
pub(crate) fn fire_activated(
    lua: &Lua,
    state: &mut GameState,
    oracle: &mut dyn ChoiceOracle,
    source: &InstanceId,
    handler: mlua::Function,
) -> std::result::Result<(), ChoicePending> {
    let Some(inst) = state.card_pool.get(source) else {
        return Ok(());
    };
    let owner = inst.owner;
    let card_id = inst.card.id.clone();

    let state_cell = RefCell::new(&mut *state);
    let oracle_cell = RefCell::new(&mut *oracle);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        handler.call::<()>((game, self_table))?;
        Ok(())
    });

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Some(pending) = pending_from_mlua_error(&e) {
                Err(pending)
            } else {
                crate::error::emit_region(
                    crate::error::Severity::Error,
                    "lua-handler",
                    "activated",
                    format!("Lua activated handler for {card_id} failed"),
                    mlua_error_chain_why(&e),
                );
                eprintln!("[lua] activated handler for {card_id} failed: {e}");
                Ok(())
            }
        }
    }
}

/// Run an activated ability's `validate` hook. Same shape as
/// `fire_activated` — handler takes `(game, self)` — but the return
/// value is interpreted as a Lua truthy/falsy gate. Returns `false` on
/// any Lua error or explicit-false return; returns `true` only on an
/// explicit truthy return. RULES A.9: an activation may only be
/// initiated if its target requirements are satisfiable; validate is
/// the gate.
#[allow(unused_must_use)] // see fire_self_only — build_game_table! lint quirk
pub(crate) fn fire_validate(
    lua: &Lua,
    state: &mut GameState,
    oracle: &mut dyn ChoiceOracle,
    source: &InstanceId,
    handler: mlua::Function,
) -> bool {
    if !state.card_pool.contains_key(source) {
        return false;
    }
    let state_cell = RefCell::new(&mut *state);
    let oracle_cell = RefCell::new(&mut *oracle);
    let owner = state_cell.borrow().card_pool.get(source).map(|i| i.owner);
    let Some(owner) = owner else { return false };
    let result: Result<bool> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        let v: Value = handler.call((game, self_table))?;
        // Lua truthiness: nil and false are falsy, everything else truthy.
        Ok(!matches!(v, Value::Nil | Value::Boolean(false)))
    });
    let _ = state_cell;
    let _ = oracle_cell;
    result.unwrap_or(false)
}

/// Fire an event whose handler takes `(game, self, partner)`. Used for
/// `on_blocked_by` (self=attacker, partner=blocker) and `on_block`
/// (self=blocker, partner=attacker). Errors log and continue.
// Same design as fire_self_only: `OnBlock` / `OnBlockedBy` fire inline as
// part of resolving the block declaration. Stack carries the declaration
// itself (R.1.c), not the trigger.
#[allow(unused_must_use)] // see fire_self_only — build_game_table! lint quirk
pub(crate) fn fire_with_partner(
    lua: &Lua,
    state: &mut GameState,
    oracle: &mut dyn ChoiceOracle,
    event: EventName,
    source: &InstanceId,
    partner: &InstanceId,
) -> std::result::Result<(), ChoicePending> {
    let Some(inst) = state.card_pool.get(source) else {
        return Ok(());
    };
    let Some(handler) = inst.card.handlers.get(&event).cloned() else {
        return Ok(());
    };
    let owner = inst.owner;
    let card_id = inst.card.id.clone();

    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    let state_cell = RefCell::new(&mut *state);
    let oracle_cell = RefCell::new(&mut *oracle);
    let result: Result<()> = lua.scope(|scope| {
        let game = build_game_table!(lua, scope, state_cell, oracle_cell, owner);
        let self_table = build_self_table(lua, &state_cell.borrow(), source)?;
        let partner_table = build_self_table(lua, &state_cell.borrow(), partner)?;
        handler.call::<()>((game, self_table, partner_table))?;
        Ok(())
    });

    if let Some(t0) = t0 {
        crate::trace::push(crate::trace::TraceEvent::Handler {
            at_us: crate::trace::now_us(),
            event: event.lua_key().to_string(),
            source: source.clone(),
            partner: Some(partner.clone()),
            duration_us: t0.elapsed().as_micros() as u64,
            outcome: handler_outcome_from_lua_result(&result),
        });
    }
    match result {
        Ok(()) => {
            credit_fire(state, event, owner);
            Ok(())
        }
        Err(e) => {
            if let Some(pending) = pending_from_mlua_error(&e) {
                Err(pending)
            } else {
                let event_key = event.lua_key();
                crate::error::emit_region(
                    crate::error::Severity::Error,
                    "lua-handler",
                    event_key,
                    format!("Lua {event_key} handler for {card_id} (partner) failed"),
                    mlua_error_chain_why(&e),
                );
                eprintln!("[lua] {event_key} handler for {card_id} failed: {e}");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod suppress_tests {
    use super::*;
    use crate::card::{EventName, StaticAffects, StaticDef, StaticScope};
    use crate::choice::RandomOracle;
    use crate::game::test_helpers::deck_of;
    use rand::SeedableRng;

    /// Install an on_die handler that draws a card for the source's
    /// owner. Used to detect whether fire_self_only ran (hand grew) or
    /// was skipped (hand unchanged).
    fn install_draw_handler(lua: &Lua, state: &mut GameState, iid: &InstanceId) {
        let handler: mlua::Function = lua
            .load("return function(game, self) game.draw(self.owner, 1) end")
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(iid)
            .unwrap()
            .card
            .handlers
            .insert(EventName::OnDie, handler);
    }

    fn make_suppressor_static() -> StaticDef {
        StaticDef {
            affects: StaticAffects {
                subtypes: vec![],
                colors: vec![],
                controller: None,
                exclude_self: false,
                scope: StaticScope::AttachedHost,
                kind: None,
                has_keyword: None,
            },
            condition: None,
            effects: vec![crate::card::StaticEffect::SuppressesHostAbilities],
        }
    }

    fn install_noop_activation(lua: &Lua, state: &mut GameState, iid: &InstanceId) {
        use crate::card::{ActivatedAbility, Timing};
        let effect: mlua::Function = lua
            .load("return function(game, self) end")
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(iid)
            .unwrap()
            .card
            .activated
            .push(ActivatedAbility {
                cost_tap: true,
                cost_components: Vec::new(),
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
            });
    }

    #[test]
    fn activation_count_zero_when_host_abilities_suppressed() {
        let lua = Lua::new();
        let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
        let mutation = s.a.hand[0].clone();
        let host = s.a.hand[1].clone();
        install_noop_activation(&lua, &mut s, &host);
        s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(make_suppressor_static());
        s.a.hand.retain(|i| i != &mutation && i != &host);
        s.a.board.push(host.clone());

        // Sanity: host's printed activated ability is counted.
        assert_eq!(s.activation_count(&host), 1, "baseline: printed activated counts");

        // Attach suppressor; activation_count drops to 0.
        s.add_attached(&host, &mutation);
        assert_eq!(s.activation_count(&host), 0, "suppressed host has no activations");
    }

    #[test]
    fn chamber_on_die_schedules_attached_for_next_main_phase_return() {
        use crate::card::load_card;
        use crate::card::CardType;
        use crate::choice::RandomOracle;
        use rand::SeedableRng;
        use std::path::Path;

        let lua = Lua::new();
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let chamber_iid = s.a.hand[0].clone();
        let victim_iid = s.b.hand[0].clone();

        // Install loaded chamber.
        let chamber_cards =
            load_card(&lua, Path::new("cards/cryogenic-chamber.lua")).expect("load chamber");
        let chamber_card = chamber_cards
            .into_iter()
            .find(|c| c.id == "cryogenic-chamber")
            .unwrap();
        s.card_pool.get_mut(&chamber_iid).unwrap().card = chamber_card;
        s.card_pool.get_mut(&victim_iid).unwrap().card.kind = CardType::Creature;
        s.a.hand.retain(|i| i != &chamber_iid);
        s.b.hand.retain(|i| i != &victim_iid);
        s.a.board.push(chamber_iid.clone());
        // Simulate the prior ETB: victim is in chamber's attached list.
        s.add_attached(&chamber_iid, &victim_iid);

        // Fire on_die directly on the chamber (no oracle prompts needed).
        let mut oracle = RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0));
        fire_self_only(&lua, &mut s, &mut oracle, EventName::OnDie, &chamber_iid)
            .expect("RandomOracle answers locally, no Pending expected");

        assert!(
            s.pending_main_phase_returns.contains(&victim_iid),
            "victim must be queued for next-main-phase return"
        );
    }

    #[test]
    fn chamber_etb_handler_attaches_chosen_creature_to_chamber() {
        use crate::card::load_card;
        use crate::card::CardType;
        use crate::choice::{ScriptedAnswer, ScriptedOracle};
        use std::path::Path;

        let lua = Lua::new();
        let mut s = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let chamber_iid = s.a.hand[0].clone();
        let victim_iid = s.b.hand[0].clone();
        let distractor_iid = s.a.hand[1].clone();

        // Replace chamber_iid's card with the loaded chamber Card. The
        // chamber's on_enter_board handler is expected to pick a board
        // creature (via game.choose_card) and game.attach(self, target).
        let chamber_cards =
            load_card(&lua, Path::new("cards/cryogenic-chamber.lua")).expect("load chamber");
        let chamber_card = chamber_cards
            .into_iter()
            .find(|c| c.id == "cryogenic-chamber")
            .unwrap();
        s.card_pool.get_mut(&chamber_iid).unwrap().card = chamber_card;
        // Mark the two non-chamber cards as creatures so they're
        // eligible targets.
        s.card_pool.get_mut(&victim_iid).unwrap().card.kind = CardType::Creature;
        s.card_pool.get_mut(&distractor_iid).unwrap().card.kind = CardType::Creature;
        // Put chamber + creatures on board.
        s.a.hand.retain(|i| i != &chamber_iid && i != &distractor_iid);
        s.b.hand.retain(|i| i != &victim_iid);
        s.a.board.push(chamber_iid.clone());
        s.a.board.push(distractor_iid.clone());
        s.b.board.push(victim_iid.clone());

        // Oracle picks the opponent's creature (victim).
        let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Card(Some(victim_iid.clone()))]);
        fire_self_only(&lua, &mut s, &mut oracle, EventName::OnEnterBoard, &chamber_iid)
            .expect("scripted oracle answers locally, no Pending expected");

        let chamber_inst = s.card_pool.get(&chamber_iid).unwrap();
        assert!(
            chamber_inst.attached.contains(&victim_iid),
            "chamber must hold the chosen victim in its attached list"
        );
        assert!(
            !s.b.board.contains(&victim_iid),
            "victim must be removed from opponent's board"
        );
        // Distractor is unaffected.
        assert!(s.a.board.contains(&distractor_iid));
    }

    #[test]
    fn fire_self_only_skips_handler_when_host_abilities_suppressed() {
        let lua = Lua::new();
        let mut s = GameState::new(deck_of(20, "a"), deck_of(20, "b"));
        let mutation = s.a.hand[0].clone();
        let host = s.a.hand[1].clone();
        install_draw_handler(&lua, &mut s, &host);
        s.card_pool.get_mut(&mutation).unwrap().card.static_def = Some(make_suppressor_static());
        s.a.hand.retain(|i| i != &mutation && i != &host);
        s.a.board.push(host.clone());

        // Sanity: with no suppressor attached, firing the handler grows
        // the hand by 1 (the draw inside the handler).
        let hand_before = s.a.hand.len();
        let mut oracle = RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0));
        fire_self_only(&lua, &mut s, &mut oracle, EventName::OnDie, &host)
            .expect("RandomOracle answers locally, no Pending expected");
        assert_eq!(s.a.hand.len(), hand_before + 1, "baseline: handler must fire and draw");

        // Attach suppressor; fire again. Hand must NOT grow.
        s.add_attached(&host, &mutation);
        let hand_with_suppressor = s.a.hand.len();
        let mut oracle = RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0));
        fire_self_only(&lua, &mut s, &mut oracle, EventName::OnDie, &host)
            .expect("RandomOracle answers locally, no Pending expected");
        assert_eq!(
            s.a.hand.len(),
            hand_with_suppressor,
            "suppressed host's handler must not fire"
        );
    }
}

#[cfg(test)]
mod lua_yield_pending_tests {
    //! Lua-yield bug fix (LIMITATIONS.md ## lua).
    //!
    //! ## Why the coroutine approach is out
    //!
    //! mlua's `create_function` callbacks run as Lua C-calls. Lua refuses
    //! to `coroutine.yield` across a C-call boundary ("attempt to yield
    //! across a C-call boundary" — Lua 5.4 baseline behavior, not an mlua
    //! gap). Proven via a scratch test 2026-06-10; the LIMITATIONS.md spec
    //! suggesting coroutine yield was based on a false premise.
    //!
    //! ## The replay path that the StepEngine already implements for
    //! engine-driven choices
    //!
    //! Engine-driven prompts (PickCard, PickAttackers, etc.) already work
    //! by: open a preview journal; attempt the step; if `ChoicePending`
    //! propagates, surface it as a `HumanPrompt`; on user answer, ROLLBACK
    //! the journal + append the answer to `HumanReplayOracle.replay` +
    //! re-attempt from scratch. The re-attempt re-runs all side-effects
    //! deterministically (RNG state is restored, mutations are journaled
    //! and rolled back).
    //!
    //! The Lua-handler fix is to thread `ChoicePending` through the same
    //! plumbing: the `build_game_table!` wrappers raise Pending as a
    //! typed mlua external error carrying the `ChoicePending` value; the
    //! `fire_*` site catches that specific error, downcasts to
    //! `ChoicePending`, and returns it up the call stack. Every site that
    //! calls `fire_self_only` / `fire_with_partner` / `fire_activated`
    //! propagates the Pending up to `play_card` / `declare_attacker` /
    //! `activate_ability` (all of which already return
    //! `Result<_, ChoicePending>`). The StepEngine's existing
    //! rollback-and-replay machinery handles the rest.
    //!
    //! Idempotency requirement on Lua handlers: the same handler is
    //! re-fired from the start after each user choice, so any side effects
    //! before the choice run again under the rolled-back state. Since RNG
    //! and mutations are both journaled + rolled back, this is
    //! deterministic; handlers don't need to know they're being re-fired.

    use super::*;
    use crate::card::EventName;
    use crate::choice::{ChoicePending, RandomOracle, ScriptedAnswer};
    use crate::game::test_helpers::deck_of;
    use crate::game::{GameState, InstanceId};
    use crate::sim::human::HumanReplayOracle;
    use rand::SeedableRng;

    /// Install an `on_die` handler on `iid` that calls
    /// `game.choose_card(pool, ...)` then `game.damage(picked, 1)`. The
    /// pool is hard-coded to a single iid (target) so the test can
    /// assert exactly which card got picked.
    fn install_choose_then_damage_handler(
        lua: &Lua,
        state: &mut GameState,
        iid: &InstanceId,
        target_iid: &InstanceId,
    ) {
        let src = format!(
            r#"
            return function(game, self)
              local picked = game.choose_card({{ "{target}" }}, {{ prompt = "test pick" }})
              if picked ~= nil then
                game.damage(picked, 1)
              end
            end
            "#,
            target = target_iid,
        );
        let handler: mlua::Function = lua.load(&src).eval().unwrap();
        state
            .card_pool
            .get_mut(iid)
            .unwrap()
            .card
            .handlers
            .insert(EventName::OnDie, handler);
    }

    /// Build a fresh state with a host (A-side) that has a
    /// choose-then-damage on_die handler aimed at a target (B-side).
    fn setup_with_choose_handler() -> (Lua, GameState, InstanceId, InstanceId) {
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let host = state.a.hand[0].clone();
        let target = state.b.hand[0].clone();
        install_choose_then_damage_handler(&lua, &mut state, &host, &target);
        (lua, state, host, target)
    }

    /// **TDD failing test for the fix.** Today this errors via
    /// `mlua::Error::external("…ChoicePending…")` which `fire_self_only`
    /// swallows with `eprintln!` (the bug). After the fix, `fire_self_only`
    /// must return a `Result<(), ChoicePending>` so the caller can
    /// surface the pending choice up the stack.
    #[test]
    fn fire_self_only_returns_choice_pending_when_oracle_has_no_answer() {
        let (lua, mut state, host, _target) = setup_with_choose_handler();
        let mut oracle =
            HumanReplayOracle::new(RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)), Some(crate::game::PlayerId::A));

        // Empty replay → oracle returns ChoicePending on the first
        // choose_card call inside the handler.
        let result = fire_self_only(&lua, &mut state, &mut oracle, EventName::OnDie, &host);

        match result {
            Err(ChoicePending::Card(req)) => {
                assert!(
                    !req.pool.is_empty(),
                    "Pending::Card must carry the request pool back up"
                );
                assert_eq!(req.asker, Some(crate::game::PlayerId::A));
            }
            Err(other) => panic!("expected ChoicePending::Card, got {other:?}"),
            Ok(()) => panic!(
                "handler completed despite empty replay — the bug is that today \
                 the wrapper drops Pending; the fix must surface it as Err"
            ),
        }
    }

    /// After the fix: a scripted answer in the replay queue lets the
    /// handler complete normally, exercising the choose-then-act sequence
    /// end to end. Today this test ALSO fails because the wrapper's mlua
    /// error path runs before the replay is consulted by the engine.
    #[test]
    fn fire_self_only_completes_when_replay_supplies_answer() {
        let (lua, mut state, host, target) = setup_with_choose_handler();
        let mut oracle =
            HumanReplayOracle::new(RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)), Some(crate::game::PlayerId::A));
        oracle.reset_replay(vec![ScriptedAnswer::Card(Some(target.clone()))]);

        let result = fire_self_only(&lua, &mut state, &mut oracle, EventName::OnDie, &host);
        assert!(result.is_ok(), "handler must complete when answer is replayed: {result:?}");
        assert_eq!(
            state.card_pool[&target].damage, 1.0,
            "handler must have run game.damage(picked, 1) with the replayed iid"
        );
    }

    /// confirm() carries Pending the same way as choose_card.
    #[test]
    fn fire_self_only_returns_choice_pending_for_confirm_with_empty_replay() {
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let host = state.a.hand[0].clone();
        let handler: mlua::Function = lua
            .load(
                r#"return function(game, self)
                       local yes = game.confirm("yes or no?")
                       if yes then game.draw(self.owner, 1) end
                   end"#,
            )
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(&host)
            .unwrap()
            .card
            .handlers
            .insert(EventName::OnDie, handler);

        let mut oracle =
            HumanReplayOracle::new(RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)), Some(crate::game::PlayerId::A));

        let result = fire_self_only(&lua, &mut state, &mut oracle, EventName::OnDie, &host);
        match result {
            Err(ChoicePending::Confirm { asker, prompt }) => {
                assert_eq!(asker, crate::game::PlayerId::A);
                assert_eq!(prompt, "yes or no?");
            }
            other => panic!("expected ChoicePending::Confirm, got {other:?}"),
        }
    }

}

#[cfg(test)]
mod mlua_chain_walker_tests {
    //! Tests for `mlua_error_chain_why` — ERROR.md slice 4 fixed the
    //! "outer wrapper hides the inner Lua line:message" lie. These
    //! pin the contract: each layer of mlua's error chain renders
    //! exactly once, the `CallbackError` wrapper noise is stripped,
    //! and a runtime error's actionable line:message is preserved.

    use super::mlua_error_chain_why;
    use std::sync::Arc;

    #[test]
    fn leaf_runtime_error_renders_with_line_and_message_intact() {
        // The whole point of the chain walker is preserving exactly
        // this kind of leaf message — the card-author needs the line
        // number + the Lua reason to fix their handler.
        let e = mlua::Error::RuntimeError(
            "[string \"draw-two\"]:42: attempt to perform arithmetic on a nil value".to_string(),
        );
        let why = mlua_error_chain_why(&e);
        assert!(
            why.contains(":42:"),
            "leaf RuntimeError must preserve line number; got: {why}"
        );
        assert!(
            why.contains("attempt to perform arithmetic"),
            "leaf RuntimeError must preserve message; got: {why}"
        );
    }

    #[test]
    fn callback_error_wrapper_is_stripped_to_reveal_inner_runtime_error() {
        // This is the EXACT pattern that motivated the fix:
        // `format!("{e}")` on a CallbackError-wrapped RuntimeError
        // shows "callback error: ..." with the inner message hidden.
        // The walker must drill through and show the inner message
        // WITHOUT the wrapper noise.
        let inner = mlua::Error::RuntimeError(
            "[string \"goblin-scribe\"]:7: bad argument #1".to_string(),
        );
        let wrapped = mlua::Error::CallbackError {
            traceback: "irrelevant".to_string(),
            cause: Arc::new(inner),
        };
        let why = mlua_error_chain_why(&wrapped);
        assert!(
            why.contains(":7: bad argument #1"),
            "must surface inner RuntimeError message; got: {why}"
        );
        assert!(
            !why.contains("callback error"),
            "CallbackError wrapper noise must be stripped; got: {why}"
        );
    }

    #[test]
    fn with_context_layer_emits_its_context_and_descends_into_cause() {
        // WithContext carries a human-supplied context message; the
        // walker preserves it (joined to the cause via " → ") because
        // it usually names the call-site (e.g. "during on_play").
        let inner = mlua::Error::RuntimeError("nil arithmetic".to_string());
        let wrapped = mlua::Error::WithContext {
            context: "while running on_play".to_string(),
            cause: Arc::new(inner),
        };
        let why = mlua_error_chain_why(&wrapped);
        assert!(
            why.contains("while running on_play"),
            "WithContext context must be emitted; got: {why}"
        );
        assert!(
            why.contains("nil arithmetic"),
            "WithContext cause must be emitted; got: {why}"
        );
        assert!(
            why.contains(" → "),
            "layers must be joined with ' → '; got: {why}"
        );
    }

    #[test]
    fn callback_wrapping_context_wrapping_runtime_walks_full_chain() {
        // The realistic shape: handler raises a RuntimeError → mlua
        // adds context as it unwinds the Lua call → mlua wraps the
        // whole thing in CallbackError when it crosses back into Rust.
        // The walker must skip CallbackError, emit WithContext's
        // context, and emit the leaf RuntimeError.
        let inner = mlua::Error::RuntimeError(
            "[string \"draw-two\"]:3: assertion failed".to_string(),
        );
        let with_ctx = mlua::Error::WithContext {
            context: "on_play(self=draw-two-iid-1)".to_string(),
            cause: Arc::new(inner),
        };
        let cb = mlua::Error::CallbackError {
            traceback: "stack traceback: ...".to_string(),
            cause: Arc::new(with_ctx),
        };
        let why = mlua_error_chain_why(&cb);
        assert!(
            why.contains("on_play(self=draw-two-iid-1)"),
            "WithContext context must appear; got: {why}"
        );
        assert!(
            why.contains(":3: assertion failed"),
            "leaf RuntimeError line:message must appear; got: {why}"
        );
        // CallbackError contributes nothing; WithContext + leaf are
        // the two emitted layers → exactly one joiner.
        let arrow_count = why.matches(" → ").count();
        assert_eq!(
            arrow_count, 1,
            "expected exactly one ' → ' between two emitted layers; got: {why}",
        );
    }

    #[test]
    fn empty_chain_falls_back_to_display() {
        // Defensive: if the chain walker somehow produces an empty
        // parts vec (impossible with the current match arms — every
        // path either pushes or descends), we fall back to the
        // original error's Display so we never surface an empty `why`.
        // This pins the fallback. Using a SyntaxError to exercise the
        // "leaf renders via Display" arm with non-RuntimeError data.
        let e = mlua::Error::SyntaxError {
            message: "unexpected symbol near '='".to_string(),
            incomplete_input: false,
        };
        let why = mlua_error_chain_why(&e);
        assert!(
            !why.is_empty(),
            "non-empty why even for non-RuntimeError leaves; got: {why}"
        );
        assert!(
            why.contains("unexpected symbol"),
            "SyntaxError message must appear; got: {why}"
        );
    }
}
