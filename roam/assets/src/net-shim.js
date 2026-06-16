// roam — JS-side libp2p shim.
//
// Phase 2b of the network seam (see roam/src/net/mod.rs). The Rust
// `JsLibp2pProvider` calls these five functions through wasm-bindgen.
// They're the only JS code that knows about the concrete `libp2p` /
// `pubsub` instances; application-layer network logic lives in Rust
// (`roam::net::state::Net`).
//
// JS in this file is "used in spite" per roam/CLAUDE.md: each export
// wraps a browser-API or library call (`pubsub.publish`, `getPeerId`)
// that wasm cannot reach directly. No state, no decisions.

let _libp2p = null;
let _pubsub = null;
const _eventQueue = [];

/// Called by the bridge once libp2p is initialized. Stores the
/// instance references and wires up the incoming-message listener
/// (incoming events are consumed by Rust in phase 2c via `drain`).
export function attach(libp2p, pubsub) {
  _libp2p = libp2p;
  _pubsub = pubsub;
  pubsub.addEventListener('message', (e) => {
    // Phase 2c will deserialize this on the Rust side. The queue
    // shape here is the wire format the Rust `poll_events` parser
    // will accept: { topic, from, bytes_b64, at_ms }.
    const d = e.detail;
    if (!d) return;
    _eventQueue.push({
      topic: d.topic || '',
      from: d.from ? d.from.toString() : '',
      bytes_b64: uint8ToBase64(d.data || new Uint8Array(0)),
      at_ms: Date.now(),
    });
  });
}

export function publish(topicStr, bytes) {
  if (!_pubsub) {
    throw new Error('net-shim publish: pubsub not attached');
  }
  // bytes is a Uint8Array view over wasm memory — copy because the
  // libp2p pubsub publish may hold the reference past the wasm call
  // returning (Promise; the wasm memory could shift under it).
  const owned = new Uint8Array(bytes.length);
  owned.set(bytes);
  // pubsub.publish returns a Promise; fire-and-forget. Failures
  // surface to the sacred-error log via the bridge's existing
  // batchedError handler — we don't re-implement that here.
  _pubsub.publish(topicStr, owned).catch((err) => {
    const msg = err && err.message ? err.message : String(err);
    // Re-route through the global error sink the bridge installed.
    if (typeof window !== 'undefined' && typeof window.roamPushError === 'function') {
      window.roamPushError({
        id: `err-net-${Date.now()}`,
        severity: 'warn',
        context: { surface: 'net-shim.publish' },
        title: 'pubsub.publish failed',
        why: msg,
        at: new Date().toISOString(),
      });
    }
  });
}

export function subscribe(topicStr) {
  if (!_pubsub) throw new Error('net-shim subscribe: pubsub not attached');
  _pubsub.subscribe(topicStr);
}

export function unsubscribe(topicStr) {
  if (!_pubsub) throw new Error('net-shim unsubscribe: pubsub not attached');
  _pubsub.unsubscribe(topicStr);
}

export function selfPeerId() {
  return _libp2p ? _libp2p.peerId.toString() : '';
}

/// Drain queued incoming events as a JSON string. Phase 2b doesn't
/// consume this yet; phase 2c routes it through `Net::tick`.
export function drainEvents() {
  if (_eventQueue.length === 0) return '[]';
  const out = JSON.stringify(_eventQueue);
  _eventQueue.length = 0;
  return out;
}

function uint8ToBase64(bytes) {
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}
