// roam — persistent libp2p identity.
//
// Keys live in IndexedDB under (db: `roam`, store: `identity`, key
// `v1`) as a libp2p-canonical protobuf-encoded Ed25519 keypair.
//
// Sacred-error per roam/CLAUDE.md: an IDB failure DOES NOT fall
// through to "mint ephemeral and keep going." It hard-fails so the
// red dot + log line surface the problem. The previous behaviour
// (silent ephemeral) made an IDB failure indistinguishable from a
// working session right up until someone noticed PeerId rotating.
//
// Module exports the load function AND the bridge bootstrap wrapper
// so they're testable in isolation. `js-bridge.js` imports both.

export const IDENTITY_DB_NAME = 'roam';
export const IDENTITY_STORE_NAME = 'identity';
export const IDENTITY_KEY = 'v1';

/// Read or mint the identity bytes. Deps injected so the function
/// runs in any environment (browser or bun-test mock).
/// - `idb`: an `indexedDB` factory (typically `globalThis.indexedDB`)
/// - `mintBytes`: returns a fresh Uint8Array of protobuf bytes
///   (typically `roam_net_generate_identity_bytes` from /roam.js)
/// - `log`: `(cls: string, msg: string) => void`
export async function loadOrMintIdentity({ idb, mintBytes, log }) {
  const db = await new Promise((resolve, reject) => {
    const req = idb.open(IDENTITY_DB_NAME, 1);
    req.onupgradeneeded = () => {
      const upgrading = req.result;
      if (!upgrading.objectStoreNames.contains(IDENTITY_STORE_NAME)) {
        upgrading.createObjectStore(IDENTITY_STORE_NAME);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  try {
    const existing = await new Promise((resolve, reject) => {
      const tx = db.transaction(IDENTITY_STORE_NAME, 'readonly');
      const store = tx.objectStore(IDENTITY_STORE_NAME);
      const req = store.get(IDENTITY_KEY);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    if (existing instanceof Uint8Array && existing.length > 0) {
      log('info', `identity: loaded from IndexedDB (${existing.length} bytes)`);
      return existing;
    }
    const minted = mintBytes();
    if (!(minted instanceof Uint8Array) || minted.length === 0) {
      throw new Error('mintBytes returned empty');
    }
    await new Promise((resolve, reject) => {
      const tx = db.transaction(IDENTITY_STORE_NAME, 'readwrite');
      const store = tx.objectStore(IDENTITY_STORE_NAME);
      const req = store.put(minted, IDENTITY_KEY);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
    log('info', `identity: minted + stored to IndexedDB (${minted.length} bytes)`);
    return minted;
  } finally {
    db.close();
  }
}

/// Bootstrap the worker by loading identity then posting init. Sacred-
/// error path: if the load rejects, surface via `setNetState('error')`
/// + `log` and DO NOT post an init to the worker. Silent ephemeral
/// fallback is what the F-class bug was; the absence of the post is
/// the load-bearing invariant this function commits to.
export async function bootstrapIdentityToWorker({
  load,
  postMessage,
  setNetState,
  bootstrap,
  log,
}) {
  try {
    const identityBytes = await load();
    postMessage({
      cmd: 'init',
      bootstrap_json: JSON.stringify(bootstrap),
      identity_bytes: identityBytes,
    });
  } catch (err) {
    log('error', `identity load/mint — network NOT started: ${err && err.message ? err.message : String(err)}`);
    setNetState('error');
    // Intentional: no postMessage. Hard-fail surfaces; the red
    // dot tooltip prompts a reload.
  }
}
