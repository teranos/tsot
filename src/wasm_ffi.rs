//! FFI surface for the WASM frontend. All functions are `extern "C"`,
//! string-typed (via `*const c_char` / `*mut c_char`), and use JSON for
//! anything richer than a primitive.
//!
//! Browser JS calls these as:
//! ```js
//! const result = Module.ccall("tsot_hello", "string", [], []);
//! ```
//!
//! Memory model: returned strings are heap-allocated `CString`s,
//! leaked out via `into_raw()`. JS reads them via `Module.UTF8ToString`
//! and frees via `tsot_free_string(ptr)`.
//!
//! Execution model (S6): the session owns a live [`StepEngine`]; every
//! FFI call advances the engine one human-decision distance via
//! [`StepEngine::step`]. No threads, no channels, no `catch_unwind` —
//! works on wasm and native through the same code path.

// The `_impl` functions, session helpers and `GameSession` struct only
// have call sites in `#[cfg(test)]` and the `#[cfg(target_arch="wasm32")]`
// extern shims. Quiet the lib build's dead-code warnings rather than
// peppering each item with attributes.
#![allow(dead_code)]

use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::ffi::{c_char, CStr, CString};
use std::sync::Arc;

use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};
use crate::sim::step::{StepEngine, StepResult};

/// Wrap a raw FFI result value in the standard envelope shape
/// `{ok, result, log, trace, errors}` so EVERY call drains the
/// observability bus + sacred-error buffer back to JS — even
/// "one-shot" calls that historically returned bare JSON. Per
/// OBS O5b + ERROR Slice 4: without this, any error pushed inside
/// `build_preset_decks` / `build_card_pool_entries` / save / load
/// goes into the void, which is the canonical preset-mystery
/// failure mode that this whole arc exists to prevent.
fn wrap_result_envelope(result: serde_json::Value) -> Result<String, String> {
    let trace = crate::trace::drain();
    let errors = crate::error::drain();
    serde_json::to_string(&serde_json::json!({
        "ok": true,
        "result": result,
        "log": [],
        "trace": trace,
        "errors": errors,
    }))
    .map_err(|e| format!("wrap_result_envelope serialize: {e}"))
}

/// Deckbuilder FFI: serialized card pool, wrapped in envelope.
/// `result` field is a JSON array of card-pool entries:
/// `[{id, name, kind, cost_text, colors, symbols, subtypes, power,
/// toughness, timing, abilities}, …]`. Stable across the session;
/// JS calls once during bootstrap. Per OBS O5b the envelope also
/// carries `log + trace + errors` drained from this call.
pub(crate) fn tsot_list_card_pool_impl() -> Result<String, String> {
    use crate::card::CardRegistry;
    use crate::sim::deck_presets::build_card_pool_entries;
    use crate::sim::playable_pool::playable_pool;

    crate::trace::set_ffi_call_label("tsot_list_card_pool");
    let _ = crate::trace::drain();
    let _ = crate::error::drain();
    crate::trace::enable(true);

    let registry =
        CardRegistry::load_embedded().map_err(|e| format!("registry load: {e}"))?;
    let pool = playable_pool(registry.cards());
    let entries = build_card_pool_entries(&pool);
    let value = serde_json::to_value(&entries)
        .map_err(|e| format!("serialize card pool: {e}"))?;
    let envelope = wrap_result_envelope(value)?;
    crate::trace::clear_ffi_call_label();
    Ok(envelope)
}

/// Deckbuilder FFI: shipped preset decks (starter + variants),
/// wrapped in envelope. `result` field is a JSON array
/// `[{id, name, cards: [card_id…]}, …]` — same shape the start_game
/// FFI's `deck_a_ids` consumes. Per OBS O5b + ERROR Slice 5: this
/// call now validates that every card_id in every preset exists in
/// the playable pool and emits a typed Error per missing id (warn-
/// level — the preset still ships) so a corrupted preset doesn't
/// silently produce an invalid deck downstream.
pub(crate) fn tsot_list_preset_decks_impl() -> Result<String, String> {
    use crate::card::CardRegistry;
    use crate::sim::deck_presets::build_preset_decks;
    use crate::sim::playable_pool::playable_pool;
    use std::collections::BTreeSet;

    crate::trace::set_ffi_call_label("tsot_list_preset_decks");
    let _ = crate::trace::drain();
    let _ = crate::error::drain();
    crate::trace::enable(true);

    let registry =
        CardRegistry::load_embedded().map_err(|e| format!("registry load: {e}"))?;
    let pool = playable_pool(registry.cards());
    let presets = build_preset_decks(&pool);

    // ERROR Slice 5: validate each preset's card_ids against the
    // playable pool. A missing id means the corpus changed without
    // updating the hardcoded preset — the original preset-mystery
    // audit failure mode. Surface every miss as a typed Error so
    // the developer sees WHICH id in WHICH preset is broken.
    let playable_ids: BTreeSet<&str> =
        pool.iter().map(|c| c.id.as_str()).collect();
    for preset in &presets {
        for card_id in &preset.cards {
            if !playable_ids.contains(card_id.as_str()) {
                let title = format!(
                    "preset '{}' references unknown card id '{}'",
                    preset.id, card_id
                );
                let why = format!(
                    "card_id '{}' is not in playable pool ({} cards). \
                     Either the preset is stale or the corpus removed the card.",
                    card_id,
                    pool.len()
                );
                crate::error::emit_region(
                    crate::error::Severity::Warn,
                    "deckbuilder",
                    "preset-dropdown",
                    title,
                    why,
                );
            }
        }
    }

    let value = serde_json::to_value(&presets)
        .map_err(|e| format!("serialize presets: {e}"))?;
    let envelope = wrap_result_envelope(value)?;
    crate::trace::clear_ffi_call_label();
    Ok(envelope)
}

/// Preview-UCT FFI args. Defaults mirror `UctConfig::default()`.
#[derive(serde::Deserialize)]
pub(crate) struct PreviewUctArgs {
    #[serde(default = "default_preview_iterations")]
    pub iterations: u32,
    #[serde(default = "default_preview_exploration_c")]
    pub exploration_c: f64,
    #[serde(default = "default_preview_max_candidates")]
    pub max_candidates: u32,
}

fn default_preview_iterations() -> u32 {
    200
}

fn default_preview_exploration_c() -> f64 {
    std::f64::consts::SQRT_2
}

fn default_preview_max_candidates() -> u32 {
    8
}

/// Run UCT on a clone of the current session state and return a
/// ranked candidate list — the AI's belief about what the player at
/// the current prompt should do. Doesn't mutate the session: state is
/// cloned, UCT runs against the clone, the result is just data.
///
/// Use case: "what would the AI pick here?" hints in the prompt
/// panel; later, the input for the regret-report eval loop (per-pick
/// comparison of human choice vs UCT's choice).
///
/// Cancellation note: the `UCT_CANCEL_REQUESTED` flag is checked once
/// per iteration inside `pick_play_uct`. Because the wasm worker is
/// single-threaded, a JS-side `cancel_uct` message arriving while
/// this FFI is mid-call can't be processed until the current FFI
/// returns — so cancel only affects the NEXT search, not the
/// in-flight one. True mid-call cancellation would require
/// SharedArrayBuffer (COOP/COEP headers), out of scope here. Keep
/// `iterations` small enough that the wait is acceptable.
pub(crate) fn tsot_preview_uct_impl(args_json: &str) -> Result<String, String> {
    crate::trace::set_ffi_call_label("tsot_preview_uct");
    let args: PreviewUctArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("preview_uct: bad args JSON: {e}"))?;

    let json = with_session(|s| -> Result<String, String> {
        let mut state = s.engine.state.clone();
        let active = state.active_player;
        let cfg = crate::sim::uct::UctConfig {
            iterations: args.iterations,
            exploration_c: args.exploration_c,
            base_seed: (state.turn as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            max_candidates: args.max_candidates,
            ..Default::default()
        };
        let (picked, trace) = crate::sim::uct::pick_play_uct(
            &mut state,
            active,
            crate::sim::ai::PickKindFilter::Any,
            &cfg,
            &s.engine.registry,
        );
        let candidates: Vec<serde_json::Value> = trace
            .root
            .children
            .iter()
            .map(|c| {
                let win_rate = if c.visits > 0 {
                    c.wins / c.visits as f64
                } else {
                    0.0
                };
                serde_json::json!({
                    "iid": c.iid,
                    "visits": c.visits,
                    "wins": c.wins,
                    "win_rate": win_rate,
                })
            })
            .collect();
        let envelope = serde_json::json!({
            "ok": true,
            "asker": format!("{:?}", active),
            "chosen": picked,
            "candidates": candidates,
            "iterations_requested": args.iterations,
            "iterations_completed": trace.root.visits,
            "note": trace.note,
        });
        serde_json::to_string(&envelope).map_err(|e| format!("serialize preview: {e}"))
    })
    .map_err(|e| e.to_string())??;
    crate::trace::clear_ffi_call_label();
    Ok(json)
}

