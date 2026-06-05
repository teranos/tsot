// JS-side resolution for the `tsot_emit_iteration_event` extern
// declared in `src/sim/uct.rs`. Invoked by wasm after each UCT
// iteration to post a live event back to the main thread for
// rendering. Runs in worker scope (the worker spawns this wasm
// module and inherits this library), so `postMessage` here goes
// from worker → main.
mergeInto(LibraryManager.library, {
  tsot_emit_iteration_event: function(ptr, len) {
    const json = UTF8ToString(ptr, len);
    postMessage({ kind: 'uct_iter', line: json });
  },
});
