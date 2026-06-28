// IndexedDB-backed Ed25519 identity persistence.
//
// Database `rave`, store `identity`, key `self`. First visit: wasm
// mints fresh bytes and calls __raveSaveIdentity. Every subsequent
// visit: wasm calls __raveLoadIdentity, restores the keypair. Same
// PeerId across sessions = same identity for the libp2p mesh.

import { showErr } from "./overlay";

const DB_NAME = "rave";
const STORE = "identity";
const KEY = "self";

function openRaveDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = (ev) => {
      const db = (ev.target as IDBOpenDBRequest).result;
      if (!db.objectStoreNames.contains(STORE)) {
        db.createObjectStore(STORE);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

export function installIdentityBridges(): void {
  window.__raveLoadIdentity = async (): Promise<Uint8Array | null> => {
    try {
      const db = await openRaveDb();
      return await new Promise<Uint8Array | null>((resolve, reject) => {
        const tx = db.transaction(STORE, "readonly");
        const store = tx.objectStore(STORE);
        const req = store.get(KEY);
        req.onsuccess = () =>
          resolve((req.result as Uint8Array | undefined) ?? null);
        req.onerror = () => reject(req.error);
      });
    } catch (e) {
      showErr(`[__raveLoadIdentity] ${e}`);
      return null;
    }
  };

  window.__raveSaveIdentity = async (bytes: Uint8Array): Promise<void> => {
    try {
      const db = await openRaveDb();
      await new Promise<void>((resolve, reject) => {
        const tx = db.transaction(STORE, "readwrite");
        const store = tx.objectStore(STORE);
        const req = store.put(bytes, KEY);
        req.onsuccess = () => resolve();
        req.onerror = () => reject(req.error);
      });
    } catch (e) {
      showErr(`[__raveSaveIdentity] ${e}`);
    }
  };
}