/// Set the UCT cancel flag from the JS side. Returns immediately;
/// the next iteration boundary inside any running `pick_play_uct`
/// will see the flag and break. See `tsot_preview_uct_impl` for the
/// single-threaded-worker caveat.
pub(crate) fn tsot_cancel_uct_impl() -> Result<String, String> {
    crate::sim::uct::request_uct_cancel();
    Ok("{\"ok\":true}".to_string())
}

/// Spectate-mode args. Both sides are AI — no `Human` AiKind, no
/// session install. The wasm runs the whole game in one ccall and
/// returns the full timeline of phase-boundary snapshots to JS, which
/// holds them in memory for the scrubber UI.
#[derive(serde::Deserialize)]
pub(crate) struct AutoGameArgs {
    pub seed: u64,
    pub deck_a_ids: Vec<String>,
    pub deck_b_ids: Vec<String>,
    /// `"heuristic"` / `"uct"` / `"mcts"`.
    pub ai_a: String,
    /// `"heuristic"` / `"uct"` / `"mcts"`.
    pub ai_b: String,
}

/// Run a both-AI game to completion and return phase-boundary state
/// snapshots so JS can render a scrubbable timeline of the game.
///
/// One snapshot is captured whenever any of `(turn, phase,
/// active_player)` changes — that's the granularity the scrubber UI
/// surfaces. With a 50-turn deckout cap and ~4 phase boundaries per
/// turn, the upper bound is ~200 snapshots × ~50–100 KB JSON each.
///
/// Both AIs must be non-Human; `NeedHuman` during this call indicates
/// a bug and returns an error envelope (it shouldn't happen — we
/// don't pass an `AiKind::Human` to the engine).
pub(crate) fn tsot_run_auto_game_impl(args_json: &str) -> Result<String, String> {
    use crate::card::CardRegistry;
    use crate::game::{GameState, Journal, PlayerId};
    use crate::sim::genome::{shuffle_deck, to_deck};
    use crate::sim::snapshot::build_state_view;
    use crate::sim::step::{StepEngine, StepResult};
    use crate::sim::AiKind;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    crate::trace::set_ffi_call_label("tsot_run_auto_game");
    crate::sim::uct::clear_uct_cancel();
    let _ = crate::trace::drain();
    crate::trace::enable(true);

    let args: AutoGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("auto_game: bad args JSON: {e}"))?;

    fn parse_auto_ai(name: &str, seed: u64) -> Result<AiKind, String> {
        Ok(match name {
            "heuristic" => AiKind::Heuristic,
            "mcts" => AiKind::Mcts(crate::sim::mcts::MctsConfig {
                base_seed: seed,
                ..Default::default()
            }),
            "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
                base_seed: seed,
                ..Default::default()
            }),
            other => return Err(format!("auto_game: unknown ai {other:?}")),
        })
    }
    let ai_a = parse_auto_ai(&args.ai_a, args.seed.wrapping_add(0xA0_C0_FF_EE))?;
    let ai_b = parse_auto_ai(&args.ai_b, args.seed.wrapping_add(0xB0_C0_FF_EE))?;

    let registry =
        CardRegistry::load_embedded().map_err(|e| format!("registry load: {e}"))?;
    let mut deck_a = to_deck(&registry, &args.deck_a_ids)
        .map_err(|e| format!("auto_game: deck A: {e:?}"))?;
    let mut deck_b = to_deck(&registry, &args.deck_b_ids)
        .map_err(|e| format!("auto_game: deck B: {e:?}"))?;
    let mut rng_a = StdRng::seed_from_u64(args.seed.wrapping_add(0xA000_A000));
    let mut rng_b = StdRng::seed_from_u64(args.seed.wrapping_add(0xB000_B000));
    shuffle_deck(&mut deck_a, &mut rng_a);
    shuffle_deck(&mut deck_b, &mut rng_b);
    let mut state = GameState::new(deck_a, deck_b);
    state.replay_journal = Some(Journal::new());

    let ais = [ai_a, ai_b];
    let mut engine = StepEngine::new(state, ais, registry, args.seed);

    let viewer = PlayerId::A;
    let mut snapshots: Vec<serde_json::Value> = Vec::new();
    let snap = |engine: &StepEngine| {
        serde_json::json!({
            "turn": engine.state.turn,
            "phase": format!("{:?}", engine.state.phase),
            "active_player": engine.state.active_player,
            "state": build_state_view(&engine.state, viewer),
        })
    };
    let key_of = |engine: &StepEngine| {
        (
            engine.state.turn,
            format!("{:?}", engine.state.phase),
            engine.state.active_player,
        )
    };

    // Initial snapshot before any step runs.
    snapshots.push(snap(&engine));
    let mut last_key = key_of(&engine);

    // Defensive step budget — the 50-turn deckout cap should bound
    // this naturally, but UCT/MCTS rollouts inside step() could be
    // long; bail loudly rather than spin.
    let mut budget = 200_000u32;
    let winner_id: Option<PlayerId> = loop {
        budget = budget
            .checked_sub(1)
            .ok_or_else(|| "auto_game: step budget exhausted".to_string())?;
        match engine.step(None) {
            StepResult::Continue => {
                let key = key_of(&engine);
                if key != last_key {
                    snapshots.push(snap(&engine));
                    last_key = key;
                }
            }
            StepResult::NeedHuman(_) => {
                return Err(
                    "auto_game: engine yielded NeedHuman — one of the AIs is Human"
                        .to_string(),
                );
            }
            StepResult::Done(stats) => break Some(stats.winner),
        }
    };

    // Always append the final state so the scrubber can land on
    // "GameOver" even when the final step didn't change phase.
    snapshots.push(snap(&engine));

    let log = std::mem::take(&mut engine.log);
    let trace_events = crate::trace::drain();
    let errors = crate::error::drain();
    let envelope = serde_json::json!({
        "ok": true,
        "snapshots": snapshots,
        "winner": winner_id,
        "total_turns": engine.state.turn,
        "log": log,
        "trace": trace_events,
        "errors": errors,
    });
    let json =
        serde_json::to_string(&envelope).map_err(|e| format!("auto_game: serialize: {e}"))?;
    crate::trace::clear_ffi_call_label();
    Ok(json)
}

