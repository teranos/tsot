// Web Worker scope. Owns the wasm engine + posts events to main.
//
// Protocol:
//   main → worker: { cmd: "list_card_pool" }
//                  { cmd: "list_preset_decks" }
//                  { cmd: "start_game", args: {…} }
//                  { cmd: "apply_action", action: {…} }
//                  { cmd: "save_game" }
//                  { cmd: "load_game", loadArgs: {save_json, opp_ai, seed} }
//                  { cmd: "test_panic" }  // observability probe
//                  { cmd: "preview_uct", previewArgs: {iterations, exploration_c, max_candidates} }
//                  { cmd: "cancel_uct" }
//                  { cmd: "run_auto_game", autoArgs: {seed, deck_a_ids, deck_b_ids, ai_a, ai_b} }
//   worker → main: { kind: "ready" }
//                  { kind: "uct_iter", line: "<json>" } // mid-call live event
//                  { kind: "info",     line: "<json>" } // "I am alive" signals
//                  { kind: "panic",    line: "<json>" } // captured Rust panic
//                  { kind: "envelope", cmd, json: "<JSON payload>" }
//                  { kind: "error",    error: "<message>" }
//
// `list_card_pool` and `list_preset_decks` are one-shot static
// queries used by the pre-game deckbuilder; no session is required.
// `start_game` / `apply_action` drive the running game.
//
// The `uct_iter` events are posted from `tsot_emit_iteration_event`
// (see assets/wasm-worker-lib.js) which the wasm calls directly
// from inside `pick_play_uct`'s iteration loop. They land on the
// main thread's onmessage queue while the worker is still mid-FFI,
// because postMessage is async w.r.t. the receiving thread.
importScripts('tsot_wasm.js');

let module = null;

// Capture everything the wasm writes to stderr. Rust's "nounwind"
// panic paths (bounds-check, slice indexing, etc.) bypass the
// `std::panic::set_hook` we install in `tsot_wasm::main` and instead
// print the standard `thread '…' panicked at src/foo.rs:42:7:\nindex
// out of bounds: …` message to stderr. By overriding emscripten's
// `printErr` we buffer those lines and ship them to main on the next
// FFI return (or on abort), so the LOG renders them as a rich
// rust-panic block with location + message — even when the hook
// never gets called.
const stderrBuffer = [];
function flushStderrAsPanic() {
  if (stderrBuffer.length === 0) return;
  const text = stderrBuffer.join('\n');
  stderrBuffer.length = 0;
  // Try to parse the standard Rust panic format. Three lines:
  //   thread '…' panicked at FILE:LINE:COL:
  //   <message>
  //   note: run with `RUST_BACKTRACE=1` …
  let location = null;
  let message = text;
  const m = text.match(/panicked at ([^\n]+?):\s*\n([\s\S]+?)(?:\nnote:|$)/);
  if (m) {
    location = m[1].trim();
    message = m[2].trim();
  }
  const envelope = {
    kind: 'Error',
    at_us: 0,
    source: 'rust-panic',
    ffi_call: null,
    message,
    location,
    recent_trace: [],
    raw_stderr: text,
  };
  postMessage({ kind: 'panic', line: JSON.stringify(envelope) });
}

createTsotModule({
  printErr: (text) => {
    stderrBuffer.push(String(text));
  },
  // Emscripten's abort handler. Fires when the wasm traps (any
  // unreachable instruction, including the one Rust emits after a
  // nounwind panic). Flush the stderr buffer as a rust-panic
  // envelope BEFORE the worker becomes useless.
  onAbort: (reason) => {
    if (stderrBuffer.length === 0 && reason) {
      stderrBuffer.push(String(reason));
    }
    flushStderrAsPanic();
  },
}).then((m) => {
  module = m;
  // Path 1 wiring: pass the shared wasm heap + the wasm-side address
  // of UCT_CANCEL_FLAG to main scope. Main writes to that address via
  // Atomics.store, and wasm reads it via the same atomic each UCT
  // iteration — no postMessage hop, no waiting for the worker to be
  // idle. That's what makes mid-search cancellation actually
  // interrupt UCT. Requires SHARED_MEMORY=1 in the wasm build and
  // COOP/COEP on the dev server.
  const heapBuffer = m.HEAP8 && m.HEAP8.buffer;
  const cancelOffset = m._UCT_CANCEL_FLAG;
  postMessage({
    kind: 'ready',
    sharedHeap: heapBuffer,
    cancelFlagOffset: typeof cancelOffset === 'number' ? cancelOffset : null,
    crossOriginIsolated:
      typeof crossOriginIsolated !== 'undefined' ? crossOriginIsolated : null,
  });
}).catch((e) => {
  postMessage({ kind: 'error', error: 'wasm init: ' + (e && e.message ? e.message : String(e)) });
});

