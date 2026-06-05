// Web Worker scope. Owns the wasm engine + posts events to main.
//
// Protocol:
//   main → worker: { cmd: "list_card_pool" }
//                  { cmd: "list_preset_decks" }
//                  { cmd: "start_game", args: {…} }
//                  { cmd: "apply_action", action: {…} }
//   worker → main: { kind: "ready" }
//                  { kind: "uct_iter", line: "<json>" } // mid-call live event
//                  { kind: "envelope", cmd, json: "<JSON payload>" }
//                  { kind: "error", error: "<message>" }
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

createTsotModule({}).then((m) => {
  module = m;
  postMessage({ kind: 'ready' });
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
    postMessage({ kind: 'error', error: 'unknown cmd: ' + cmd });
  } catch (e) {
    postMessage({ kind: 'error', error: (e && e.message) || String(e) });
  }
};