/// Save the current session's game state + cursor as a JSON
/// `SaveFile`. Caller is expected to be in the middle of a game
/// (i.e., `tsot_start_game` has been called); returns Err if no
/// session is active. The save captures `GameState` + the
/// `EngineCursor` so the load path can place the engine at the
/// same decision point. It does NOT capture the RNG state or the
/// opponent AI — those are reconstructed at load time from the
/// load_game args + a fresh seed.
pub(crate) fn tsot_save_game_impl() -> Result<String, String> {
    use crate::replay::SaveFile;
    crate::trace::set_ffi_call_label("tsot_save_game");
    let _ = crate::trace::drain();
    let _ = crate::error::drain();
    crate::trace::enable(true);
    let save_json = with_session(|s| -> Result<String, String> {
        let save = SaveFile::from_step_engine(&s.engine, 0);
        save.to_json().map_err(|e| format!("save: {e}"))
    })
    .map_err(|e| e.to_string())??;
    // Reparse the SaveFile JSON so it embeds as the structured
    // `result` value in the envelope rather than as an opaque
    // escaped string. JS callers wanting raw can re-stringify.
    let value: serde_json::Value = serde_json::from_str(&save_json)
        .map_err(|e| format!("reparse save_json: {e}"))?;
    let envelope = wrap_result_envelope(value)?;
    crate::trace::clear_ffi_call_label();
    Ok(envelope)
}

/// Load-game args: the `SaveFile` JSON plus the AI to drive the
/// opponent on resume. Human is always side A in v1; the save
/// doesn't remember what AI played opposite — the caller picks.
#[derive(serde::Deserialize)]
pub(crate) struct LoadGameArgs {
    pub save_json: String,
    /// `"heuristic"` / `"uct"` / `"mcts"`.
    pub opp_ai: String,
    /// Fresh seed for the post-load engine RNG. Save doesn't preserve
    /// the original engine seed (StdRng's state isn't serialized), so
    /// AI rollouts after a load won't be byte-identical to a
    /// continuous play — but the loaded position itself is exact.
    pub seed: u64,
}

/// Install a session from a `SaveFile` JSON. Returns the envelope
/// the UI would have received from the next-prompt-after-resume
/// (matches `tsot_start_game`'s contract), so JS can re-render
/// directly without an extra round-trip.
pub(crate) fn tsot_load_game_impl(args_json: &str) -> Result<String, String> {
    use crate::card::CardRegistry;
    use crate::replay::SaveFile;
    use crate::sim::step::StepEngine;
    use crate::sim::AiKind;

    crate::trace::set_ffi_call_label("tsot_load_game");
    crate::sim::uct::clear_uct_cancel();
    let args: LoadGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("load_game: bad args JSON: {e}"))?;

    let _ = clear_session();
    let _ = crate::trace::drain();
    crate::trace::enable(true);

    // Per OBS O5b: tsot_save_game now returns the wrapped envelope
    // shape `{ok, result, log, trace, errors}`. Callers that pass
    // that envelope through unchanged would otherwise fail SaveFile
    // parse. Detect the envelope shape and extract the inner result
    // before handing to SaveFile::from_json. Legacy raw-SaveFile-JSON
    // callers continue to work unchanged.
    let load_input = match serde_json::from_str::<serde_json::Value>(&args.save_json) {
        Ok(v) if v.get("ok").is_some() && v.get("result").is_some() => {
            serde_json::to_string(&v["result"])
                .unwrap_or_else(|_| args.save_json.clone())
        }
        _ => args.save_json.clone(),
    };
    let save = SaveFile::from_json(&load_input)
        .map_err(|e| format!("load_game[parse SaveFile]: {e}"))?;
    let cursor_opt = save.cursor.clone();
    let registry = CardRegistry::load_embedded()
        .map_err(|e| format!("load_game[registry load]: {e}"))?;
    let state = save
        .restore(&registry)
        .map_err(|e| format!("load_game[rebind handlers]: {e}"))?;

    let opp = match args.opp_ai.as_str() {
        "heuristic" => AiKind::Heuristic,
        "mcts" => AiKind::Mcts(crate::sim::mcts::MctsConfig {
            base_seed: args.seed.wrapping_add(0xCAFE_BABE),
            ..Default::default()
        }),
        "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
            base_seed: args.seed.wrapping_add(0x00C0_FFEE),
            ..Default::default()
        }),
        other => return Err(format!("load_game: unknown opp_ai {other:?}")),
    };
    let (iface, _prompt_rx, _action_tx) = HumanInterface::new();
    let ais = [AiKind::Human(Arc::new(iface)), opp];
    let mut engine = StepEngine::new(state, ais, registry, args.seed);
    // Apply the saved cursor so the engine resumes at the exact
    // decision point. If the save predates cursor-aware FFI (cursor
    // is None), leave engine.cursor at its default StartTurn — the
    // engine will then re-enter turn-setup, which may re-run untap/
    // draw on a state already past those phases. That's the legacy
    // SaveFile shape; documented in replay.rs.
    if let Some(cursor) = cursor_opt {
        engine.cursor = cursor;
    }

    let mut session = GameSession { engine };
    let prompt = drive_to_next_yield(&mut session.engine, None)
        .map_err(|e| format!("load_game[drive_to_next_yield]: {e}"))?;
    let log = std::mem::take(&mut session.engine.log);
    let trace_events = crate::trace::drain();
    let errors = crate::error::drain();
    let envelope = serde_json::json!({
        "prompt": prompt,
        "log": log,
        "trace": trace_events,
        "errors": errors,
    });
    let envelope_json = serde_json::to_string(&envelope)
        .map_err(|e| format!("load_game[serialize envelope]: {e}"))?;
    install_session(session);
    crate::trace::clear_ffi_call_label();
    Ok(envelope_json)
}

/// Live game session — one per browser tab. Owns the [`StepEngine`]
/// that drives the game; each FFI call resumes the engine where it
/// left off (no save-and-replay, no rebuild-per-step).
pub(crate) struct GameSession {
    pub engine: StepEngine,
}

thread_local! {
    /// Single-tab single-game slot. `None` before `tsot_start_game`,
    /// `Some(_)` for the duration of a game, dropped + reset to
    /// `None` on `tsot_end_game` or before a fresh `tsot_start_game`.
    pub(crate) static SESSION: RefCell<Option<GameSession>> = const { RefCell::new(None) };
}

/// Borrow the live session, run a closure on it, return the result.
/// Returns `Err(...)` if no session is active so callers can surface
/// "no game in progress" cleanly instead of panicking.
pub(crate) fn with_session<R, F: FnOnce(&mut GameSession) -> R>(f: F) -> Result<R, &'static str> {
    SESSION.with(|cell| {
        let mut borrow = cell.borrow_mut();
        match borrow.as_mut() {
            Some(s) => Ok(f(s)),
            None => Err("no game in progress (call tsot_start_game first)"),
        }
    })
}

/// Install a new session, dropping any previous one.
pub(crate) fn install_session(session: GameSession) {
    SESSION.with(|cell| {
        *cell.borrow_mut() = Some(session);
    });
}

/// Tear down the current session (drop everything). Returns true if
/// a session was active, false if there was nothing to tear down.
pub(crate) fn clear_session() -> bool {
    SESSION.with(|cell| cell.borrow_mut().take().is_some())
}

/// Args accepted by [`tsot_start_game_impl`]. JSON-encoded by the JS
/// caller. `opp_ai` is one of "heuristic" / "mcts" / "uct"; the
/// human always plays side A in v1.
#[derive(Clone, serde::Deserialize)]
pub(crate) struct StartGameArgs {
    seed: u64,
    deck_a_ids: Vec<String>,
    deck_b_ids: Vec<String>,
    opp_ai: String,
}

