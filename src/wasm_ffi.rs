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

/// Live game session — one per browser tab. The JS side talks to
/// this; the engine itself (CardRegistry + GameState + Lua VM) lives
/// elsewhere (engine thread on native, single-threaded yielding fiber
/// on wasm — D4) since `mlua::Lua` is `!Send` and the engine's
/// `mpsc::recv()` calls would deadlock a single-threaded process.
///
/// Field roles:
/// - `iface` — refcount on the `HumanInterface` shared with whichever
///    `AiKind::Human` arm drives the engine's human-side decisions.
///    Held here so we can introspect / dispose without unwrapping the
///    AiKind.
/// - `prompt_rx` — JS-facing prompt source. Engine pushes
///    `HumanPrompt`s through `iface.prompt_tx`; we pull them here
///    and serialize them across the FFI.
/// - `action_tx` — JS-facing action sink. JS pushes
///    `HumanAction`s here; engine consumes via `iface.action_rx`.
/// - `engine_join` — native-only thread handle for the engine task.
///    `None` on wasm (single-thread, no native spawn). Used at
///    teardown to join the engine thread cleanly.
pub(crate) struct GameSession {
    pub iface: Arc<HumanInterface>,
    pub prompt_rx: Receiver<HumanPrompt>,
    pub action_tx: Sender<HumanAction>,
    #[cfg(not(target_arch = "wasm32"))]
    pub engine_join: Option<std::thread::JoinHandle<()>>,
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
#[derive(serde::Deserialize)]
struct StartGameArgs {
    seed: u64,
    deck_a_ids: Vec<String>,
    deck_b_ids: Vec<String>,
    opp_ai: String,
}

/// Initialize a session + run the engine to the first human decision,
/// return the first prompt as JSON. This is the Rust-internal worker
/// that both the native test and the wasm `extern "C"` export call.
///
/// Native: spawns an engine thread (`mlua::Lua` is `!Send` so the
/// registry + GameState are built INSIDE that thread). Main thread
/// waits on `prompt_rx.recv()` for the first prompt.
///
/// Wasm: not implemented in this commit — needs D4 (Asyncify
/// bridge) before the single-threaded model works. Returns Err for
/// now so the failure is clean instead of a deadlock.
pub(crate) fn tsot_start_game_impl(args_json: &str) -> Result<String, String> {
    let args: StartGameArgs = serde_json::from_str(args_json)
        .map_err(|e| format!("tsot_start_game: bad args JSON: {e}"))?;

    // Wipe any prior session before starting fresh.
    let _ = clear_session();

    // Build channels. iface lives in the engine's `AiKind::Human(_)`;
    // we keep prompt_rx + action_tx for the JS-facing handles.
    let (iface, prompt_rx, action_tx) = HumanInterface::new();
    let iface = Arc::new(iface);
    let iface_for_engine = iface.clone();

    #[cfg(not(target_arch = "wasm32"))]
    let engine_join = Some(spawn_engine_thread_native(args, iface_for_engine));

    #[cfg(target_arch = "wasm32")]
    {
        let _ = iface_for_engine; // suppress unused for wasm stub
        return Err(
            "tsot_start_game: wasm engine driver not yet wired (waiting on D4 Asyncify bridge)"
                .to_string(),
        );
    }

    // Wait for the engine to push its first prompt. On native this
    // blocks the main thread; on wasm this would deadlock without
    // D4 (which is why the wasm path above is stubbed).
    let first_prompt = prompt_rx
        .recv()
        .map_err(|e| format!("engine dropped prompt channel before first prompt: {e}"))?;
    let prompt_json = serde_json::to_string(&first_prompt)
        .map_err(|e| format!("serialize first prompt: {e}"))?;

    install_session(GameSession {
        iface,
        prompt_rx,
        action_tx,
        #[cfg(not(target_arch = "wasm32"))]
        engine_join,
    });

    Ok(prompt_json)
}

/// Native-only engine thread. Builds its own CardRegistry (Lua VM
/// is `!Send`, can't be moved across thread boundaries), rebuilds
/// decks from id-strings, runs `run_game_continue` with the human
/// on side A and the configured AI on side B. Exits when the game
/// ends; the channels are dropped on exit, which wakes any pending
/// JS-side `recv()` with a disconnect error.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_engine_thread_native(
    args: StartGameArgs,
    iface_for_engine: Arc<HumanInterface>,
) -> std::thread::JoinHandle<()> {
    use rand::rngs::StdRng;
    use rand::SeedableRng;
    use crate::card::CardRegistry;
    use crate::game::GameState;
    use crate::sim::genome::to_deck;
    use crate::sim::run::run_game_continue;
    use crate::sim::AiKind;

    std::thread::spawn(move || {
        // Per-thread CardRegistry (mlua's Lua is !Send).
        let registry = match CardRegistry::load_embedded() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[engine] failed to load card registry: {e}");
                return;
            }
        };
        let deck_a = match to_deck(&registry, &args.deck_a_ids) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[engine] deck A rebuild failed: {e:?}");
                return;
            }
        };
        let deck_b = match to_deck(&registry, &args.deck_b_ids) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[engine] deck B rebuild failed: {e:?}");
                return;
            }
        };
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(crate::game::Journal::new());

        let opp = match args.opp_ai.as_str() {
            "heuristic" => AiKind::Heuristic,
            "mcts" => AiKind::Mcts(crate::sim::mcts::MctsConfig {
                base_seed: args.seed.wrapping_add(0xCAFE_BABE),
                ..Default::default()
            }),
            "uct" => AiKind::Uct(crate::sim::uct::UctConfig {
                base_seed: args.seed.wrapping_add(0xC0FF_EE),
                ..Default::default()
            }),
            other => {
                eprintln!("[engine] unknown opp_ai {other:?}, defaulting to heuristic");
                AiKind::Heuristic
            }
        };
        let ais = [AiKind::Human(iface_for_engine), opp];

        let mut rng = StdRng::seed_from_u64(args.seed);
        let mut log: Vec<String> = Vec::new();
        let _stats = run_game_continue(&mut state, &mut rng, &mut log, registry.lua(), &ais);
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

    // Asyncify proof-of-concept. `emscripten_sleep` yields to the JS
    // event loop for the given ms, then resumes Rust execution as if
    // the call had blocked. With `-sASYNCIFY=1` and `{async: true}`
    // on the JS ccall, returns a Promise.
    extern "C" {
        fn emscripten_sleep(ms: u32);
    }

    /// Async smoke-test export. Sleeps 100ms (yielding to JS), then
    /// returns a string.
    #[no_mangle]
    pub extern "C" fn tsot_async_sleep() -> *mut c_char {
        unsafe { emscripten_sleep(100); }
        export("yielded and resumed")
    }
}

// Re-export so the wasm bin can reach them through `tsot::wasm_ffi::tsot_*`.
#[cfg(target_arch = "wasm32")]
pub use wasm_exports::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardRegistry;
    use crate::cast_routing::CastRouting;
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
            iface,
            prompt_rx,
            action_tx,
            #[cfg(not(target_arch = "wasm32"))]
            engine_join: None,
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
}
