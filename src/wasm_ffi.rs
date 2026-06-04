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

use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::ffi::{c_char, CStr, CString};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::sim::human::{HumanAction, HumanInterface, HumanPrompt};

/// Live game session — one per browser tab. The D4 shim model is
/// save-and-replay: the session holds the JSON args + a growing
/// history of HumanActions the user has submitted; each FFI call
/// rebuilds the engine from scratch and replays through the history.
/// O(N) work per step, O(N²) total per game. Acceptable for v1; the
/// state-machine refactor (STATE_MACHINE.md) is the proper fix.
///
/// `iface` is kept around so identity-comparison tests still work
/// (lifecycle test in D1). Not strictly needed by the replay path
/// since each FFI call constructs a fresh HumanInterface inside
/// `drive_engine_to_next_yield`.
pub(crate) struct GameSession {
    pub args: StartGameArgs,
    pub history: Vec<HumanAction>,
    pub iface: Arc<HumanInterface>,
    pub prompt_rx: Receiver<HumanPrompt>,
    pub action_tx: Sender<HumanAction>,
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

/// Install a new session, dropping any previous one. Used by
/// `tsot_start_game` (D2) once it's built the GameState + iface.
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

/// Initialize a session + drive the engine to the first human
/// decision, return the prompt JSON.
///
/// Save-and-replay shim: rebuilds the engine, replays the action
/// history through a `HumanInterface` in scripted mode, panics with
/// `YieldSignal` on the first decision past the history end.
/// `catch_unwind` recovers the prompt — depends on Rust's
/// `panic_unwind` crate.
///
/// Native: works. Wasm: blocks at link time (see `.cargo/config.toml`
/// — `panic_unwind` symbol resolution requires nightly + build-std
/// OR the StepEngine refactor that doesn't use `catch_unwind` at all).
/// The StepEngine path (STATE_MACHINE.md S1-S13) is the chosen fix.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn tsot_start_game_impl(args_json: &str) -> Result<String, String> {
    let args: StartGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("tsot_start_game: bad args JSON: {e}"))?;

    let _ = clear_session();

    let first_prompt = drive_engine_to_next_yield(&args, &[])?;
    let prompt_json = serde_json::to_string(&first_prompt)
        .map_err(|e| format!("serialize first prompt: {e}"))?;

    // The iface / channels stay in the session purely so the D1
    // identity-check test keeps working. The replay path constructs
    // a fresh `HumanInterface` on every FFI call.
    let (iface, prompt_rx, action_tx) = HumanInterface::new();
    install_session(GameSession {
        args,
        history: Vec::new(),
        iface: Arc::new(iface),
        prompt_rx,
        action_tx,
    });

    Ok(prompt_json)
}

/// Append the action to history, replay, return the next prompt.
/// Same save-and-replay shim shape as [`tsot_start_game_impl`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn tsot_apply_action_impl(action_json: &str) -> Result<String, String> {
    let action: HumanAction = serde_json::from_str(action_json)
        .map_err(|e| format!("tsot_apply_action: bad action JSON: {e}"))?;

    // Snapshot args + extend history while holding the session lock
    // briefly; the engine drive happens outside the lock to keep the
    // critical section short.
    let (args, mut history) = with_session(|s| (s.args.clone(), s.history.clone()))
        .map_err(|e| e.to_string())?;
    history.push(action);

    let next_prompt = drive_engine_to_next_yield(&args, &history)?;
    let prompt_json = serde_json::to_string(&next_prompt)
        .map_err(|e| format!("serialize next prompt: {e}"))?;

    let _ = with_session(|s| {
        s.history = history;
    });

    Ok(prompt_json)
}

/// Wasm stub: returns an error pointing at the StepEngine refactor.
/// Real wasm-side driving will land with S6.
#[cfg(target_arch = "wasm32")]
pub(crate) fn tsot_start_game_impl(_args_json: &str) -> Result<String, String> {
    Err("tsot_start_game: wasm path needs StepEngine (STATE_MACHINE.md S1-S6) — \
         catch_unwind-based shim doesn't link against the precompiled wasm \
         stdlib's exception ABI."
        .to_string())
}

/// Wasm stub: see `tsot_start_game_impl`.
#[cfg(target_arch = "wasm32")]
pub(crate) fn tsot_apply_action_impl(_action_json: &str) -> Result<String, String> {
    Err("tsot_apply_action: wasm path needs StepEngine (STATE_MACHINE.md S1-S6)".to_string())
}