/// Build a fresh engine from `args`, drive it to the first human
/// decision, install the session, return the prompt JSON.
pub(crate) fn tsot_start_game_impl(args_json: &str) -> Result<String, String> {
    crate::trace::set_ffi_call_label("tsot_start_game");
    crate::sim::uct::clear_uct_cancel();
    let args: StartGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("tsot_start_game: bad args JSON: {e}"))?;

    let _ = clear_session();

    // O5: enable the trace bus for the duration of this FFI call.
    // Any stale events from a previous call get cleared so the
    // envelope carries only this call's slice.
    let _ = crate::trace::drain();
    crate::trace::enable(true);

    let engine = build_engine(&args)?;
    let mut session = GameSession { engine };
    let prompt = drive_to_next_yield(&mut session.engine, None)?;
    // Drain the engine log into the envelope so the JS LOG panel can
    // surface every internal decision (card picks, attackers, blocks,
    // UCT trace ASCII tree) without devtools. The buffer is cleared
    // after every yield so JS sees only the lines since the last call.
    let log = std::mem::take(&mut session.engine.log);
    let trace_events = crate::trace::drain();
    let errors = crate::error::drain();
    let envelope = serde_json::json!({
        "prompt": prompt,
        "log": log,
        "trace": trace_events,
        "errors": errors,
    });
    let envelope_json =
        serde_json::to_string(&envelope).map_err(|e| format!("serialize first prompt: {e}"))?;
    install_session(session);
    crate::trace::clear_ffi_call_label();
    Ok(envelope_json)
}

/// Submit a HumanAction. The engine resumes with the supplied action,
/// drives forward until the next NeedHuman / Done, returns the prompt
/// JSON.
pub(crate) fn tsot_apply_action_impl(action_json: &str) -> Result<String, String> {
    crate::trace::set_ffi_call_label("tsot_apply_action");
    crate::sim::uct::clear_uct_cancel();
    let action: HumanAction = serde_json::from_str(action_json)
        .map_err(|e| format!("tsot_apply_action: bad action JSON: {e}"))?;

    // O5: fresh trace slice per FFI call. Drain any leftover events
    // (from the start_game call or a panicked previous call) so this
    // envelope's `trace` carries only this apply_action's work.
    let _ = crate::trace::drain();
    crate::trace::enable(true);

    let (prompt, log) = with_session(|s| -> Result<_, String> {
        let prompt = drive_to_next_yield(&mut s.engine, Some(action))?;
        let log = std::mem::take(&mut s.engine.log);
        Ok((prompt, log))
    })
    .map_err(|e| e.to_string())??;
    let trace_events = crate::trace::drain();
    let errors = crate::error::drain();
    let envelope = serde_json::json!({
        "prompt": prompt,
        "log": log,
        "trace": trace_events,
        "errors": errors,
    });
    let json = serde_json::to_string(&envelope)
        .map_err(|e| format!("serialize next prompt: {e}"))?;
    crate::trace::clear_ffi_call_label();
    Ok(json)
}

/// Construct the engine from the JSON args. Card registry is rebuilt
/// each `start_game` (cheap for v1; cache if it ever shows up in a
/// profile).
fn build_engine(args: &StartGameArgs) -> Result<StepEngine, String> {
    use crate::card::CardRegistry;
    use crate::game::GameState;
    use crate::sim::genome::{shuffle_deck, to_deck};
    use crate::sim::AiKind;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    let registry = CardRegistry::load_embedded().map_err(|e| format!("registry load: {e}"))?;
    let mut deck_a =
        to_deck(&registry, &args.deck_a_ids).map_err(|e| format!("deck A rebuild: {e:?}"))?;
    let mut deck_b =
        to_deck(&registry, &args.deck_b_ids).map_err(|e| format!("deck B rebuild: {e:?}"))?;
    // RULES S.0: shuffle each deck before drawing the opening 5.
    // Per-deck seed derived from `args.seed` so A and B shuffle
    // independently but the whole game is replayable from one seed.
    let mut rng_a = StdRng::seed_from_u64(args.seed.wrapping_add(0xA000_A000));
    let mut rng_b = StdRng::seed_from_u64(args.seed.wrapping_add(0xB000_B000));
    shuffle_deck(&mut deck_a, &mut rng_a);
    shuffle_deck(&mut deck_b, &mut rng_b);
    let mut state = GameState::new(deck_a, deck_b);
    state.replay_journal = Some(crate::game::Journal::new());

    let (iface, _prompt_rx, _action_tx) = HumanInterface::new();
    let iface = Arc::new(iface);

    let opp = match args.opp_ai.as_str() {
        "heuristic" => AiKind::Heuristic,
        "mcts" => AiKind::Mcts(crate::sim::mcts::MctsConfig {
            base_seed: args.seed.wrapping_add(0xCAFE_BABE),
            ..Default::default()
        }),
        "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
            base_seed: args.seed.wrapping_add(0x00C0_FFEE),
            ..Default::default()
        }),
        other => return Err(format!("unknown opp_ai {other:?}")),
    };
    let ais = [AiKind::Human(iface), opp];
    Ok(StepEngine::new(state, ais, registry, args.seed))
}

/// Drive the engine until it yields a `NeedHuman` or signals `Done`.
/// On `Done`, synthesize a `HumanPrompt::GameOver` so the frontend has
/// a single uniform return type.
fn drive_to_next_yield(
    engine: &mut StepEngine,
    first_pending: Option<HumanAction>,
) -> Result<HumanPrompt, String> {
    use crate::game::PlayerId;
    use crate::sim::snapshot::build_state_view;

    let mut pending = first_pending;
    let mut budget = 100_000u32;
    loop {
        budget = budget
            .checked_sub(1)
            .ok_or_else(|| "drive_to_next_yield: step budget exhausted".to_string())?;
        match engine.step(pending.take()) {
            StepResult::Continue => {}
            StepResult::NeedHuman(p) => return Ok(*p),
            StepResult::Done(_) => {
                // Human is side A in v1; synthesize a GameOver prompt
                // with A's view of the final state so the frontend can
                // render the result without a second FFI round-trip.
                let view = build_state_view(&engine.state, PlayerId::A);
                return Ok(HumanPrompt::GameOver {
                    state: view,
                    winner: engine.state.winner,
                });
            }
        }
    }
}

// Errors-as-first-class envelope. When an `_impl` returns Err, we
// push a `TraceEvent::Error` into the bus (so the breadcrumb trail
// is preserved), drain the bus, and return a JSON envelope shaped
// `{ok:false, error, trace: [...]}`. JS sees the same trace array
// shape it sees on success, with the Error event included, and the
// LOG renders it through the existing `appendTrace` path. No
// "error: …" prefixed strings, no silent suppression.
pub(crate) fn err_envelope(stage: Option<&str>, message: &str) -> String {
    crate::trace::emit_error("rust-ffi", stage, message);
    // Sacred-error pipeline (ERROR.md Slice 4): also push a typed
    // Error so the JS-side dispatcher routes through tsotPushError
    // and the overlay anchors at the click cursor.
    let title = stage
        .map(|s| format!("Rust FFI failed: {s}"))
        .unwrap_or_else(|| "Rust FFI failed".to_string());
    match stage {
        Some(s) => {
            crate::error::emit_region(
                crate::error::Severity::Error,
                "rust-ffi",
                s,
                title,
                message.to_string(),
            );
        }
        None => {
            crate::error::emit(
                crate::error::Severity::Error,
                "rust-ffi",
                title,
                message.to_string(),
            );
        }
    }
    let trace = crate::trace::drain();
    let errors = crate::error::drain();
    serde_json::to_string(&serde_json::json!({
        "ok": false,
        "error": message,
        "trace": trace,
        "errors": errors,
    }))
    .unwrap_or_else(|_| {
        "{\"ok\":false,\"error\":\"<err_envelope serialize failed>\",\"trace\":[],\"errors\":[]}"
            .to_string()
    })
}

