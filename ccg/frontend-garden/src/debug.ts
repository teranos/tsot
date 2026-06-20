// Live observability for the garden runtime. Renders an in-page
// overlay: counters on top, execution trace below. No DevTools
// required.
//
// Counters are monotonic-truthful: each wraps a suspect API and
// increments on the leaky action, decrements on the cleanup action.
// If a counter climbs and never falls, the wrapped API leaks — proof
// observable on the page.
//
// Trace is a rolling log of edge calls: every IDB open + close,
// every document.addEventListener by type, every wrapped
// renderContent invocation. When the user clicks the page, the
// trace shows exactly which wrapped APIs fired and in what order.
// The leak path becomes a pattern you can read directly.
//
// Reusable: when a new suspect appears, add a counter + wrapper.
// When a new edge needs tracing, call `trace(tag, msg)`.

interface CounterReader {
  name: string;
  read: () => number;
}

const counters: CounterReader[] = [];

function register(name: string, read: () => number): void {
  counters.push({ name, read });
}

// ---------------------------------------------------------------
// Trace ring buffer.
// ---------------------------------------------------------------

const MAX_TRACE = 80;
const traceLines: string[] = [];
const startTime = performance.now();

export function trace(tag: string, msg: string): void {
  const t = Math.round(performance.now() - startTime);
  traceLines.unshift(`${t.toString().padStart(6)}ms  ${tag.padEnd(14)} ${msg}`);
  if (traceLines.length > MAX_TRACE) traceLines.length = MAX_TRACE;
  scheduleRender();
}

// ---------------------------------------------------------------
// 1. IndexedDB open / close.
// ---------------------------------------------------------------

let idbOpenActive = 0;
let idbOpenTotal = 0;
let idbCloseTotal = 0;

const origIdbOpen = indexedDB.open.bind(indexedDB);
(indexedDB as IDBFactory).open = function (
  name: string,
  version?: number,
): IDBOpenDBRequest {
  const req = origIdbOpen(name, version);
  req.addEventListener('success', () => {
    idbOpenActive++;
    idbOpenTotal++;
    trace('idb.open', `${name} (active=${idbOpenActive})`);
    const db = req.result;
    const origClose = db.close.bind(db);
    db.close = function (): void {
      idbOpenActive--;
      idbCloseTotal++;
      trace('idb.close', `${name} (active=${idbOpenActive})`);
      return origClose();
    };
  });
  return req;
} as typeof indexedDB.open;

register('idb.open-active', () => idbOpenActive);
register('idb.open-total', () => idbOpenTotal);
register('idb.close-total', () => idbCloseTotal);

// ---------------------------------------------------------------
// 2. document.addEventListener by type.
// ---------------------------------------------------------------

const docAddByType = new Map<string, number>();
const origDocAdd = document.addEventListener.bind(document);
document.addEventListener = function (
  type: string,
  listener: EventListenerOrEventListenerObject | null,
  options?: boolean | AddEventListenerOptions,
): void {
  docAddByType.set(type, (docAddByType.get(type) ?? 0) + 1);
  trace('doc.listen+', `${type} (count=${docAddByType.get(type)})`);
  return origDocAdd(type, listener, options);
} as typeof document.addEventListener;

const origDocRemove = document.removeEventListener.bind(document);
document.removeEventListener = function (
  type: string,
  listener: EventListenerOrEventListenerObject | null,
  options?: boolean | EventListenerOptions,
): void {
  trace('doc.listen-', type);
  return origDocRemove(type, listener, options);
} as typeof document.removeEventListener;

register('doc.listeners-total', () =>
  Array.from(docAddByType.values()).reduce((a, b) => a + b, 0),
);

// ---------------------------------------------------------------
// 3. Live DOM counts.
// ---------------------------------------------------------------

register('dom.glyph-elements', () => document.querySelectorAll('[data-glyph-id]').length);
register('dom.tsot-card', () => document.querySelectorAll('.tsot-card').length);
register('dom.tsot-card-cell', () => document.querySelectorAll('.tsot-card-cell').length);
register('dom.tsot-card-window', () => document.querySelectorAll('.tsot-card-window').length);