function callWasm(name, argJson) {
  const ptr = module.ccall(name, 'number', ['string'], [argJson]);
  if (ptr === 0) return '';
  const s = module.UTF8ToString(ptr);
  module.ccall('tsot_free_string', null, ['number'], [ptr]);
  return s;
}

function callWasmNoArgs(name) {
  const ptr = module.ccall(name, 'number', [], []);
  if (ptr === 0) return '';
  const s = module.UTF8ToString(ptr);
  module.ccall('tsot_free_string', null, ['number'], [ptr]);
  return s;
}

onmessage = (ev) => {
  const { cmd, args, action } = ev.data;
  // `load_game` carries its payload under `loadArgs` to avoid name
  // collision with `args` (start_game) and `action` (apply_action).
  // Reset the stderr buffer at FFI entry so a panic during THIS
  // call carries only this call's stderr, not bleed-over from a
  // previous call.
  stderrBuffer.length = 0;
  try {
    if (cmd === 'list_card_pool') {
      const json = callWasmNoArgs('tsot_list_card_pool');
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'list_preset_decks') {
      const json = callWasmNoArgs('tsot_list_preset_decks');
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'start_game') {
      const json = callWasm('tsot_start_game', JSON.stringify(args));
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'apply_action') {
      const json = callWasm('tsot_apply_action', JSON.stringify(action));
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'save_game') {
      const json = callWasmNoArgs('tsot_save_game');
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'load_game') {
      const { loadArgs } = ev.data;
      const json = callWasm('tsot_load_game', JSON.stringify(loadArgs));
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'test_panic') {
      // Observability probe — intentionally panics on the Rust
      // side. If the panic hook works, the LOG renders a rich
      // rust-panic block; if not, we see an opaque wasm-trap.
      const json = callWasmNoArgs('tsot_test_panic');
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'preview_uct') {
      // Run UCT on a clone of the session state and return the
      // ranked candidate envelope. Doesn't mutate the live game.
      const { previewArgs } = ev.data;
      const json = callWasm('tsot_preview_uct', JSON.stringify(previewArgs || {}));
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'run_auto_game') {
      // Spectate: both-AI game runs to completion in one ccall and
      // returns the full phase-boundary snapshot timeline for the
      // scrubber. Long-running for UCT-vs-UCT; the LOG still streams
      // live UCT iter events because tsot_emit_iteration_event posts
      // mid-call (see ADR-0001).
      const { autoArgs } = ev.data;
      const json = callWasm('tsot_run_auto_game', JSON.stringify(autoArgs || {}));
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    if (cmd === 'cancel_uct') {
      // Single-threaded-worker caveat: this `onmessage` only fires
      // between FFI calls, not during one. So if a preview is
      // already in flight when 'cancel_uct' arrives, the cancel
      // takes effect for the NEXT search instead. Documented in
      // src/wasm_ffi.rs::tsot_preview_uct_impl.
      const json = callWasmNoArgs('tsot_cancel_uct');
      postMessage({ kind: 'envelope', cmd, json });
      return;
    }
    postMessage({ kind: 'error', error: 'unknown cmd: ' + cmd });
  } catch (e) {
    // Wasm trap (or any other exception inside callWasm). If Rust
    // printed anything to stderr before the trap, flush it as a
    // rust-panic envelope (carries the standard `thread '…' panicked
    // at FILE:LINE:COL: <msg>` format Rust uses for nounwind
    // panics). If stderr is empty (panic=abort can skip the print
    // path entirely), surface whatever the JS exception itself
    // carries — message + stack + name — as a panic envelope so the
    // LOG shows everything we DO have instead of swallowing it.
    if (stderrBuffer.length > 0) {
      flushStderrAsPanic();
    } else {
      const message = (e && e.message) ? String(e.message) : String(e);
      const stack = (e && e.stack) ? String(e.stack) : null;
      const name = (e && e.name) ? String(e.name) : null;
      const envelope = {
        kind: 'Error',
        at_us: 0,
        source: 'wasm-trap',
        ffi_call: cmd || null,
        message,
        location: null,
        recent_trace: [],
        // Everything else we can glean from the JS exception so
        // nothing is hidden. The LOG block surfaces these in the
        // breadcrumb area; the developer can read them in-place.
        js_stack: stack,
        js_error_name: name,
      };
      postMessage({ kind: 'panic', line: JSON.stringify(envelope) });
    }
    postMessage({ kind: 'error', error: (e && e.message) || String(e) });
  }
};
