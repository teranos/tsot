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
    let args: StartGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("tsot_start_game: bad args JSON: {e}"))?;

    let _ = clear_session();

    let engine = build_engine(&args)?;
    let mut session = GameSession { engine };
    let prompt = drive_to_next_yield(&mut session.engine, None)?;
    // Drain the engine log into the envelope so the JS LOG panel can
    // surface every internal decision (card picks, attackers, blocks,
    // UCT trace ASCII tree) without devtools. The buffer is cleared
    // after every yield so JS sees only the lines since the last call.
    let log = std::mem::take(&mut session.engine.log);
    let envelope = serde_json::json!({ "prompt": prompt, "log": log });
    let envelope_json =
        serde_json::to_string(&envelope).map_err(|e| format!("serialize first prompt: {e}"))?;
    install_session(session);
    Ok(envelope_json)
}

/// Submit a HumanAction. The engine resumes with the supplied action,
/// drives forward until the next NeedHuman / Done, returns the prompt
/// JSON.
pub(crate) fn tsot_apply_action_impl(action_json: &str) -> Result<String, String> {
    let action: HumanAction = serde_json::from_str(action_json)
        .map_err(|e| format!("tsot_apply_action: bad action JSON: {e}"))?;

    let (prompt, log) = with_session(|s| -> Result<_, String> {
        let prompt = drive_to_next_yield(&mut s.engine, Some(action))?;
        let log = std::mem::take(&mut s.engine.log);
        Ok((prompt, log))
    })
    .map_err(|e| e.to_string())??;
    let envelope = serde_json::json!({ "prompt": prompt, "log": log });
    serde_json::to_string(&envelope).map_err(|e| format!("serialize next prompt: {e}"))
}

/// Construct the engine from the JSON args. Card registry is rebuilt
/// each `start_game` (cheap for v1; cache if it ever shows up in a
/// profile).
fn build_engine(args: &StartGameArgs) -> Result<StepEngine, String> {
    use crate::card::CardRegistry;
    use crate::game::GameState;
    use crate::sim::genome::to_deck;
    use crate::sim::AiKind;

    let registry = CardRegistry::load_embedded().map_err(|e| format!("registry load: {e}"))?;
    let deck_a =
        to_deck(&registry, &args.deck_a_ids).map_err(|e| format!("deck A rebuild: {e:?}"))?;
    let deck_b =
        to_deck(&registry, &args.deck_b_ids).map_err(|e| format!("deck B rebuild: {e:?}"))?;
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
            Err(e) => export(format!("error: {e}")),
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
            Err(e) => export(format!("error: {e}")),
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
}
