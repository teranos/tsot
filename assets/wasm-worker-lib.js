// JS-side resolution of Rust externs declared in the engine.
// Linked at wasm-build time via `--js-library=assets/wasm-worker-lib.js`
// in .cargo/config.toml. Functions here run in worker scope (the
// worker spawns this wasm module), so `postMessage` here goes from
// worker → main.
//
// Two externs:
//
// - `tsot_emit_iteration_event` — called from inside
//   `pick_play_uct`'s iteration loop. Lets the LOG render live UCT
//   progress while the FFI is still mid-flight on the worker
//   thread.
//
// - `tsot_emit_panic` — called from the Rust panic hook
//   (`trace::install_panic_hook`, installed by `tsot_wasm::main`)
//   BEFORE the wasm trap aborts. Carries a full
//   `TraceEvent::Panic` envelope: message, location, the FFI call
//   we were inside, and the trace events buffered just before the
//   panic. The whole envelope crosses worker → main; the LOG panel
//   renders it as a distinct error block. Errors are first-class
//   observability events here — nothing is collapsed or hidden.
mergeInto(LibraryManager.library, {
  tsot_emit_iteration_event: function(ptr, len) {
    const json = UTF8ToString(ptr, len);
    postMessage({ kind: 'uct_iter', line: json });
  },
  tsot_emit_panic: function(ptr, len) {
    const json = UTF8ToString(ptr, len);
    postMessage({ kind: 'panic', line: json });
  },
  // "I am alive" signal from the Rust side — used by
  // `install_panic_hook` to confirm in the LOG that the hook
  // actually ran. Distinct kind so the main thread can render it
  // as a non-error event.
  tsot_emit_info: function(ptr, len) {
    const json = UTF8ToString(ptr, len);
    postMessage({ kind: 'info', line: json });
  },
});