// Below: wasm-only FFI exports. The session-management primitives
// above compile + test on every target.
#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::*;

    /// Allocate a `CString` and return its raw pointer. Caller is
    /// responsible for calling [`tsot_free_string`] to free the memory.
    fn export(s: impl Into<Vec<u8>>) -> *mut c_char {
        CString::new(s).unwrap_or_default().into_raw()
    }

    /// Free a string previously returned by an FFI function. JS calls
    /// this once it's done with the string.
    ///
    /// # Safety
    /// `ptr` must be a pointer previously returned from one of this
    /// module's FFI functions, or `null`. Calling with any other pointer
    /// is undefined behavior.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_free_string(ptr: *mut c_char) {
        if ptr.is_null() {
            return;
        }
        drop(unsafe { CString::from_raw(ptr) });
    }

    /// Smoke-test export. Returns a static greeting so JS can verify
    /// the wasm is loaded and the FFI boundary works.
    #[no_mangle]
    pub extern "C" fn tsot_hello() -> *mut c_char {
        export(format!("tsot wasm alive (build {})", env!("CARGO_PKG_VERSION")))
    }

    /// Drain the partial trace buffer mid-flight. Called by JS while
    /// awaiting `tsot_apply_action` (with `async: true`) — JS polls
    /// this on a `setInterval` so UCT iteration events render in
    /// the LOG as they're emitted, not at the end of the FFI call.
    /// Returns a JSON array (possibly empty). Always freed via
    /// `tsot_free_string`.
    #[no_mangle]
    pub extern "C" fn tsot_drain_partial_trace() -> *mut c_char {
        let events = crate::trace::drain();
        let json = serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string());
        export(json)
    }

    /// Echo a string back through the FFI. Used to verify input
    /// handling before wiring real game APIs.
    ///
    /// # Safety
    /// `input` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_echo(input: *const c_char) -> *mut c_char {
        if input.is_null() {
            return export("");
        }
        let s = unsafe { CStr::from_ptr(input) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        export(format!("echo: {s}"))
    }

    /// Start a new game. JSON args: `{seed, deck_a_ids, deck_b_ids,
    /// opp_ai}`. Returns first HumanPrompt as JSON, or an error
    /// string starting with `"error: "`. Free the returned pointer
    /// with [`tsot_free_string`] when done.
    ///
    /// # Safety
    /// `args` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_start_game(args: *const c_char) -> *mut c_char {
        if args.is_null() {
            return export("error: null args");
        }
        let s = unsafe { CStr::from_ptr(args) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        match super::tsot_start_game_impl(s) {
            Ok(prompt) => export(prompt),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Submit a HumanAction. JSON shape per `HumanAction`'s
    /// internally-tagged serde format. Returns next HumanPrompt as
    /// JSON, or `"error: …"`. Free with [`tsot_free_string`].
    ///
    /// # Safety
    /// `action` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_apply_action(action: *const c_char) -> *mut c_char {
        if action.is_null() {
            return export("error: null action");
        }
        let s = unsafe { CStr::from_ptr(action) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        match super::tsot_apply_action_impl(s) {
            Ok(prompt) => export(prompt),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Deckbuilder: full playable card pool as JSON. One-shot, no
    /// session needed. Free with [`tsot_free_string`].
    #[no_mangle]
    pub extern "C" fn tsot_list_card_pool() -> *mut c_char {
        match super::tsot_list_card_pool_impl() {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Deckbuilder: shipped preset decks (starter + 7 gauntlet
    /// variants) as JSON. One-shot, no session needed. Free with
    /// [`tsot_free_string`].
    #[no_mangle]
    pub extern "C" fn tsot_list_preset_decks() -> *mut c_char {
        match super::tsot_list_preset_decks_impl() {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Save the current session as a JSON `SaveFile`. Returns
    /// `"error: …"` if no session is active. Free with
    /// [`tsot_free_string`].
    #[no_mangle]
    pub extern "C" fn tsot_save_game() -> *mut c_char {
        match super::tsot_save_game_impl() {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Replace the session with one restored from a JSON `SaveFile`.
    /// JSON args: `{save_json, opp_ai, seed}`. Returns the envelope
    /// the next-prompt-after-resume produces (matches start_game's
    /// contract). Free with [`tsot_free_string`].
    ///
    /// # Safety
    /// `args` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_load_game(args: *const c_char) -> *mut c_char {
        if args.is_null() {
            return export("error: null args");
        }
        let s = unsafe { CStr::from_ptr(args) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        match super::tsot_load_game_impl(s) {
            Ok(prompt) => export(prompt),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Observability probe: panic on purpose. If the panic hook is
    /// installed and works through emscripten's trap path, the LOG
    /// shows a rich `[RUST-PANIC]` block with file:line. If it
    /// doesn't, we see an opaque `[WASM-TRAP]` — telling us the
    /// hook isn't reaching the JS side. One-click diagnostic.
    #[no_mangle]
    pub extern "C" fn tsot_test_panic() -> *mut c_char {
        crate::trace::set_ffi_call_label("tsot_test_panic");
        panic!("tsot_test_panic: intentional panic from the FFI surface");
    }

    /// Run UCT on a clone of the current session state and return a
    /// ranked candidate envelope. JSON args:
    /// `{iterations, exploration_c, max_candidates}` (all optional;
    /// defaults from `PreviewUctArgs`). Returns the envelope JSON.
    ///
    /// # Safety
    /// `args` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_preview_uct(args: *const c_char) -> *mut c_char {
        if args.is_null() {
            return export("error: null args");
        }
        let s = unsafe { CStr::from_ptr(args) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        match super::tsot_preview_uct_impl(s) {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Request UCT cancellation. The next iteration boundary in any
    /// running `pick_play_uct` checks the flag and breaks. Single-
    /// threaded-worker caveat applies (see `tsot_preview_uct_impl`).
    #[no_mangle]
    pub extern "C" fn tsot_cancel_uct() -> *mut c_char {
        match super::tsot_cancel_uct_impl() {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }

    /// Spectator entry point: run a both-AI game to completion and
    /// return the timeline of phase-boundary snapshots so JS can
    /// render the scrubber. JSON args:
    /// `{seed, deck_a_ids, deck_b_ids, ai_a, ai_b}`.
    ///
    /// # Safety
    /// `args` must be a valid pointer to a null-terminated UTF-8 string.
    #[no_mangle]
    pub unsafe extern "C" fn tsot_run_auto_game(args: *const c_char) -> *mut c_char {
        if args.is_null() {
            return export("error: null args");
        }
        let s = unsafe { CStr::from_ptr(args) }
            .to_str()
            .unwrap_or("<invalid utf-8>");
        match super::tsot_run_auto_game_impl(s) {
            Ok(json) => export(json),
            Err(e) => export(super::err_envelope(None, &e)),
        }
    }
}

// Re-export so the wasm bin can reach them through `tsot::wasm_ffi::tsot_*`.
#[cfg(target_arch = "wasm32")]
pub use wasm_exports::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardRegistry;
    use serde_json::Value;

    /// Pick a vanilla creature with hand/mill-only cost: this template
    /// is castable on turn 1 from the opening hand without triggering
    /// any choice-oracle paths (which still block on `HumanInterface`
    /// until S7 lands).
    fn vanilla_template() -> crate::card::Card {
        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.activated.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .expect("expected at least one vanilla creature in the corpus")
            .clone()
    }

    /// Session lifecycle: install / use / clear / re-clear. Mirrors
    /// the D1 smoke test but stores an actual StepEngine now.
    #[test]
    fn session_lifecycle_install_use_clear() {
        let _ = clear_session();
        assert!(!clear_session(), "expected no session before install");
        let err = with_session(|_| ()).unwrap_err();
        assert!(err.contains("no game in progress"));

        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = StartGameArgs {
            seed: 0xCAFE,
            deck_a_ids: deck_ids.clone(),
            deck_b_ids: deck_ids,
            opp_ai: "heuristic".to_string(),
        };
        let engine = build_engine(&args).expect("build_engine");
        install_session(GameSession { engine });

        // Session exists; we can borrow it.
        let active_player =
            with_session(|s| s.engine.state.active_player).expect("session present");
        assert_eq!(active_player, crate::game::PlayerId::A);

        assert!(clear_session(), "expected clear to find a session");
        assert!(!clear_session(), "expected second clear to be a no-op");
        let err = with_session(|_| ()).unwrap_err();
        assert!(err.contains("no game in progress"));
    }

    /// S6: tsot_start_game parses JSON, builds the engine via the
    /// StepEngine path (no threads, no catch_unwind), drives to the
    /// first human decision, returns serialized HumanPrompt.
    #[test]
    fn start_game_returns_first_pickcard_prompt() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let env_json = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");
        let env: Value = serde_json::from_str(&env_json).expect("envelope JSON parses");
        // The wasm FFI now returns `{prompt, log}` envelope so the JS
        // LOG panel can surface engine log lines per yield. Test
        // reads through `.prompt`.
        let prompt = &env["prompt"];
        assert_eq!(prompt["kind"], "PickCard", "first decision should be a card pick");
        assert_eq!(prompt["player"], "A", "first decision is on side A (the human)");
        let candidates = prompt["candidates"]
            .as_array()
            .expect("candidates array present");
        assert!(
            !candidates.is_empty(),
            "fresh hand should have ≥1 playable candidate, got: {candidates:?}"
        );

        assert!(clear_session(), "expected start_game to install a session");
    }

    /// S6: tsot_apply_action resumes the engine with the supplied
    /// action; sending `Pass` on the Main1 PickCard advances into
    /// Combat — the next prompt is PickAttackers for the human.
    #[test]
    fn apply_action_pass_advances_to_attacker_prompt() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let _first_prompt = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");

        let action_json = serde_json::json!({ "kind": "Pass" }).to_string();
        let next_env_json =
            tsot_apply_action_impl(&action_json).expect("tsot_apply_action returned Err");

        let env: Value =
            serde_json::from_str(&next_env_json).expect("envelope JSON parses");
        let next = &env["prompt"];
        assert_eq!(
            next["kind"], "PickAttackers",
            "after Pass on Main1, next decision is combat attacker pick (got {next})"
        );
        assert_eq!(
            next["player"], "A",
            "attacker pick is still the active player A (got {next})"
        );

        assert!(clear_session(), "session should still be active");
    }

    // ----- O5: wasm FFI envelope contains the structured trace ---

    /// INTENT: `tsot_start_game_impl` returns an envelope with a
    /// `trace` field that is a JSON array. This is the foundation
    /// for the UI rendering: the trace stream crosses the FFI
    /// boundary as structured data, not strings.
    #[test]
    fn start_game_envelope_contains_trace_array() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let env_json = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");
        let env: Value = serde_json::from_str(&env_json).expect("envelope parses");
        let trace = env["trace"]
            .as_array()
            .expect("envelope.trace should be an array");
        assert!(
            !trace.is_empty(),
            "trace should contain events from the engine run, got empty"
        );
        assert!(clear_session(), "expected start_game to install a session");
    }

    /// INTENT: the trace array carries Step events recorded by the
    /// engine during `tsot_start_game`. Proves the bus is enabled
    /// at FFI entry and the structured events flow through.
    #[test]
    fn start_game_trace_contains_step_events() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let env_json = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");
        let env: Value = serde_json::from_str(&env_json).expect("envelope parses");
        let trace = env["trace"].as_array().expect("trace is array");
        let step_count = trace
            .iter()
            .filter(|e| e["kind"] == "Step")
            .count();
        assert!(
            step_count >= 1,
            "trace should contain ≥1 Step event, got {step_count} (full trace: {trace:?})"
        );
        assert!(clear_session());
    }

    /// INTENT: the trace array carries Cursor events. Proves the
    /// O2 instrumentation flows from the engine across the FFI.
    #[test]
    fn start_game_trace_contains_cursor_events() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let env_json = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");
        let env: Value = serde_json::from_str(&env_json).expect("envelope parses");
        let trace = env["trace"].as_array().expect("trace is array");
        let cursor_count = trace
            .iter()
            .filter(|e| e["kind"] == "Cursor")
            .count();
        assert!(
            cursor_count >= 1,
            "trace should contain ≥1 Cursor event, got {cursor_count}"
        );
        assert!(clear_session());
    }

    /// INTENT: `tsot_apply_action_impl` also returns an envelope
    /// with `trace`. Same contract — every FFI call carries its
    /// own trace slice.
    #[test]
    fn apply_action_envelope_contains_trace_array() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let _ = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");
        let action_json = serde_json::json!({ "kind": "Pass" }).to_string();
        let env_json =
            tsot_apply_action_impl(&action_json).expect("tsot_apply_action returned Err");
        let env: Value = serde_json::from_str(&env_json).expect("envelope parses");
        let trace = env["trace"]
            .as_array()
            .expect("envelope.trace should be an array");
        assert!(!trace.is_empty(), "trace should be non-empty after Pass");
        assert!(clear_session());
    }

    // ----- Deckbuilder FFI ---------------------------------------

    /// INTENT: `tsot_list_card_pool_impl` returns a non-empty JSON
    /// array of card pool entries with the fields the JS deckbuilder
    /// renders. No session required.
    #[test]
    fn list_card_pool_returns_pool_entries_json() {
        let json = tsot_list_card_pool_impl().expect("list_card_pool returned Err");
        let env: Value = serde_json::from_str(&json).expect("envelope JSON parses");
        assert_eq!(env["ok"], true, "envelope ok=true");
        assert!(
            env["trace"].is_array(),
            "envelope.trace is an array (OBS O5b: non-envelope FFI now drains trace)"
        );
        assert!(
            env["errors"].is_array(),
            "envelope.errors is an array (ERROR Slice 4: typed errors flow through)"
        );
        let arr = env["result"]
            .as_array()
            .expect("envelope.result is the card-pool array");
        assert!(!arr.is_empty(), "card pool should be non-empty");
        // Every entry has the keys the JS deckbuilder renders.
        for (i, entry) in arr.iter().enumerate() {
            for field in [
                "id",
                "name",
                "kind",
                "cost_text",
                "colors",
                "symbols",
                "subtypes",
                "abilities",
            ] {
                assert!(
                    entry.get(field).is_some(),
                    "entry[{i}] missing field {field}: {entry}"
                );
            }
        }
    }

    /// INTENT: `tsot_list_preset_decks_impl` returns the curated
    /// presets (Red Starter first as deckbuilder default + Blue
    /// Starter), each with a flat `cards` array of 50 card IDs —
    /// the shape `tsot_start_game`'s `deck_a_ids` consumes.
    /// Order pinned 2026-06-09 (Red took id `"starter"`).
    #[test]
    fn list_preset_decks_returns_starter_plus_gauntlet_json() {
        let json = tsot_list_preset_decks_impl().expect("list_preset_decks returned Err");
        let env: Value = serde_json::from_str(&json).expect("envelope JSON parses");
        assert_eq!(env["ok"], true, "envelope ok=true");
        assert!(
            env["trace"].is_array(),
            "envelope.trace is an array (OBS O5b)"
        );
        assert!(
            env["errors"].is_array(),
            "envelope.errors is an array (ERROR Slice 5)"
        );
        let arr = env["result"]
            .as_array()
            .expect("envelope.result is the presets array");
        assert_eq!(arr.len(), 3, "Red Starter + Blue Starter + Yield Test = 3 presets");
        assert_eq!(arr[0]["id"], "starter", "first preset is the Red Starter");
        assert_eq!(arr[1]["id"], "starter-blue", "second preset is the Blue Starter");
        assert_eq!(arr[2]["id"], "yield-test", "third preset is the Yield Test");
        for (i, preset) in arr.iter().enumerate() {
            let cards = preset["cards"]
                .as_array()
                .unwrap_or_else(|| panic!("preset[{i}] missing cards array: {preset}"));
            assert_eq!(
                cards.len(),
                50,
                "preset[{i}] ({}) should have 50 cards",
                preset["id"]
            );
            for card_id in cards {
                assert!(
                    card_id.is_string(),
                    "preset[{i}] cards must be strings, got {card_id}"
                );
            }
        }
    }

    // ----- Auto-game (spectate) FFI -----------------------------

    /// INTENT: `tsot_run_auto_game_impl` with both Heuristic AIs and
    /// a vanilla deck runs the game to completion and returns a
    /// snapshot timeline + a winner. The vanilla template is fast
    /// (no Lua handlers) so this finishes in ms even in debug.
    #[test]
    fn auto_game_heuristic_vs_heuristic_returns_snapshots_and_winner() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "ai_a": "heuristic",
            "ai_b": "heuristic",
        })
        .to_string();
        let json = tsot_run_auto_game_impl(&args).expect("auto_game ran");
        let env: Value = serde_json::from_str(&json).expect("envelope parses");
        assert_eq!(env["ok"], true);
        let snapshots = env["snapshots"]
            .as_array()
            .expect("snapshots is an array");
        assert!(
            snapshots.len() >= 2,
            "expected ≥ 2 snapshots (initial + final), got {}",
            snapshots.len()
        );
        assert!(
            env.get("winner").is_some(),
            "envelope must carry a winner field"
        );
        assert!(
            env["total_turns"].as_u64().is_some(),
            "total_turns is a number"
        );
    }

    /// INTENT: each snapshot carries the keys the scrubber UI
    /// reads — `turn`, `phase`, `active_player`, and a `state` view.
    #[test]
    fn auto_game_snapshots_carry_state_view_and_keys() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "ai_a": "heuristic",
            "ai_b": "heuristic",
        })
        .to_string();
        let json = tsot_run_auto_game_impl(&args).expect("auto_game ran");
        let env: Value = serde_json::from_str(&json).expect("envelope parses");
        let snapshots = env["snapshots"].as_array().unwrap();
        for (i, s) in snapshots.iter().enumerate() {
            for field in ["turn", "phase", "active_player", "state"] {
                assert!(
                    s.get(field).is_some(),
                    "snapshot[{i}] missing field `{field}`: {s}"
                );
            }
            assert!(
                s["state"].get("players").is_some(),
                "snapshot[{i}] state should be a StateView with `players` field: {s}"
            );
        }
    }

    /// INTENT: snapshots advance monotonically by `(turn, phase,
    /// active_player)` key — no duplicates back-to-back, and we
    /// never go backward in turn.
    #[test]
    fn auto_game_snapshots_advance_monotonically() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "ai_a": "heuristic",
            "ai_b": "heuristic",
        })
        .to_string();
        let json = tsot_run_auto_game_impl(&args).expect("auto_game ran");
        let env: Value = serde_json::from_str(&json).expect("envelope parses");
        let snapshots = env["snapshots"].as_array().unwrap();
        // Turns never go down; back-to-back identical (turn, phase,
        // active_player) triples are allowed only for the final
        // duplicate-of-last-state we always append at game end.
        let mut prev_turn: u64 = 0;
        for s in snapshots {
            let t = s["turn"].as_u64().unwrap_or(0);
            assert!(t >= prev_turn, "turn went backward: {prev_turn} -> {t}");
            prev_turn = t;
        }
    }

    /// INTENT: unknown ai name fails cleanly with an error, not a
    /// panic — the scrubber UI's AI picker should never produce one
    /// of these, but a typo on the JS side shouldn't crash the FFI.
    #[test]
    fn auto_game_unknown_ai_name_returns_error() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "ai_a": "heuristic",
            "ai_b": "no-such-ai",
        })
        .to_string();
        let result = tsot_run_auto_game_impl(&args);
        assert!(result.is_err(), "unknown ai must Err, got: {result:?}");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("unknown ai") && msg.contains("no-such-ai"),
            "error message should name the bad value: {msg}"
        );
    }

    // ----- Save / Load FFI ---------------------------------------

    /// INTENT: `tsot_save_game_impl` requires a live session.
    /// Without one, returns Err and the caller surfaces "no game in
    /// progress" rather than crashing.
    #[test]
    fn save_game_without_session_returns_error() {
        let _ = clear_session();
        let result = tsot_save_game_impl();
        assert!(
            result.is_err(),
            "save_game with no session should Err, got: {result:?}"
        );
    }

    /// INTENT: `tsot_save_game_impl` returns parseable SaveFile JSON
    /// when a session is active.
    #[test]
    fn save_game_returns_parseable_savefile_json() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();
        let _ = tsot_start_game_impl(&args).expect("start_game");

        let envelope_json = tsot_save_game_impl().expect("save_game returned Err");
        let env: Value =
            serde_json::from_str(&envelope_json).expect("envelope JSON parses");
        assert_eq!(env["ok"], true, "envelope ok=true");
        assert!(
            env["trace"].is_array(),
            "envelope.trace is an array (OBS O5b)"
        );
        assert!(
            env["errors"].is_array(),
            "envelope.errors is an array"
        );
        let parsed = &env["result"];
        assert!(
            parsed.get("state").is_some(),
            "SaveFile JSON (in envelope.result) should have a state field"
        );
        assert!(
            parsed.get("cursor").is_some(),
            "SaveFile JSON should have a cursor field (set by from_step_engine)"
        );
        assert!(clear_session());
    }

    /// INTENT: reproduce the user's "index out of bounds" load
    /// failure with the actual starter deck (multi-card mix) on a
    /// game that has been ADVANCED — the turn the user saw fail
    /// was 9, not 1. Drive turns by dispatching the action that
    /// matches each prompt's kind, save, then load. If load
    /// panics, the native backtrace tells us file:line.
    #[test]
    fn starter_deck_advanced_save_then_load_does_not_panic() {
        let _ = clear_session();
        let deck_ids = crate::sim::deck_presets::STARTER_DECK_IDS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let start_args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "uct",
        })
        .to_string();
        let first_env = tsot_start_game_impl(&start_args).expect("start_game");
        let mut last_prompt_kind = prompt_kind_of(&first_env);

        // Drive the game forward by dispatching an action that
        // matches the prompt's kind. Pass for PickCard / Main2Pick,
        // empty Attackers for PickAttackers, empty Blocks for
        // PickBlocks. Stop once turn ≥ 5.
        for _ in 0..400 {
            let turn = with_session(|s| s.engine.state.turn).expect("session");
            if turn >= 9 {
                break;
            }
            let action = match last_prompt_kind.as_deref() {
                Some("PickAttackers") => {
                    serde_json::json!({ "kind": "Attackers", "iids": [] }).to_string()
                }
                Some("PickBlocks") => {
                    serde_json::json!({ "kind": "Blocks", "pairs": [] }).to_string()
                }
                Some("GameOver") => break,
                _ => serde_json::json!({ "kind": "Pass" }).to_string(),
            };
            let env = tsot_apply_action_impl(&action).expect("apply_action");
            last_prompt_kind = prompt_kind_of(&env);
        }

        let save_json = tsot_save_game_impl().expect("save_game");
        assert!(clear_session(), "session present pre-clear");
        let load_args = serde_json::json!({
            "save_json": save_json,
            "opp_ai": "uct",
            "seed": 0xBEEF_u64,
        })
        .to_string();
        let _ = tsot_load_game_impl(&load_args).expect("load_game must not panic");
        assert!(clear_session());
    }

    /// Pull the `prompt.kind` out of an FFI envelope JSON. Returns
    /// `None` if the envelope shape doesn't match (e.g., GameOver
    /// or error envelope).
    fn prompt_kind_of(env_json: &str) -> Option<String> {
        let v: Value = serde_json::from_str(env_json).ok()?;
        v.get("prompt")?.get("kind")?.as_str().map(String::from)
    }

    // ----- Preview UCT FFI ---------------------------------------

    /// INTENT: `tsot_preview_uct_impl` without an active session
    /// returns Err — same contract as `tsot_save_game_impl`.
    #[test]
    fn preview_uct_without_session_returns_error() {
        let _ = clear_session();
        let args = "{}";
        let result = tsot_preview_uct_impl(args);
        assert!(
            result.is_err(),
            "preview_uct with no session must Err, got: {result:?}"
        );
    }

    /// INTENT: with a live session, `tsot_preview_uct_impl` returns
    /// a parseable envelope carrying the candidate array, requested/
    /// completed iteration counts, and ok=true.
    #[test]
    fn preview_uct_returns_candidates_envelope() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();
        let _ = tsot_start_game_impl(&args).expect("start_game");

        let preview_args = serde_json::json!({
            "iterations": 4,
            "exploration_c": std::f64::consts::SQRT_2,
            "max_candidates": 4,
        })
        .to_string();
        let env_json = tsot_preview_uct_impl(&preview_args).expect("preview_uct");
        let env: Value = serde_json::from_str(&env_json).expect("preview envelope parses");
        assert_eq!(env["ok"], true, "preview returns ok:true");
        assert!(env.get("candidates").and_then(|c| c.as_array()).is_some(),
            "envelope has a `candidates` array");
        assert_eq!(env["iterations_requested"], 4);
        assert!(
            env["iterations_completed"].as_u64().is_some(),
            "iterations_completed is a number"
        );
        assert!(clear_session());
    }

    /// INTENT: preview must not mutate the live session state. Run
    /// start_game, snapshot the current turn/phase, run a preview,
    /// re-read the session — turn/phase unchanged.
    #[test]
    fn preview_uct_does_not_mutate_session_state() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();
        let _ = tsot_start_game_impl(&args).expect("start_game");

        let (turn_before, phase_before, active_before) =
            with_session(|s| (s.engine.state.turn, s.engine.state.phase, s.engine.state.active_player))
                .expect("session");

        let preview_args = serde_json::json!({"iterations": 4, "max_candidates": 4}).to_string();
        let _ = tsot_preview_uct_impl(&preview_args).expect("preview_uct");

        let (turn_after, phase_after, active_after) =
            with_session(|s| (s.engine.state.turn, s.engine.state.phase, s.engine.state.active_player))
                .expect("session");
        assert_eq!(turn_before, turn_after, "preview must not advance turn");
        assert_eq!(phase_before, phase_after, "preview must not change phase");
        assert_eq!(active_before, active_after, "preview must not change active player");
        assert!(clear_session());
    }

    /// INTENT: `tsot_cancel_uct_impl` flips the cancel flag visible
    /// to `pick_play_uct`. Same thread, single-call round-trip.
    #[test]
    fn cancel_uct_sets_the_thread_local_flag() {
        crate::sim::uct::clear_uct_cancel();
        assert!(!crate::sim::uct::is_uct_cancel_requested());
        let _ = tsot_cancel_uct_impl().expect("cancel_uct");
        assert!(
            crate::sim::uct::is_uct_cancel_requested(),
            "cancel_uct must set the flag"
        );
        crate::sim::uct::clear_uct_cancel();
    }

    /// INTENT: load the user's actual failing save (turn 1, Main1
    /// after casting blue-monkey) and verify the load itself does
    /// not panic. Native backtrace will name the exact file:line if
    /// it does. Reproduces the user's "index out of bounds" wasm
    /// trap that the JS stack only attributed to `tsot_load_game`.
    #[test]
    fn user_failing_save_turn_1_load_does_not_panic() {
        let _ = clear_session();
        let save_json = std::fs::read_to_string(
            "tests/fixtures/failing-load-turn-1.json",
        )
        .expect("read fixture turn-1 save");
        let load_args = serde_json::json!({
            "save_json": save_json,
            "opp_ai": "uct",
            "seed": 0xBEEF_u64,
        })
        .to_string();
        let _ = tsot_load_game_impl(&load_args)
            .expect("load_game must not panic");
        assert!(clear_session());
    }

    /// INTENT: reproduce the user's "index out of bounds" load
    /// failure with the actual starter deck (multi-card mix) instead
    /// of a 50-card single-template deck. This is the exact flow:
    /// start_game with starter deck → save → load. If load panics,
    /// the native backtrace tells us file:line.
    #[test]
    fn starter_deck_save_then_load_does_not_panic() {
        let _ = clear_session();
        let deck_ids = crate::sim::deck_presets::STARTER_DECK_IDS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let start_args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "uct",
        })
        .to_string();
        let _ = tsot_start_game_impl(&start_args).expect("start_game");
        let save_json = tsot_save_game_impl().expect("save_game");
        assert!(clear_session(), "session present pre-clear");
        let load_args = serde_json::json!({
            "save_json": save_json,
            "opp_ai": "uct",
            "seed": 0xBEEF_u64,
        })
        .to_string();
        let _ = tsot_load_game_impl(&load_args).expect("load_game must not panic");
        assert!(clear_session());
    }

    /// INTENT: round-trip — start a game, save, load it back via
    /// the FFI with a fresh opp_ai + seed, the session is alive and
    /// the resumed game-state phase matches.
    #[test]
    fn save_then_load_restores_game_phase() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let start_args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();
        let start_env_json =
            tsot_start_game_impl(&start_args).expect("start_game returned Err");
        let start_env: Value =
            serde_json::from_str(&start_env_json).expect("start envelope parses");
        let phase_before =
            start_env["prompt"]["state"]["phase"].as_str().unwrap_or("").to_string();
        assert!(!phase_before.is_empty(), "start prompt should carry a phase");

        let save_json = tsot_save_game_impl().expect("save_game returned Err");

        // Tear down then load_game from the JSON.
        assert!(clear_session(), "session should be present pre-clear");
        let load_args = serde_json::json!({
            "save_json": save_json,
            "opp_ai": "heuristic",
            "seed": 0xBEEF_u64,
        })
        .to_string();
        let load_env_json =
            tsot_load_game_impl(&load_args).expect("load_game returned Err");
        let load_env: Value =
            serde_json::from_str(&load_env_json).expect("load envelope parses");
        let phase_after =
            load_env["prompt"]["state"]["phase"].as_str().unwrap_or("").to_string();

        assert_eq!(
            phase_before, phase_after,
            "save → load should land in the same phase (before={phase_before}, after={phase_after})"
        );
        assert!(clear_session(), "load_game should install a session");
    }

    /// INTENT: each FFI call's trace is fresh — events from the
    /// previous call don't bleed into the next. The FFI drains the
    /// bus at exit (no cross-call accumulation).
    #[test]
    fn apply_action_trace_does_not_inherit_start_game_trace() {
        let _ = clear_session();
        let template = vanilla_template();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();
        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();
        let start_env: Value = serde_json::from_str(
            &tsot_start_game_impl(&args).expect("tsot_start_game returned Err"),
        )
        .expect("envelope parses");
        let start_step_count = start_env["trace"]
            .as_array()
            .map(|arr| arr.iter().filter(|e| e["kind"] == "Step").count())
            .unwrap_or(0);

        let action_json = serde_json::json!({ "kind": "Pass" }).to_string();
        let next_env: Value = serde_json::from_str(
            &tsot_apply_action_impl(&action_json).expect("tsot_apply_action returned Err"),
        )
        .expect("envelope parses");
        let next_trace = next_env["trace"]
            .as_array()
            .expect("envelope.trace should be an array");
        assert!(
            next_trace.len() < start_step_count + 1000,
            "trace should be the slice for THIS call, not accumulated; got {} (start had {start_step_count} Steps)",
            next_trace.len()
        );
        let next_step_count = next_trace
            .iter()
            .filter(|e| e["kind"] == "Step")
            .count();
        assert!(
            next_step_count >= 1,
            "apply_action should still contain Step events for its own call, got {next_step_count}"
        );
        assert!(clear_session());
    }
}
