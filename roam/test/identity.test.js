// roam — identity load + worker bootstrap.
//
// The load-bearing invariant under test:
//   when IDB-load rejects, no `cmd: 'init'` is ever posted to the
//   worker, and `setNetState('error')` IS called. This is what
//   distinguishes the 0.3.2 hard-fail behaviour from the 0.3.1
//   silent ephemeral fallback that the user only noticed when
//   they saw PeerId rotating across reloads.
//
// Run: `bun test` from `roam/`.
// Mutation-verifiable: revert the catch in `identity.js`
// `bootstrapIdentityToWorker` back to posting `{ cmd: 'init',
// identity_bytes: new Uint8Array(0) }` and the second test fails.

import { describe, test, expect } from 'bun:test';
import {
  loadOrMintIdentity,
  bootstrapIdentityToWorker,
  IDENTITY_DB_NAME,
  IDENTITY_STORE_NAME,
  IDENTITY_KEY,
} from '../assets/src/identity.js';

// Minimal IDBRequest-shaped object. The browser fires `onsuccess` or
// `onerror` after the caller assigns the handler; our fakes match
// that order so the assignment-then-fire dance in loadOrMintIdentity
// resolves the same way as it would against a real IDB.
const fireSuccess = (req, result) => {
  req.result = result;
  queueMicrotask(() => req.onsuccess && req.onsuccess());
};
const fireError = (req, error) => {
  req.error = error;
  queueMicrotask(() => req.onerror && req.onerror());
};

function makeStoredEntry(bytes) {
  let stored = bytes;
  return {
    open(_name, _version) {
      const req = {};
      const dbObjectStores = ['identity'];
      const db = {
        objectStoreNames: { contains: (n) => dbObjectStores.includes(n) },
        createObjectStore() {},
        close() {},
        transaction(_store, _mode) {
          return {
            objectStore() {
              return {
                get(_key) {
                  const r = {};
                  fireSuccess(r, stored);
                  return r;
                },
                put(value, _key) {
                  stored = value;
                  const r = {};
                  fireSuccess(r);
                  return r;
                },
              };
            },
          };
        },
      };
      fireSuccess(req, db);
      return req;
    },
  };
}

describe('loadOrMintIdentity', () => {
  test('returns stored bytes on hit', async () => {
    const stored = new Uint8Array([1, 2, 3, 4]);
    const idb = makeStoredEntry(stored);
    const log = () => {};
    const mintBytes = () => {
      throw new Error('mint must not run when IDB hit');
    };
    const out = await loadOrMintIdentity({ idb, mintBytes, log });
    expect(out).toBe(stored);
  });

  test('mints + stores on miss', async () => {
    const idb = makeStoredEntry(undefined);
    const log = () => {};
    const minted = new Uint8Array([9, 8, 7, 6]);
    const mintBytes = () => minted;
    const out = await loadOrMintIdentity({ idb, mintBytes, log });
    expect(out).toBe(minted);
  });

  test('rejects when IDB.open errors (the failure that 0.3.2 sacred-fails on)', async () => {
    const idb = {
      open() {
        const req = {};
        fireError(req, new Error('simulated IDB unavailable'));
        return req;
      },
    };
    const log = () => {};
    const mintBytes = () => {
      throw new Error('mint must not run when IDB.open fails');
    };
    await expect(
      loadOrMintIdentity({ idb, mintBytes, log }),
    ).rejects.toThrow('simulated IDB unavailable');
  });
});

describe('bootstrapIdentityToWorker — hard-fail invariant', () => {
  test('on load success, posts cmd:init with identity bytes', async () => {
    const identityBytes = new Uint8Array([1, 2, 3]);
    const posted = [];
    const stateChanges = [];
    await bootstrapIdentityToWorker({
      load: async () => identityBytes,
      postMessage: (m) => posted.push(m),
      setNetState: (s) => stateChanges.push(s),
      bootstrap: ['/dns4/relay.test/tcp/443/wss'],
      log: () => {},
    });
    expect(posted).toHaveLength(1);
    expect(posted[0].cmd).toBe('init');
    expect(posted[0].identity_bytes).toBe(identityBytes);
    expect(stateChanges).toEqual([]);
  });

  // THE F2-class equivalent for identity. If a future change reverts
  // the catch to a silent ephemeral fallback, this test fails. Without
  // it, the 0.3.1 bug shape can return and only manifest as PeerId
  // rotation that nobody notices for days.
  test('on load failure: NO cmd:init posted AND setNetState(error) called', async () => {
    const posted = [];
    const stateChanges = [];
    await bootstrapIdentityToWorker({
      load: async () => {
        throw new Error('IDB blew up');
      },
      postMessage: (m) => posted.push(m),
      setNetState: (s) => stateChanges.push(s),
      bootstrap: ['/dns4/relay.test/tcp/443/wss'],
      log: () => {},
    });
    expect(posted).toHaveLength(0);
    expect(stateChanges).toEqual(['error']);
  });
});

describe('schema constants', () => {
  // Pin the IndexedDB schema so a future rename doesn't silently
  // strand every existing user's identity in the orphan v1 store.
  test('database/store/key names match the deployed schema', () => {
    expect(IDENTITY_DB_NAME).toBe('roam');
    expect(IDENTITY_STORE_NAME).toBe('identity');
    expect(IDENTITY_KEY).toBe('v1');
  });
});
