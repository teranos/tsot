import type { GlyphPersistence } from '@qntx/glyphs';

const DB_NAME = 'tsot';
const DB_VERSION = 1;
const STORE = 'state';
const KEY = 'game';

interface GameState {
  minimizedGlyphs: string[];
  manifestedAt: Record<string, number>;
  openedFirstTime: Record<string, boolean>;
}

let cache: GameState = {
  minimizedGlyphs: [],
  manifestedAt: {},
  openedFirstTime: {},
};

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function readState(): Promise<GameState | null> {
  return openDb().then(
    (db) =>
      new Promise<GameState | null>((resolve, reject) => {
        const tx = db.transaction(STORE, 'readonly');
        const req = tx.objectStore(STORE).get(KEY);
        req.onsuccess = () => resolve((req.result as GameState | undefined) ?? null);
        req.onerror = () => reject(req.error);
      }),
  );
}

function writeState(): Promise<void> {
  return openDb().then(
    (db) =>
      new Promise<void>((resolve, reject) => {
        const tx = db.transaction(STORE, 'readwrite');
        tx.objectStore(STORE).put(cache, KEY);
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      }),
  );
}

function persist(): void {
  writeState().catch((e) => console.error('[TSOT] persist failed', e));
}

export async function loadState(): Promise<{ isFirstEver: boolean }> {
  const loaded = await readState();
  if (loaded) {
    cache = loaded;
    return { isFirstEver: false };
  }
  return { isFirstEver: true };
}

export const persistence: GlyphPersistence = {
  getMinimizedGlyphs: () => [...cache.minimizedGlyphs],
  addMinimizedGlyph: (id) => {
    if (!cache.minimizedGlyphs.includes(id)) {
      cache.minimizedGlyphs.push(id);
      persist();
    }
  },
  removeMinimizedGlyph: (id) => {
    cache.minimizedGlyphs = cache.minimizedGlyphs.filter((x) => x !== id);
    persist();
  },
};

export function markManifested(command: string): void {
  if (!(command in cache.manifestedAt)) {
    cache.manifestedAt[command] = Date.now();
    persist();
  }
}

export function isManifested(command: string): boolean {
  return command in cache.manifestedAt;
}

export function markOpenedFirstTime(command: string): boolean {
  if (cache.openedFirstTime[command]) return false;
  cache.openedFirstTime[command] = true;
  persist();
  return true;
}
