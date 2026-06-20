// Network worker. Loads its own wasm instance + RustLibp2pProvider in
// a browser worker context, so the Swarm's `spawn_local` tasks aren't
// starved by render + Elm + wasm-init contention on the main page
// thread. (Measured starvation on main: max ~9.8s gaps between
// 100ms-Delay wakes — commit fc00b2a heartbeat instrumentation.)
//
// Protocol (postMessage):
//
//   main → worker:
//     { cmd: 'init', bootstrap_json }
//     { cmd: 'publish', topic, bytes: number[] }
//     { cmd: 'subscribe', topic }
//     { cmd: 'unsubscribe', topic }
//
//   worker → main:
//     { kind: 'ready', identity }                       // after init succeeds
//     { kind: 'error', where, message }                 // any failure
//     { kind: 'events', messages: [{ topic, from, bytes, at_ms }] }
//                                                       // periodic, ~50ms cadence
//     { kind: 'traces', json }                          // mirrors `roam_drain_trace`
//
// The main-thread bridge consumes these via the JsLibp2pProvider
// callbacks (selfPeerId, publish, subscribe, unsubscribe, drainEvents).
// The seam between worker and JsLibp2pProvider is intentionally the
// same five-function shape as `net-shim.js` — Rust application code
// in `Net` is unchanged.

// Sacred-error compliance. The worker has its own scope; main-thread
// `window.onerror` and `unhandledrejection` don't see anything that
// happens in here. Without these listeners, a wasm-init failure or
// async throw would leave the user staring at "worker init in flight"
// forever with no signal — exactly the "open devtools" anti-pattern
// CLAUDE.md prohibits. Route every error through `postMessage` so the
// bridge surfaces it in the #log panel.
self.addEventListener('error', (e) => {
  self.postMessage({
    kind: 'error',
    where: 'worker.onerror',
    message: e.message || '(no message)',
    filename: e.filename || '',
    line: e.lineno || 0,
    col: e.colno || 0,
    stack: e.error && e.error.stack ? e.error.stack : '',
  });
});
self.addEventListener('unhandledrejection', (e) => {
  const reason = e.reason;
  self.postMessage({
    kind: 'error',
    where: 'worker.unhandledrejection',
    message: reason && reason.message ? reason.message : String(reason),
    stack: reason && reason.stack ? reason.stack : '',
  });
});

// Capability probe — runs BEFORE wasm init so we know what the
// worker's `globalThis` actually exposes. If `hasRTCConstruct` is
// false, `libp2p-webrtc-websys` will fail or hang in this worker;
// the same diagnostic in Firefox vs Chrome answers the
// "is RTCPeerConnection available in workers" question without
// needing MDN, Bugzilla, or my memory.
(() => {
  const hasRTCType = typeof RTCPeerConnection !== 'undefined';
  let hasRTCConstruct = false;
  let constructError = null;
  if (hasRTCType) {
    try {
      const pc = new RTCPeerConnection({});
      pc.close();
      hasRTCConstruct = true;
    } catch (err) {
      constructError = err && err.message ? err.message : String(err);
    }
  }
  self.postMessage({
    kind: 'capability',
    hasRTCType,
    hasRTCConstruct,
    hasWebSocket: typeof WebSocket !== 'undefined',
    userAgent: (self.navigator && self.navigator.userAgent) || '(no navigator)',
    constructError,
  });
})();

import init, * as roam from '/roam.js';

let initialized = false;
let identity = null;
let tickHandle = null;
const eventQueue = [];

// CRITICAL: register the message handler BEFORE `await init()`.
// The bridge sends `cmd: 'init'` immediately after spawning this
// worker. If the handler isn't attached when the event dispatches,
// browsers MAY silently drop it (observed intermittently in Chrome
// 149). Buffer incoming commands here until wasm + provider are
// ready; the real handler below drains them.
const pendingCmds = [];
let cmdHandler = (msg) => pendingCmds.push(msg);
self.addEventListener('message', (e) => cmdHandler(e.data || {}));

// Lifecycle trace — surfaces "where is the worker right now" in the
// main-thread event log, so a hang between `wasm-init-start` and
// `wasm-init-done` is visible without devtools.
self.postMessage({ kind: 'lifecycle', stage: 'wasm-init-start' });
try {
  await init();
  self.postMessage({ kind: 'lifecycle', stage: 'wasm-init-done' });
} catch (err) {
  self.postMessage({
    kind: 'error',
    where: 'wasm-init',
    message: err && err.message ? err.message : String(err),
    stack: err && err.stack ? err.stack : '',
  });
  throw err;
}