// ---------------------------------------------------------------
// 4. renderContent invocation tracker.
//
// The stash pattern (manifestations/stash.ts) means renderContent
// must fire ONCE per glyph lifetime. If a glyph's count exceeds 1,
// stash was bypassed.
// ---------------------------------------------------------------

const renderCountById = new Map<string, number>();

export function wrapRenderContent<T extends () => HTMLElement>(
  id: string,
  fn: T,
): T {
  return ((): HTMLElement => {
    const n = (renderCountById.get(id) ?? 0) + 1;
    renderCountById.set(id, n);
    trace('renderContent', `${id} (call #${n})`);
    return fn();
  }) as T;
}

register('render-content.unique-glyphs', () => renderCountById.size);
register('render-content.total-calls', () =>
  Array.from(renderCountById.values()).reduce((a, b) => a + b, 0),
);

// ---------------------------------------------------------------
// In-page overlay.
// ---------------------------------------------------------------

let overlay: HTMLElement | null = null;
let renderScheduled = false;

function ensureOverlay(): HTMLElement {
  if (overlay) return overlay;
  const root = document.createElement('div');
  root.id = 'tsot-debug-overlay';
  root.style.cssText = [
    'position:fixed',
    'top:8px',
    'right:8px',
    'z-index:2147483647',
    'width:380px',
    'max-height:90vh',
    'overflow:auto',
    'background:rgba(10,10,12,0.92)',
    'color:#cfd8dc',
    'border:1px solid #2c2c2e',
    'border-radius:8px',
    'padding:8px 10px',
    'font-family:ui-monospace,SFMono-Regular,Menlo,monospace',
    'font-size:10px',
    'line-height:1.4',
    'backdrop-filter:blur(8px)',
    'pointer-events:none',
    'user-select:text',
    'white-space:pre',
  ].join(';');
  document.body.appendChild(root);
  overlay = root;
  return root;
}

function snapshot(): Record<string, number> {
  const out: Record<string, number> = {};
  for (const c of counters) out[c.name] = c.read();
  return out;
}

function formatSnapshot(): string {
  const snap = snapshot();
  const keys = Object.keys(snap);
  const widest = Math.max(...keys.map((k) => k.length));
  return keys.map((k) => `${k.padEnd(widest)}  ${snap[k]}`).join('\n');
}

function renderOverlay(): void {
  renderScheduled = false;
  const el = ensureOverlay();
  const head = `── counters ──────────────\n${formatSnapshot()}`;
  const tail = `\n\n── trace (newest first, last ${MAX_TRACE}) ──\n${traceLines.join('\n')}`;
  el.textContent = head + tail;
}

function scheduleRender(): void {
  if (renderScheduled) return;
  renderScheduled = true;
  requestAnimationFrame(renderOverlay);
}

let intervalId: ReturnType<typeof setInterval> | null = null;

async function postToServer(): Promise<void> {
  try {
    await fetch('/debug', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        t_iso: new Date().toISOString(),
        t_ms: Math.round(performance.now() - startTime),
        url: location.href,
        counters: snapshot(),
        // Trace tail in chronological order (oldest → newest) for the
        // server log; the in-page overlay shows newest-first.
        trace_tail: traceLines.slice().reverse(),
      }),
    });
  } catch {
    // Server may not exist in production. Silent.
  }
}

export function startDebugOverlay(intervalMs: number = 1000): void {
  if (intervalId !== null) return;
  if (document.body) {
    ensureOverlay();
    renderOverlay();
  } else {
    document.addEventListener('DOMContentLoaded', () => {
      ensureOverlay();
      renderOverlay();
    });
  }
  intervalId = setInterval(() => {
    renderOverlay();
    void postToServer();
  }, intervalMs);
}

export function stopDebugOverlay(): void {
  if (intervalId !== null) {
    clearInterval(intervalId);
    intervalId = null;
  }
  if (overlay) {
    overlay.remove();
    overlay = null;
  }
}

// Auto-enable: opt out via `?nodebug` or localStorage.nodebug=1
const optedOut =
  location.search.includes('nodebug') || localStorage.getItem('nodebug') === '1';
if (!optedOut) startDebugOverlay();

trace('boot', 'debug.ts loaded');