/// Build the engine from `args`, replay `history` through a scripted
/// HumanInterface, catch the YieldSignal that fires on the first
/// human decision past the history. Returns the captured prompt, or
/// (when the engine ran to completion without any further human
/// decisions) a synthesized `GameOver` prompt with the final state.
///
/// Re-panics on any panic payload that isn't a `YieldSignal` so real
/// engine bugs aren't silently swallowed.
#[cfg(not(target_arch = "wasm32"))]
fn drive_engine_to_next_yield(
    args: &StartGameArgs,
    history: &[HumanAction],
) -> Result<HumanPrompt, String> {
    use std::panic::AssertUnwindSafe;
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use crate::card::CardRegistry;
    use crate::game::GameState;
    use crate::sim::genome::to_deck;
    use crate::sim::human::{ScriptedSource, YieldSignal};
    use crate::sim::run::run_game_continue;
    use crate::sim::snapshot::build_state_view;
    use crate::sim::AiKind;

    // Capture the engine's final state via a Mutex so the
    // post-catch_unwind branch can grab it after the closure runs.
    let final_state_slot: std::sync::Mutex<Option<GameState>> = std::sync::Mutex::new(None);
    let args_owned = args.clone();
    let history_owned = history.to_vec();

    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        // Per-call CardRegistry. Cheap to rebuild but the dominant
        // cost of repeated FFI calls; cache here if it becomes a hot
        // path (it shouldn't for v1 — humans decide slower than this).
        let registry = CardRegistry::load_embedded()
            .map_err(|e| format!("registry load: {e}"))?;
        let deck_a = to_deck(&registry, &args_owned.deck_a_ids)
            .map_err(|e| format!("deck A rebuild: {e:?}"))?;
        let deck_b = to_deck(&registry, &args_owned.deck_b_ids)
            .map_err(|e| format!("deck B rebuild: {e:?}"))?;
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(crate::game::Journal::new());

        let (iface, _prompt_rx, _action_tx) = HumanInterface::new();
        *iface.scripted.lock().expect("fresh mutex") = Some(ScriptedSource {
            actions: history_owned,
            cursor: 0,
        });
        let iface = Arc::new(iface);

        let opp = match args_owned.opp_ai.as_str() {
            "heuristic" => AiKind::Heuristic,
            "mcts" => AiKind::Mcts(crate::sim::mcts::MctsConfig {
                base_seed: args_owned.seed.wrapping_add(0xCAFE_BABE),
                ..Default::default()
            }),
            "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
                base_seed: args_owned.seed.wrapping_add(0x00C0_FFEE),
                ..Default::default()
            }),
            other => {
                return Err(format!("unknown opp_ai {other:?}"));
            }
        };
        let ais = [AiKind::Human(iface), opp];

        let mut rng = StdRng::seed_from_u64(args_owned.seed);
        let mut log: Vec<String> = Vec::new();
        let _stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), &ais);

        // Game ran to completion without any further human decision.
        *final_state_slot.lock().expect("final-state mutex") = Some(state);
        Ok(())
    }));

    match outcome {
        Err(payload) => match payload.downcast::<YieldSignal>() {
            Ok(signal) => Ok(signal.prompt),
            Err(payload) => std::panic::resume_unwind(payload),
        },
        Ok(Ok(())) => {
            let state = final_state_slot
                .into_inner()
                .map_err(|e| format!("final-state mutex poisoned: {e}"))?
                .ok_or_else(|| "engine returned without populating final state".to_string())?;
            let viewer = crate::game::PlayerId::A; // human is always side A in v1
            let view = build_state_view(&state, viewer);
            Ok(HumanPrompt::GameOver {
                state: view,
                winner: state.winner,
            })
        }
        Ok(Err(e)) => Err(e),
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

    // (Previous Asyncify proof `tsot_async_sleep` removed —
    // ASYNCIFY=1 is incompatible with the Rust `catch_unwind` the D4
    // shim depends on, and we no longer need JS-yielding extern
    // functions for the synchronous save-and-replay model.)

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
    use crate::game::PlayerId;
    use serde_json::Value;

    /// Exercise the session lifecycle: install → with → clear →
    /// with-fails. Builds a minimal session (just the JS-facing
    /// handles) — no engine state. Verifies the FFI's
    /// session-management plumbing is correct.
    #[test]
    fn session_lifecycle_install_use_clear() {
        // No session before install.
        assert!(!clear_session(), "expected no session before install");
        let err = with_session(|_| ()).unwrap_err();
        assert!(err.contains("no game in progress"));

        let (iface, prompt_rx, action_tx) = HumanInterface::new();
        let iface = Arc::new(iface);
        let iface_outer = iface.clone();

        let session = GameSession {
            args: StartGameArgs {
                seed: 0,
                deck_a_ids: Vec::new(),
                deck_b_ids: Vec::new(),
                opp_ai: "heuristic".to_string(),
            },
            history: Vec::new(),
            iface,
            prompt_rx,
            action_tx,
        };

        install_session(session);

        // Identity: the iface field is the same Arc we kept.
        let arc_addr_outer = Arc::as_ptr(&iface_outer) as usize;
        let arc_addr_inner = with_session(|s| Arc::as_ptr(&s.iface) as usize).unwrap();
        assert_eq!(
            arc_addr_outer, arc_addr_inner,
            "session.iface should be the same Arc we installed"
        );

        // Clear succeeds + returns true.
        assert!(clear_session(), "expected clear to find a session");
        // Second clear: no session, returns false.
        assert!(!clear_session(), "expected second clear to be a no-op");
        // with_session post-clear surfaces the same error.
        let err = with_session(|_| ()).unwrap_err();
        assert!(err.contains("no game in progress"));
    }

    /// D2: tsot_start_game parses JSON args, builds session, runs
    /// the engine to the first human decision, returns serialized
    /// HumanPrompt. Native test uses a thread (mpsc blocks). On wasm
    /// this will need the D4 Asyncify bridge — the Rust-internal
    /// `_impl` function below is shared between both targets, only
    /// the engine-driver differs.
    #[test]
    fn start_game_returns_first_pickcard_prompt() {
        // Make sure no prior test left a session installed.
        let _ = clear_session();

        let registry_for_deck = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        // Pick a vanilla creature: castable, no Lua handlers, no
        // X-cost or sacrifice/graveyard setup cost. That way the
        // affordability check trivially passes on opening hand and
        // we get at least one candidate in the first PickCard.
        let template = registry_for_deck
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
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
            .clone();
        eprintln!("[test] using template card: {} ({:?})", template.id, template.cost);
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();

        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        let prompt_json = tsot_start_game_impl(&args).expect("tsot_start_game returned Err");

        // HumanPrompt is Serialize-only; for the test we just sanity-
        // check the JSON shape rather than full-deserialize it.
        let prompt: Value = serde_json::from_str(&prompt_json).expect("prompt JSON parses");
        assert_eq!(prompt["kind"], "PickCard", "first decision should be a card pick");
        assert_eq!(prompt["player"], "A", "first decision is on side A (the human)");
        let candidates = prompt["candidates"]
            .as_array()
            .expect("candidates array present");
        assert!(
            !candidates.is_empty(),
            "fresh hand should have ≥1 playable candidate, got: {candidates:?}"
        );

        // Session is now active.
        let _ = PlayerId::A; // suppress unused import warning when test is filtered
        assert!(clear_session(), "expected start_game to install a session");
    }

    /// D3: tsot_apply_action pushes the JS-supplied action through
    /// the channel, the engine consumes it, advances to the next
    /// human decision, and we serialize that next prompt back to
    /// JS. Test: start a game → PickCard → send Pass → expect the
    /// engine to move past Main1 into Combat → next prompt should
    /// be PickAttackers (for side A — the human).
    #[test]
    fn apply_action_pass_advances_to_attacker_prompt() {
        let _ = clear_session();

        let registry_for_deck = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let template = registry_for_deck
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_ids: Vec<String> = (0..50).map(|_| template.id.clone()).collect();

        let args = serde_json::json!({
            "seed": 0xCAFE_u64,
            "deck_a_ids": deck_ids,
            "deck_b_ids": deck_ids,
            "opp_ai": "heuristic",
        })
        .to_string();

        // Set up the session via D2.
        let _first_prompt =
            tsot_start_game_impl(&args).expect("tsot_start_game returned Err");

        // D3 under test: send Pass, expect the next prompt.
        let action_json = serde_json::json!({ "kind": "Pass" }).to_string();
        let next_prompt_json =
            tsot_apply_action_impl(&action_json).expect("tsot_apply_action returned Err");

        let next: Value = serde_json::from_str(&next_prompt_json).expect("prompt JSON parses");
        assert_eq!(
            next["kind"], "PickAttackers",
            "after Pass on Main1, next decision is combat attacker pick (got {next})"
        );
        assert_eq!(
            next["player"], "A",
            "attacker pick is still the active player A (got {next})"
        );

        // Cleanup.
        assert!(clear_session(), "session should still be active");
    }
}