// Now that wasm is up, swap the buffer-only handler for the real one
// and drain anything that arrived during init.
const realHandler = async (msg) => {
  try {
    switch (msg.cmd) {
      case 'init': {
        self.postMessage({ kind: 'lifecycle', stage: 'init-msg-received' });
        self.postMessage({ kind: 'lifecycle', stage: 'provider-init-start' });
        // identity_bytes is a Uint8Array (or empty array if the bridge
        // had nothing in IndexedDB). Pass through to wasm; empty means
        // "generate fresh" — the bridge persists the resulting PeerId
        // by minting via roam_net_generate_identity_bytes first, so
        // the empty path here is only hit in fault flows.
        const identityBytes = msg.identity_bytes instanceof Uint8Array
          ? msg.identity_bytes
          : new Uint8Array(msg.identity_bytes || []);
        identity = roam.roam_net_worker_provider_init(msg.bootstrap_json, identityBytes);
        self.postMessage({ kind: 'lifecycle', stage: 'provider-init-done' });
        initialized = true;
        self.postMessage({ kind: 'ready', identity });
        startTickLoop();
        break;
      }
      case 'publish': {
        if (!initialized) {
          self.postMessage({ kind: 'error', where: 'publish', message: 'worker not initialized' });
          break;
        }
        // bytes arrived as a plain array via postMessage; rebuild as
        // Uint8Array so wasm-bindgen receives the expected shape.
        const bytes = msg.bytes instanceof Uint8Array ? msg.bytes : new Uint8Array(msg.bytes);
        roam.roam_net_worker_provider_publish(msg.topic, bytes);
        break;
      }
      case 'subscribe': {
        if (!initialized) break;
        roam.roam_net_worker_provider_subscribe(msg.topic);
        break;
      }
      case 'unsubscribe': {
        if (!initialized) break;
        roam.roam_net_worker_provider_unsubscribe(msg.topic);
        break;
      }
      default: {
        self.postMessage({
          kind: 'error',
          where: 'onmessage',
          message: `unknown cmd: ${JSON.stringify(msg)}`,
        });
      }
    }
  } catch (err) {
    self.postMessage({
      kind: 'error',
      where: msg.cmd || 'onmessage',
      message: err && err.message ? err.message : String(err),
    });
  }
};
// Swap the buffer handler for the real one, then drain anything that
// arrived during wasm init. From here on, message events go straight
// to `realHandler` via `cmdHandler`.
cmdHandler = (msg) => { realHandler(msg); };
self.postMessage({
  kind: 'lifecycle',
  stage: `drain-pending-cmds count=${pendingCmds.length}`,
});
for (const m of pendingCmds.splice(0)) realHandler(m);

function startTickLoop() {
  if (tickHandle !== null) return;
  // 50ms drain cadence — matches the main thread's render/tick budget.
  // Worker has its own event loop, so this stays close to 50ms even
  // when the page is busy.
  let tickCount = 0;
  tickHandle = setInterval(() => {
    tickCount += 1;
    try {
      const messagesJson = roam.roam_net_worker_provider_drain_events();
      if (messagesJson && messagesJson !== '[]') {
        // Forward as a structured object so the main side doesn't have
        // to parse twice. The bridge's drainEvents callback re-stringifies.
        const messages = JSON.parse(messagesJson);
        if (messages.length > 0) {
          self.postMessage({ kind: 'events', messages });
        }
      }
      const traceJson = roam.roam_drain_trace();
      if (traceJson && traceJson !== '[]') {
        self.postMessage({ kind: 'traces', json: traceJson });
      }
      // Lifeness probe — post every 40 ticks (~2s) so the main thread
      // can see whether the tick loop is actually running.
      if (tickCount % 40 === 0) {
        self.postMessage({ kind: 'tick-debug', count: tickCount, traceLen: traceJson ? traceJson.length : 0 });
      }
    } catch (err) {
      self.postMessage({
        kind: 'error',
        where: 'tick',
        message: err && err.message ? err.message : String(err),
      });
    }
  }, 50);
}
