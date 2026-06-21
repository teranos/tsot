// roam v0.3 — cross-browser peer visibility via js-libp2p over WebRTC.
// Bundled by `bun build` into dist/js-bridge.js.
//
// Errors-as-first-class (see CLAUDE.md): no silent catches. Every
// caught error pushes a TraceEvent to the on-page log with the
// message + stack. window.onerror + unhandledrejection capture
// anything we missed.

// js-libp2p substrate retired in 0.3.2 — rust-libp2p (Web Worker) is
// the only network path. All gossipsub / transport / discovery work
// happens in `assets/src/net-worker.js`; this file's networking role
// is the `WorkerBridge` postMessage relay (callbacks handed to
// `roam_net_init`).
import initWasm, {
  roam_init,
  roam_tick,
  roam_state,
  roam_viewport_write,
  roam_viewport_ptr,
  roam_color_table_ptr,
  roam_color_table_len,
  roam_pixels_per_tile,
  roam_tick_count,
  roam_tick_blocked_count,
  roam_state_read_count,
  roam_viewport_read_count,
  roam_set_position,
  roam_drain_trace,
  roam_drain_errors,
  roam_session_snapshot,
  roam_restore_session,
  roam_render_init,
  roam_render_frame,
  roam_player_state_ptr,
  roam_player_state_len,
  roam_net_init,
  roam_net_init_rust_libp2p,
  roam_net_publish_position,
  roam_net_tick,
  roam_net_peer_count,
  roam_net_peer_state_seq,
  roam_net_peers_json,
  roam_net_generate_identity_bytes,
} from '/roam.js';

// Network substrate: rust-libp2p in a Web Worker. The dual-path
// design (?provider=js fallback) was retired in 0.3.2; only the
// worker path remains. WorkerBridge in roam Rust holds the
// NetworkProvider; this file wires its callbacks to net-worker.js.

// At-a-glance network state, driven into the colored dot on #status
// (see play.html CSS `#status.net-*::before`). Five buckets:
//   idle   — substrate hasn't started yet (page just loaded)
//   init   — wasm/transport handshake in flight
//   ready  — substrate up, no peer connection yet
//   peers  — at least one connected peer in mesh
//   error  — substrate reported a fatal error
let netState = 'init';
const NET_STATE_TOOLTIPS = {
  idle:  'idle — substrate hasn\'t started yet',
  init:  'init — wasm or transport handshake in flight',
  ready: 'ready — substrate up, no peer connection yet',
  peers: 'peers — at least one connected peer in the gossipsub mesh',
  error: 'error — substrate failed; reload page to retry',
};
function setNetState(next) {
  if (netState === next) return;
  // Stuck-in-error guard: don't drop back to "ready" or "init" without
  // proof of peer connection — once we're red, only the next reload or
  // an actual peer:up should turn the dot.
  if (netState === 'error' && next !== 'peers' && next !== 'error') return;
  netState = next;
  // Dot lives on `#world-hud` (the top-of-canvas overlay). That
  // element gets created later in this file (see `worldHud` setup);
  // until then `getElementById` returns null and we just store state
  // — the class gets applied lazily on the next setNetState call once
  // the element exists.
  const el = document.getElementById('world-hud');
  if (el) {
    el.className = `net-${next}`;
    el.title = NET_STATE_TOOLTIPS[next] || next;
  }
}

const INPUT_W = 1 << 0;
const INPUT_A = 1 << 1;
const INPUT_S = 1 << 2;
const INPUT_D = 1 << 3;

const TOPIC = 'roam-positions/v1';
const PEER_TIMEOUT_MS = 2000;
const POST_INTERVAL_MS = 50;

// process.env.DEBUG and console.* hijack are set up by the inline
// script in play.html BEFORE this module's imports resolve. The
// inline script pushes every console call into window.__roamConsoleBuf;
// we drain it on a timer into the visible event log.
function drainConsoleBuf() {
  const buf = (typeof window !== 'undefined' && window.__roamConsoleBuf) || [];
  if (buf.length === 0) return;
  const items = buf.splice(0, buf.length);
  for (const item of items) {
    // Only real signal: warnings and errors. console.debug / .info /
    // .log from libp2p are torrential and add no actionable info —
    // explicit redial OK / dial failures already flow through our own
    // logger. If you need full libp2p debug back, widen this gate.
    if (item.m !== 'warn' && item.m !== 'error') continue;
    let msg;
    try {
      msg = item.args.map((a) => {
        if (typeof a === 'string') return a;
        if (a instanceof Error) return `${a.name}: ${a.message}`;
        try { return JSON.stringify(a); } catch { return String(a); }
      }).join(' ');
    } catch (e) {
      msg = '(unserializable)';
    }
    logEvent(item.m === 'error' ? 'error' : 'info', `console.${item.m}: ${msg.slice(0, 500)}`);
  }
}
setInterval(drainConsoleBuf, 200);

// Production relay (CloudFront → Lightsail). Hardcoded in source —
// NOT read from `dist/relay-multiaddr.txt` at runtime — so dev
// utilities (headless probes, scratch scripts) can't mutate it.
// Identity loaded from AWS Secrets Manager on the relay box; the
// peer-id is stable across deploys because the secret persists.
const PRODUCTION_RELAY = '/dns4/relay.sbvh.nl/tcp/443/wss/p2p/12D3KooWMSVxS7ntMVuvVADgZWMZwsjyYmcZvhnyQAJ53PtSJHpN';

// Dev-time relay override. `?relay=/ip4/127.0.0.1/tcp/9001/ws/p2p/…`
// lets a developer point the substrate at a local relay (started via
// `bun run relay/relay.ts`) without touching the source-of-truth
// constant above. Used to isolate "is this disconnect a path-level
// problem or a libp2p-protocol problem?" — same bridge → worker →
// rust-libp2p stack, different relay endpoint, side-by-side compare.
const RELAY_MULTIADDR =
  new URLSearchParams(location.search).get('relay') || PRODUCTION_RELAY;

const status = document.getElementById('status');
const canvas = document.getElementById('c');
// World canvas now belongs to Rust's WebGL2 renderer. `roam_render_init`
// is called below once wasm is ready; getContext('2d') is intentionally
// NOT called on this element — WebGL2 and 2D are exclusive per canvas.
const clockEl = document.getElementById('clock');

// Live wall-clock so when the user pastes a screenshot we can correlate
// it with the rest of the world (and with each other's timezones).
function tickClock() {
  const d = new Date();
  const t = d.toISOString().slice(11, 23); // HH:MM:SS.mmm
  clockEl.textContent = `${t}  ${d.toString().slice(25, 33)}`; // + tz
}
tickClock();
setInterval(tickClock, 100);
const selfEl = document.getElementById('self');
const connsEl = document.getElementById('conns');
const meshEl = document.getElementById('mesh');
const logEl = document.getElementById('log');
const invEl = document.getElementById('inv');

// Single source of truth for color is roam::teranos via the FFI palette
// table (read once at init). These arrays exist only as labels for the
// HUD; the RGB triplets come from Rust and are looked up by enum
// discriminant against the palette buffer below.
const INV_LABELS = ['red', 'yellow', 'blue', 'purple', 'azure', 'pink', 'glow'];

// Palette table layout, defined by roam::wasm_ffi::roam_color_table_ptr_impl:
// 5 TileKind RGBs, then 7 FlowerColor RGBs, then 3 FlowerCore RGBs.
// JS never invents RGB.
const PALETTE_TILE_OFFSET = 0;
const PALETTE_PETAL_OFFSET = 15;
const PALETTE_CORE_OFFSET = 36;
const PALETTE_LEN = 45;

// Live handle re-acquired from wasm memory at init. We DON'T cache a
// `Uint8Array` view: WebAssembly.Memory.buffer gets replaced (and the
// old one detached) every time wasm grows its heap — common during
// WebGL buffer uploads. Holding a stale view returns undefined and
// produces `rgb(undefined,undefined,undefined)` which CanvasGradient
// silently rejects with "Invalid color".
let wasmMemoryRef = null;
let colorTablePtr = 0;

function paletteBytes(offset, len) {
  if (!wasmMemoryRef) return null;
  return new Uint8Array(wasmMemoryRef.buffer, colorTablePtr + offset, len);
}
function petalRgb(discriminant) {
  const p = paletteBytes(PALETTE_PETAL_OFFSET + discriminant * 3, 3);
  if (!p) return '#fff';
  return `rgb(${p[0]},${p[1]},${p[2]})`;
}
function coreRgb(discriminant) {
  const p = paletteBytes(PALETTE_CORE_OFFSET + discriminant * 3, 3);
  if (!p) return '#fff';
  return `rgb(${p[0]},${p[1]},${p[2]})`;
}

// Inventory rendering — no canvas2D anywhere. Each flower is an inline
// SVG rendered via the browser's native vector pipeline; no
// `createRadialGradient` allocation per icon. The bridge writes the
// markup string once when the inventory changes (cached + change-
// detected) — Rust still owns every RGB and every discriminant.
const INV_ICON_SIZE = 24; // px
const INV_PETAL_R = 3.6;
const INV_PETAL_DIST = 4.3;
const INV_CORE_R = 2.4;
let invLastSignature = '';

function flowerSvg(f, idx) {
  const petal = petalRgb(f.pc);
  const edge = petalRgb(f.pe);
  const core = coreRgb(f.cc);
  const coreEdge = f.ce === 1 ? petal : f.ce === 2 ? edge : '#fff';
  const n = f.n || 5;
  const cx = INV_ICON_SIZE / 2;
  const cy = INV_ICON_SIZE / 2;
  const petalGradId = `ip${idx}`;
  const coreGradId = `ic${idx}`;
  let petals = '';
  for (let k = 0; k < n; k++) {
    const a = -Math.PI / 2 + (k * 2 * Math.PI) / n;
    const px = cx + Math.cos(a) * INV_PETAL_DIST;
    const py = cy + Math.sin(a) * INV_PETAL_DIST;
    petals += `<circle cx="${px.toFixed(2)}" cy="${py.toFixed(2)}" r="${INV_PETAL_R}" fill="url(#${petalGradId})"/>`;
  }
  return (
    `<svg width="${INV_ICON_SIZE}" height="${INV_ICON_SIZE}" viewBox="0 0 ${INV_ICON_SIZE} ${INV_ICON_SIZE}" xmlns="http://www.w3.org/2000/svg">` +
      `<defs>` +
        `<radialGradient id="${petalGradId}"><stop offset="0" stop-color="${petal}"/><stop offset="1" stop-color="${edge}"/></radialGradient>` +
        `<radialGradient id="${coreGradId}"><stop offset="0" stop-color="${core}"/><stop offset="1" stop-color="${coreEdge}"/></radialGradient>` +
      `</defs>` +
      petals +
      `<circle cx="${cx}" cy="${cy}" r="${INV_CORE_R}" fill="url(#${coreGradId})"/>` +
    `</svg>`
  );
}

function renderInventory(inv) {
  if (!invEl || !wasmMemoryRef) return;
  const items = Array.isArray(inv) ? inv : [];
  // Cheap change-detection so we don't rewrite the DOM when nothing
  // changed. innerHTML assignment is expensive (re-parses markup,
  // re-creates all child nodes); skip when identical.
  const sig =
    items.length +
    ':' +
    items.map((f) => `${f.pc},${f.pe},${f.cc},${f.ce},${f.n}`).join('|');
  if (sig === invLastSignature) return;
  invLastSignature = sig;
  if (items.length === 0) {
    invEl.innerHTML = '';
    return;
  }
  let html = '';
  for (let i = 0; i < items.length; i++) {
    html += flowerSvg(items[i], i);
  }
  invEl.innerHTML = html;
}


// --- event log (errors are sacred) ---
// Unbounded log. Truncating to make the panel "tidy" loses information.
// If the log grows huge, "download log" copies the whole thing to
// clipboard / file; the in-page panel can show only the tail but the
// data is never dropped.
//
// localStorage persistence: log survives reloads. Saved every 5s and
// on visibilitychange (tab hide). LocalStorage limit is ~5MB per
// origin — if we approach it, we surface an error rather than
// silently dropping.
const LOG_STORAGE_KEY = 'roam.log.v1';
const LOG_RENDER_TAIL = 500; // how many tail lines the panel renders; the buffer is unbounded
const LOG_PERSIST_INTERVAL_MS = 5000;
let logLines = [];
try {
  const saved = localStorage.getItem(LOG_STORAGE_KEY);
  if (saved) {
    logLines = JSON.parse(saved);
  }
} catch (e) {
  // Don't lose info silently; we'll log this after the panel is wired.
  setTimeout(() => logError('logLines restore', e), 0);
}
function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
// DOM update batching. The previous logEvent rebuilt `logEl.innerHTML`
// on every call, forcing a Layout/Reflow per event. With heavy
// libp2p traffic that became the dominant cost in the Firefox
// profiler (constant Reflow, Style, DoFlushPendingNotifications).
// Now each call appends to logLines and marks the panel dirty; an
// rAF coalesces the DOM flush so multiple events share one Layout.
let __logDomDirty = false;
let __logFlushScheduled = false;
// Dev-tap mirror: every logEvent line is fire-and-forget POSTed to
// `http://localhost:9100/log` so a tail on `/tmp/roam-dev.log` (the
// dev-tap server in `test/dev-tap.ts`) gives live visibility into
// what the page sees, without devtools or screenshots. Silent when
// the server isn't running — `.catch(() => {})` because failing to
// log to the dev-tap shouldn't itself become a logged error.
function devTap(cls, line) {
  try {
    fetch('http://localhost:9100/log', {
      method: 'POST',
      body: `[${cls}] ${line}`,
      keepalive: true,
    }).catch(() => {});
  } catch {}
}

// IDENTITY MENU (roam/IDENTITY.md):
//   A6 — open one browser-based DID wallet UX; note what "verification" looks like.
//   S1 — surface did:key:z6Mk… in the SELF panel alongside PeerId.
//   S2 — "rotate identity" action: clean IndexedDB → mint fresh; confirmation gate.
//   S3 — export keypair: download protobuf-encoded bytes as a text blob.
//   S4 — import keypair: validate, replace IndexedDB entry, restart worker.
//   S5 — sketch a second-device pairing flow (QR encoding of the bytes).
//   M3 — WebAuthn-wraps-Ed25519 so the key never exits the secure enclave.
//   D1 — show "you are: <short-DID>" in the world HUD next to peer count.
//   D3 — sparkline of signed-message rate in LIBP2P CONNECTIONS.
//   D6 — polish rotate-identity confirmation modal copy.
//   D7 — visible confirmation when import succeeds (peerId line lights up briefly).

// Persistent libp2p identity lives in `./identity.js` — extracted
// so the load + worker-bootstrap path is testable in isolation under
// bun:test. See `test/identity.test.js` for the falsifiable contract
// (mocked IDB throws → no init posted to worker, hard-fail surfaced).
import {
  loadOrMintIdentity as _loadOrMintIdentity,
  bootstrapIdentityToWorker as _bootstrapIdentityToWorker,
} from './identity.js';

const loadOrMintIdentity = () => _loadOrMintIdentity({
  idb: indexedDB,
  mintBytes: roam_net_generate_identity_bytes,
  log: logEvent,
});

function logEvent(cls, line, fingerprint) {
  devTap(cls, line);
  const t = new Date().toISOString().slice(11, 23); // ms precision
  // Consecutive-duplicate collapse. High-volume sources (worker trace
  // bus under ?provider=rust) emit dozens of events per second sharing
  // a (kind, tag) signature but differing in seq / timestamp / payload.
  // Without aggregation the human-readable signal is buried under
  // mechanical repetition. When the caller supplies a `fingerprint`
  // and it matches the most-recent entry, bump a count instead of
  // appending a fresh row — the most-recent payload + timestamp win,
  // so the displayed line still reflects current state.
  const last = logLines[logLines.length - 1];
  if (fingerprint && last && last.fingerprint === fingerprint) {
    last.count = (last.count || 1) + 1;
    last.t = t;
    last.line = line;
  } else {
    logLines.push({ cls, t, line, fingerprint, count: 1 });
  }
  __logDomDirty = true;
  if (!__logFlushScheduled) {
    __logFlushScheduled = true;
    requestAnimationFrame(flushLogDom);
  }
}
function flushLogDom() {
  __logFlushScheduled = false;
  if (!__logDomDirty) return;
  __logDomDirty = false;
  const tail = logLines.slice(-LOG_RENDER_TAIL);
  logEl.innerHTML = tail
    .map((e) => {
      const suffix = e.count > 1 ? `  ×${e.count}` : '';
      return `<span class="ev-${e.cls}">${e.t}  ${escapeHtml(e.line)}${suffix}</span>`;
    })
    .join('\n');
  // Setting `scrollTop = scrollHeight` clipped the freshest entry's
  // wrapped continuation under `pre-wrap; word-break: break-all`: the
  // scroll lands using a `scrollHeight` snapshot from before the new
  // line's wrap height was reflected in layout, so the bottom of the
  // multi-visual-line entry ended up below the viewport. Scrolling
  // the last element explicitly into view handles wrapping correctly
  // — the browser measures the element's full layout box.
  const last = logEl.lastElementChild;
  if (last && typeof last.scrollIntoView === 'function') {
    last.scrollIntoView({ block: 'end', behavior: 'instant' });
  } else {
    logEl.scrollTop = logEl.scrollHeight;
  }
}
// Monotonic id source for typed Error envelopes minted on the JS
// side. Distinct from Rust's `err-roam-N` namespace so the two
// counters can't collide in the Elm-side keyed render.
let __roamJsErrId = 0;
function nextJsErrId() {
  __roamJsErrId += 1;
  return `err-js-${__roamJsErrId}`;
}

// Cursor last seen on a click, captured so port-triggered failures
// (no click in flight) can anchor at the most recent interaction.
let __roamLastClickAnchor = null;
window.addEventListener('mousedown', (e) => {
  __roamLastClickAnchor = { x: e.clientX, y: e.clientY };
});

// `popover: false` opts out of the cursor-popover surface for failures
// that are already being handled by a working retry subsystem with its
// own visible status (bootstrap dials, redial driver). They still land
// in the event log; they just don't drag the user's attention away from
// what they were doing. The sacred-errors rule mandates visibility —
// not that every error get the loudest channel. See `roam/CLAUDE.md`
// "Errors are sacred" + parent CLAUDE.md "contextually in points of
// interaction": when the redial-card IS the point of interaction for
// retried network failures, the popover is a duplicate.
function logError(where, err, { popover = true } = {}) {
  // Walk the full cause chain — no depth cap.
  let chain = err;
  const parts = [];
  let depth = 0;
  const seen = new WeakSet();
  while (chain) {
    if (typeof chain === 'object' && seen.has(chain)) {
      parts.push(`${'  '.repeat(depth)}<cycle detected>`);
      break;
    }
    if (typeof chain === 'object') seen.add(chain);
    const name = chain && chain.name ? chain.name : 'Error';
    const msg = chain && chain.message ? chain.message : String(chain);
    parts.push(`${'  '.repeat(depth)}${name}: ${msg}`);
    chain = chain && chain.cause;
    depth++;
  }
  const diag = err && err.diagnostic ? `\n  diagnostic:\n    ${err.diagnostic.replace(/\n/g, '\n    ')}` : '';
  const stack = err && err.stack ? err.stack.split('\n').join('\n  ') : '(no stack)';
  // Historical log line — fallback timeline.
  logEvent('error', `${where}:\n  ${parts.join('\n  ')}${diag}\n  ${stack}`);
  // Sacred path — typed Error envelope through the axiom pipeline.
  if (popover && typeof window.roamPushError === 'function') {
    const titleErr = err && err.message ? err.message : String(err);
    window.roamPushError({
      id: nextJsErrId(),
      severity: 'error',
      context: {
        surface: where,
        ...(__roamLastClickAnchor ? { anchor: __roamLastClickAnchor } : {}),
      },
      title: titleErr,
      why: parts.join('\n'),
      trace: stack.split('\n'),
      at: new Date().toISOString(),
    });
  }
}

// Press `l` to copy the most recent 20 log entries to clipboard.
// Counterpart to `i` for screenshot. Bounded slice (was: full buffer)
// because under `?provider=rust` the log can grow large enough that
// pasting the whole thing into an LLM session causes the receiving
// context to compaction-loop. 20 lines is the practical ceiling for a
// useful diagnostic snippet — enough to see a startup sequence or a
// failure window, small enough to never blow up downstream tooling.
const LOG_COPY_TAIL_LINES = 20;
window.addEventListener('keydown', async (e) => {
  if (e.key !== 'l' || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  const ae = document.activeElement;
  if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) return;
  const tail = logLines.slice(-LOG_COPY_TAIL_LINES);
  const txt = tail.map((e) => `${e.t} [${e.cls}] ${e.line}`).join('\n');
  try {
    await navigator.clipboard.writeText(txt);
    logEvent('info',
      `log → clipboard (last ${tail.length} of ${logLines.length} entries, ${(txt.length / 1024).toFixed(1)}KB)`);
  } catch (err) {
    logError('log copy', err);
  }
});

// Press `k` to clear the log + persistence. For when starting a fresh
// diagnostic and prior noise is in the way.
window.addEventListener('keydown', (e) => {
  if (e.key !== 'k' || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  const ae = document.activeElement;
  if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) return;
  const n = logLines.length;
  logLines = [];
  try { localStorage.removeItem(LOG_STORAGE_KEY); } catch (err) { logError('log clear', err); }
  logEvent('info', `log cleared (${n} entries dropped)`);
});

// Persist log to localStorage every 5s + on tab hide. We never drop
// entries silently; if storage write fails (quota), log a real error.
function persistLog() {
  let lines = logLines;
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      localStorage.setItem(LOG_STORAGE_KEY, JSON.stringify(lines));
      if (attempt > 0) {
        logEvent('info', `log persisted after halving ${attempt}x (kept ${lines.length} of ${logLines.length})`);
      }
      return;
    } catch (err) {
      if (err && (err.name === 'QuotaExceededError' || err.code === 22)) {
        lines = lines.slice(-Math.max(1, Math.floor(lines.length / 2)));
        continue;
      }
      logError('localStorage.setItem(log)', err);
      return;
    }
  }
  logError('localStorage.setItem(log)', new Error(`quota exceeded; dropped log entries (${logLines.length} → ${lines.length})`));
}
setInterval(persistLog, LOG_PERSIST_INTERVAL_MS);
document.addEventListener('visibilitychange', () => {
  if (document.visibilityState === 'hidden') persistLog();
});
window.addEventListener('beforeunload', persistLog);

// Probe the same host:port the WebSocket tried, via fetch(). The
// browser WS API deliberately hides connect-failure reasons; fetch
// surfaces them. Returns a single-line diagnostic that classifies
// the failure mode (TCP refused / server-responds-but-not-WS / CORS
// / etc.) so the user can read the cause directly from the error log.
async function probeRelayHttp(multiaddrStr) {
  const m = multiaddrStr.match(/^\/(ip4|dns4)\/([^/]+)\/tcp\/(\d+)\/ws/);
  if (!m) return `multiaddr unparseable for probe: ${multiaddrStr}`;
  const host = m[2];
  const port = m[3];
  const url = `http://${host}:${port}/`;
  const t0 = performance.now();
  // The relay is WebSocket-only; a plain HTTP GET may hang because
  // the ws server doesn't reply to non-upgrade requests on a short
  // timer. Bound the probe so it always reports something.
  const PROBE_TIMEOUT_MS = 3000;
  try {
    const res = await Promise.race([
      fetch(url, { method: 'GET', mode: 'no-cors', cache: 'no-store' }),
      new Promise((_, rej) => setTimeout(
        () => rej(new Error(`probe timeout after ${PROBE_TIMEOUT_MS}ms`)),
        PROBE_TIMEOUT_MS,
      )),
    ]);
    const ms = (performance.now() - t0).toFixed(0);
    return `fetch(${url}) returned in ${ms}ms (type=${res.type}, status=${res.status || 'opaque'}) — server IS reachable at TCP+HTTP; WS upgrade is the failing step`;
  } catch (e) {
    const ms = (performance.now() - t0).toFixed(0);
    if (e.message && e.message.startsWith('probe timeout')) {
      return `fetch(${url}) hung past ${PROBE_TIMEOUT_MS}ms — TCP connected but server isn't responding to plain HTTP (WS-only server is the usual cause)`;
    }
    return `fetch(${url}) threw after ${ms}ms — ${e.name}: ${e.message} — server NOT reachable at host:port (DNS/TCP failure)`;
  }
}
function short(id) { return id ? id.toString().slice(-12) : '<none>'; }

// Anything thrown that nothing else caught → log it. No silent crashes.
window.addEventListener('error', (e) => logError('window.onerror', e.error || e.message));
window.addEventListener('unhandledrejection', (e) => logError('unhandledrejection', e.reason));

// Flush any console activity that happened before logEvent existed.
drainConsoleBuf();

// Per-second-batched error log: hot-loop errors like "no peers" can
// fire dozens of times/sec. We don't drop any; we aggregate identical
// messages within a 1s window into one entry that carries the count.
const errorBatches = new Map(); // key=`${where}|${msg}` → {count, firstErr, timer}
function batchedError(where, msg, err) {
  const key = `${where}|${msg}`;
  let entry = errorBatches.get(key);
  if (entry) {
    entry.count++;
    return;
  }
  entry = { count: 1, firstErr: err };
  errorBatches.set(key, entry);
  setTimeout(() => {
    errorBatches.delete(key);
    if (entry.count === 1) {
      logError(where, entry.firstErr);
    } else {
      const augmented = entry.firstErr instanceof Error ? entry.firstErr : new Error(msg);
      augmented.message = `${msg} (×${entry.count} in 1s)`;
      logError(where, augmented);
    }
  }, 1000);
}

// --- screenshot-to-clipboard (press `i`) ---
// Handler is synchronous so the keypress's transient activation is
// still valid when we call navigator.clipboard.write. ClipboardItem
// accepts Promise<Blob>; the clipboard API awaits internally without
// losing activation.
window.addEventListener('keydown', (e) => {
  if (e.key !== 'i' || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  const ae = document.activeElement;
  if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) return;

  const blobP = new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (!blob) reject(new Error('canvas.toBlob returned null'));
      else {
        logEvent('info', `screenshot rendered (${canvas.width}×${canvas.height}, ${(blob.size / 1024).toFixed(0)}KB)`);
        resolve(blob);
      }
    }, 'image/png');
  });

  navigator.clipboard.write([new ClipboardItem({ 'image/png': blobP })])
    .then(() => logEvent('info', `screenshot → clipboard`))
    .catch((err) => logError('clipboard.write', err));
});

// --- input ---
// WASD + arrow keys both drive the same four canonical inputs. The
// Set stores the normalized w/a/s/d letters; arrow keys map to those
// before insertion so `inputBits` doesn't have to know about either.
const keys = new Set();
const KEY_MAP = {
  w: 'w', a: 'a', s: 's', d: 'd',
  arrowup: 'w', arrowleft: 'a', arrowdown: 's', arrowright: 'd',
};
window.addEventListener('keydown', (e) => {
  const k = KEY_MAP[e.key.toLowerCase()];
  if (k) { keys.add(k); e.preventDefault(); }
});
window.addEventListener('keyup', (e) => {
  const k = KEY_MAP[e.key.toLowerCase()];
  if (k) { keys.delete(k); e.preventDefault(); }
});
function inputBits() {
  let i = 0;
  if (keys.has('w')) i |= INPUT_W;
  if (keys.has('a')) i |= INPUT_A;
  if (keys.has('s')) i |= INPUT_S;
  if (keys.has('d')) i |= INPUT_D;
  return i;
}

// Phase 2d: the JS-side `remotePeers` Map and the `ingest` ↦ Map
// pipeline used to live here. Both are gone. Incoming pubsub
// messages now flow through `net-shim.js` into the Rust-owned
// `Net.peers` table. The renderer reads peers from Rust; the bridge
// reads counts through `roam_net_peer_count` / `roam_net_peer_state_seq`.

// Start wasm load IMMEDIATELY, in parallel with the libp2p init that
// follows. Without this, the canvas + game loop block on libp2p (which
// can take many seconds), and the page reads as "loading…" the whole
// time. Now the square moves the instant the wasm is ready, no matter
// how long libp2p takes.
status.textContent = 'loading wasm + libp2p in parallel…';
const moduleP = initWasm();

// --- raw WebSocket probe ---
// Pre-libp2p sanity check: open a bare `new WebSocket(ws://host:port/)`
// against each local relay address and log open/error/close/timeout
// events. Cuts the search space cleanly:
//   - probe OPEN + relay-side `[hook:http] upgrade` fires
//     → browser WS API + relay WS server both work; bug is inside
//       @libp2p/websockets browser code (it opens TCP without ever
//       sending the WebSocket Upgrade request)
//   - probe ERROR / no relay-side request event
//     → browser↔server WS handshake itself broken; bug is below libp2p
async function probeRawWebSocket(addrStr) {
  const m = addrStr.match(/^\/(ip4|dns4)\/([^/]+)\/tcp\/(\d+)\/ws/);
  if (!m) return;
  const host = m[2];
  const port = m[3];
  const url = `ws://${host}:${port}/`;
  return new Promise((resolve) => {
    const t0 = performance.now();
    let ws;
    try {
      ws = new WebSocket(url);
    } catch (e) {
      logError(`raw WS new(${url})`, e);
      resolve();
      return;
    }
    const ms = () => (performance.now() - t0).toFixed(0);
    let done = false;
    const finish = (cls, msg) => {
      if (done) return;
      done = true;
      logEvent(cls, `raw WS ${url}: ${msg} (${ms()}ms)`);
      try { ws.close(); } catch {}
      resolve();
    };
    ws.addEventListener('open', () => finish('connect', 'OPEN — browser-side WebSocket handshake succeeded'));
    ws.addEventListener('error', () => finish('error', 'ERROR — browser WS API hides cause; relay-side hook log shows whether upgrade arrived'));
    ws.addEventListener('close', (e) => {
      if (!done) finish('error', `CLOSE before open code=${e.code} reason="${e.reason}" wasClean=${e.wasClean}`);
      else logEvent('info', `raw WS ${url} closed code=${e.code}`);
    });
    setTimeout(() => finish('error', 'TIMEOUT — no open/error/close in 5s'), 5000);
  });
}

// Bootstrap list. Used to be loaded at runtime from
// `dist/relay-multiaddr.txt`, which dev utilities (headless probes,
// scratch scripts) trivially clobbered. Now hardcoded — the only
// way the relay's peer-id changes is a deliberate
// `tofu taint aws_secretsmanager_secret.relay_identity` followed by
// `apply`. Source-of-truth lives in committed code.
const bootstrapList = [RELAY_MULTIADDR];

// Raw-WebSocket probe of each local relay BEFORE libp2p init. Tells
// us if the browser-native WS API can talk to the relay independent
// of any libp2p code. Runs in parallel; we don't await them all (one
// failing probe shouldn't delay libp2p), but each result lands in the
// log when it completes.
for (const addrStr of bootstrapList) {
  if (!addrStr.includes('127.0.0.1') && !addrStr.includes('localhost')) continue;
  probeRawWebSocket(addrStr); // fire and observe
}

// --- HUD counters ---
let publishFailRate = 0; // running count for HUD

// PROVIDER=rust HUD state. The worker substrate emits `net::peer_up`
// and `net::peer_down` traces; the bridge keeps a count so the world-
// HUD can show truthful `peers=N` and a `libp2p (rust) conns=N` status
// instead of the JS-libp2p-instance-or-zero short-circuit that left it
// permanently reading `peers=0  libp2p off` under `?provider=rust`.
let rustPeerCount = 0;
let rustConnCount = 0;
// Liveness counter — incremented for every `net::heartbeat` trace the
// worker emits. Heartbeats themselves are filtered out of the visible
// event log (pure noise; break dedup runs on adjacent error events).
// Surfaced via SELF panel so the worker's pulse is still observable
// without flooding the log.
let rustHeartbeatCount = 0;
// `tick-debug` counters lifted out of the visible log (they broke
// consecutive-duplicate dedup runs of adjacent `provider_error`s).
// Surfaced in the SELF panel for diagnostic completeness.
let rustTickDebugCount = 0;
let rustTickDebugTraceLen = 0;
// Liveness sparkline state. Each second's incoming-event count goes
// into a fixed-size ring buffer; rendered into the otherwise-empty
// #conns panel under ?provider=rust so "is the thing alive" is a
// glance question, not a scroll-through-the-log question.
const SPARK_BIN_COUNT = 60;
const SPARK_BIN_MS = 1000;
const sparkBins = new Array(SPARK_BIN_COUNT).fill(0);
let sparkLastBinStartMs = Date.now();
function sparkRecord() {
  const now = Date.now();
  while (now - sparkLastBinStartMs >= SPARK_BIN_MS) {
    sparkBins.shift();
    sparkBins.push(0);
    sparkLastBinStartMs += SPARK_BIN_MS;
  }
  sparkBins[sparkBins.length - 1] += 1;
}
function sparkRender() {
  const w = 240, h = 36;
  const max = Math.max(1, ...sparkBins);
  const pts = sparkBins
    .map((v, i) => {
      const x = (i / (SPARK_BIN_COUNT - 1)) * w;
      const y = h - (v / max) * (h - 2) - 1;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
  const totalLastMin = sparkBins.reduce((a, b) => a + b, 0);
  const recent = sparkBins[sparkBins.length - 1] || 0;
  return (
    `worker events: ${recent}/s now, ${totalLastMin} last 60s, peak ${max}/s\n` +
    `<svg width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" style="display:block;margin-top:4px">` +
    `<rect width="${w}" height="${h}" fill="#1a1a1a" stroke="#333"/>` +
    `<polyline fill="none" stroke="#6cf" stroke-width="1" points="${pts}"/>` +
    `</svg>`
  );
}
// Most recent `net::peer_down` reason text. Captured at the moment the
// event arrives so the disconnect cause survives the row scrolling out
// of the rendered tail. Surfaced in the SELF panel under
// `?provider=rust`; the value is the libp2p `ConnectionError` debug-
// formatted by the seam in `rust_libp2p.rs::handle_swarm_event`.
let lastPeerDownAt = '';
let lastPeerDownReason = '';
// Worker readiness + identity for the SELF panel. The network
// (gossipsub / transport / dial) lives entirely in the Web Worker
// (`assets/src/net-worker.js`); this file reads readiness flags here.
let workerIdentity = '';
let workerDidKey = '';
let workerReady = false;
// PEER_ID: short device label shown in the SELF panel. The real
// PeerId comes from the worker (`workerIdentity`) once it's ready;
// this is the bootstrap placeholder before the worker reports back.
const PEER_ID = crypto.randomUUID().slice(0, 8);

// Phase 2d: BroadcastChannel (same-browser fallback) was deleted
// when the JS-side peer table went away. Same-browser comms still
// work because the local relay routes them through libp2p loopback;
// the BC path was an extra fallback that didn't survive the seam
// migration. Can be re-added later as a `NetworkProvider` impl
// (`BroadcastChannelProvider`) plugging into the same trait.

// --- wasm loader + game loop ---
// TSOT bridge probe + always-visible state. The SELF panel renders
// `__tsotBridge.state` so we don't have to fish for a log line.
let __tsotBridge = { state: 'pending', cardCount: 0 };
Promise.resolve(window.tsotReady).then((tsot) => {
  if (!tsot) {
    __tsotBridge = { state: 'error: no module', cardCount: 0 };
    logEvent('error', 'tsotReady resolved with no module');
    return;
  }
  let json;
  try {
    json = tsot.ccall('tsot_list_card_pool', 'string', [], []);
  } catch (err) {
    __tsotBridge = { state: 'error: ccall threw', cardCount: 0 };
    logError('tsot_list_card_pool', err);
    return;
  }
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    __tsotBridge = { state: 'error: JSON.parse', cardCount: 0 };
    const wrapped = new Error(`JSON.parse failed on tsot_list_card_pool response`, { cause: err });
    wrapped.diagnostic = `raw response (first 500 chars): ${String(json).slice(0, 500)}`;
    logError('tsot_list_card_pool JSON.parse', wrapped);
    return;
  }
  // Envelope shape from tsot wasm_ffi::wrap_result_envelope:
  // { ok, result: [...], log, trace, errors }
  if (!parsed || parsed.ok !== true) {
    __tsotBridge = { state: 'error: envelope ok=false', cardCount: 0 };
    logError('tsot_list_card_pool envelope', new Error(`ok=${parsed && parsed.ok}`));
    return;
  }
  const arr = Array.isArray(parsed.result) ? parsed.result : null;
  if (!arr) {
    __tsotBridge = { state: 'error: result not array', cardCount: 0 };
    logError('tsot_list_card_pool envelope', new Error(`result shape: ${typeof parsed.result}`));
    return;
  }
  __tsotBridge = { state: 'ready', cardCount: arr.length };
}).catch((err) => {
  __tsotBridge = { state: 'error: promise rejected', cardCount: 0 };
  logError('tsotReady', err);
});

moduleP.then((wasm) => {
  roam_init();

  // wasm-bindgen's init() resolves with the InitOutput object, which
  // exposes the linear memory. Every typed-buffer FFI below reads
  // through views over this single ArrayBuffer.
  const wasmMemory = wasm.memory;
  const PIXELS_PER_TILE = roam_pixels_per_tile();

  // Size the canvas explicitly — the Elm template doesn't carry
  // width/height attrs so the browser default of 300x150 was leaking
  // through. WebGL's framebuffer comes from this backing-store size;
  // CSS scaling won't add resolution. 720x720 is a comfortable square
  // for the M1 + GPU + libp2p combination on the typical dev layout.
  canvas.width = 1440;
  canvas.height = 1440;
  canvas.style.width = '1440px';
  canvas.style.height = '1440px';

  // Status text overlay. The browser renders text well; making Rust
  // load a bitmap font + glyph atlas just so a 12-char debug HUD can
  // live "in WebGL" would be busywork. Use the platform: a DOM element
  // positioned over the canvas top-left, textContent updated per frame.
  // Single source of truth for the format string is in this file, but
  // every datum comes from Rust (state JSON / libp2p / counters).
  const worldHud = document.createElement('div');
  worldHud.id = 'world-hud';
  worldHud.style.cssText = [
    'position: absolute',
    'top: 6px',
    'left: 10px',
    'font: 12px/1.4 ui-monospace, Menlo, monospace',
    'color: #fff',
    'background: rgba(0, 0, 0, 0.55)',
    'padding: 4px 8px',
    'border-radius: 4px',
    'pointer-events: none',
    'white-space: pre',
    'z-index: 5',
  ].join(';');
  if (canvas.parentElement) {
    canvas.parentElement.style.position = 'relative';
    canvas.parentElement.appendChild(worldHud);
  }
  // Apply the current net-state class + tooltip now that the element
  // exists. setNetState's earlier calls (from before worldHud was
  // created) were no-ops; this catches up.
  worldHud.className = `net-${netState}`;
  worldHud.title = NET_STATE_TOOLTIPS[netState] || netState;

  // Hand the world canvas to Rust's WebGL2 renderer. From this point
  // on, every world-canvas pixel comes from `roam_render_frame`. The
  // JS bridge issues no draw calls against the world canvas; canvas2D
  // and WebGL2 are exclusive per canvas, and we've committed to GL.
  try {
    roam_render_init(canvas);
    logEvent('info', `render_gl: WebGL2 attached to world canvas (${canvas.width}x${canvas.height})`);
  } catch (err) {
    logError('render_gl init', err);
  }

  // Network seam. rust-libp2p Swarm lives in a Web Worker, not the
  // main thread. The main-thread `Net` uses `WorkerBridge`'s five-
  // callback shape; the callbacks below postMessage to the worker.
  // See `assets/src/net-worker.js` for the worker side.
  {
    logEvent('info', 'spawning net-worker');
    let netWorker;
    try {
      netWorker = new Worker(new URL('./net-worker.js', location.href), { type: 'module' });
      logEvent('info', `net-worker spawned at ${new URL('./net-worker.js', location.href)}`);
    } catch (err) {
      logError('net-worker spawn', err);
      throw err;
    }
    // workerIdentity and workerReady are hoisted to module scope so
    // the SELF panel renderer can read them.
    const incomingMessageBuffer = [];

    netWorker.addEventListener('message', (e) => {
      const msg = e.data || {};
      switch (msg.kind) {
        case 'ready':
          workerIdentity = msg.identity || '';
          workerDidKey = msg.did_key || '';
          workerReady = true;
          setNetState('ready');
          logEvent('info', `net: net-worker ready, identity=${workerIdentity} did=${workerDidKey || '(none)'}`);
          // The initial `subscribe` command was sent eagerly (below) at
          // bridge boot, before the worker was ready, and got silently
          // dropped on the worker side (`if (!initialized) break;`).
          // Re-subscribe now that the worker can answer. Gossipsub
          // tolerates duplicate subscribes; this is the cheap idempotent
          // catch-up.
          netWorker.postMessage({ cmd: 'subscribe', topic: 'roam-positions/v1' });
          break;
        case 'events':
          for (const m of msg.messages || []) {
            incomingMessageBuffer.push(m);
          }
          break;
        case 'traces':
          // Mirror the worker's trace bus into the main thread log. The
          // worker has its own `roam_drain_trace` thread-local, separate
          // from main thread's.
          //
          // Also drive the rust-provider HUD counters from this stream:
          // `net::peer_up` / `net::peer_down` are the seam's source of
          // truth for connection state. Reading them here keeps the HUD
          // honest under `?provider=rust` (where the JS-libp2p instance
          // is null and `libp2p.getConnections()` doesn't exist).
          try {
            const events = JSON.parse(msg.json);
            for (const e of events) {
              const ev = e.event;
              sparkRecord();
              if (ev.kind === 'Note') {
                if (ev.tag === 'net::peer_up') {
                  rustConnCount += 1;
                  rustPeerCount = rustConnCount;
                } else if (ev.tag === 'net::peer_down') {
                  rustConnCount = Math.max(0, rustConnCount - 1);
                  rustPeerCount = rustConnCount;
                  // Capture the reason text now so the disconnect cause
                  // survives the row scrolling out of the rendered tail.
                  // Surfaced in the SELF panel.
                  lastPeerDownAt = new Date().toISOString().slice(11, 23);
                  lastPeerDownReason = ev.msg || '(no reason in event)';
                }
                // net::heartbeat is pure liveness signal — fires once per
                // libp2p heartbeat tick, semantically empty. The SELF
                // panel's `rust trace: ticks=N` and the periodic
                // `net-worker tick alive` lines already prove the worker
                // is running. Rendering heartbeats in the event log buys
                // nothing and breaks consecutive-duplicate dedup runs for
                // adjacent provider_errors, making the log unreadable
                // under any non-trivial error rate. Counted, not shown.
                if (ev.tag === 'net::heartbeat') {
                  rustHeartbeatCount += 1;
                  continue;
                }
              }
              // Fingerprint by (kind, tag) for Note events so repeated
              // taggings (net::provider_error) collapse into a single
              // row with a count.
              const fp = ev.kind === 'Note' && ev.tag
                ? `worker:Note:${ev.tag}`
                : `worker:${ev.kind}`;
              // Display line. Stringifying the entire event object put
              // the machinery (`{"kind":"Note","tag":"..."}`) at the
              // left, the actually-useful payload (`msg`) past the
              // panel's truncation column, so every line looked
              // identical. Lead with the tag, then the msg. For non-
              // Note kinds or events without a msg, fall back to the
              // raw object so we don't silently drop information.
              let line;
              if (ev.kind === 'Note' && ev.tag) {
                const msg = ev.msg && String(ev.msg).length > 0
                  ? ` ${ev.msg}`
                  : '';
                line = `${ev.tag}${msg}`;
              } else {
                line = `${ev.kind} ${JSON.stringify(ev)}`;
              }
              // Color-code by tag using the existing #log .ev-* CSS
              // classes (defined in play.html). Grey-on-grey for every
              // event made the log unskimmable; this lets the eye land
              // on disconnects (red) and connects (green) at a glance.
              let cls = 'info';
              if (ev.kind === 'Note' && ev.tag) {
                if (ev.tag === 'net::peer_up') cls = 'connect';
                else if (ev.tag === 'net::peer_down') cls = 'disconnect';
                else if (ev.tag === 'net::provider_error') cls = 'error';
                else if (ev.tag === 'net::sub_change') cls = 'sub';
              }
              logEvent(cls, line, fp);
            }
          } catch (err) {
            logError('net-worker traces parse', err);
          }
          break;
        case 'tick-debug':
          // Liveness ping from the worker — fires every batched tick
          // (~1/sec wall time). Pure noise in the visible log AND
          // breaks consecutive-duplicate dedup runs of adjacent
          // `provider_error` rows. Counted, not rendered. The SELF
          // panel `rust trace: ticks=N` line already surfaces tick
          // count; `tick-debug` carried only `count` + `traceLen`,
          // both diagnostic-only.
          rustTickDebugCount = msg.count;
          rustTickDebugTraceLen = msg.traceLen;
          break;
        case 'lifecycle':
          logEvent('info', `net-worker lifecycle: ${msg.stage}`);
          break;
        case 'capability': {
          // Capability snapshot from the worker BEFORE wasm init.
          // Hypothesis test for "WebRTC isn't available in workers";
          // result lands in the log so we don't have to rely on
          // memory or MDN.
          const rtc = msg.hasRTCConstruct ? 'OK'
            : msg.hasRTCType ? `type-exists-but-construct-throws: ${msg.constructError}`
            : 'absent';
          // RTCPeerConnection absence in workers is a known, accepted
          // constraint — the transport stack dropped libp2p-webrtc-
          // websys in `ef62411`. Surfacing this through `logError`
          // raised a red "sacred error" overlay for what is documented,
          // expected behavior. The `logEvent('info', ...)` line above
          // already records the capability snapshot for diagnostics.
          logEvent('info',
            `net-worker capability: RTCPeerConnection=${rtc}, WebSocket=${msg.hasWebSocket ? 'OK' : 'absent'}, ua="${msg.userAgent}"`);
          break;
        }
        case 'error': {
          // Worker errors carry stack + source location from the
          // worker's `self.onerror` / `unhandledrejection`. Build a
          // synthetic Error so logError's cause-chain + stack
          // formatter renders the full context in #log.
          const err = new Error(msg.message || '(no message)');
          if (msg.stack) err.stack = msg.stack;
          if (msg.filename) {
            err.diagnostic = `at ${msg.filename}:${msg.line || '?'}:${msg.col || '?'}`;
          }
          setNetState('error');
          logError(`net-worker ${msg.where}`, err);
          break;
        }
        default:
          logEvent('info', `net-worker unknown msg: ${JSON.stringify(msg)}`);
      }
    });
    netWorker.addEventListener('error', (e) => logError('net-worker onerror', e.error || e.message));

    // Persistent identity: pull the libp2p-canonical protobuf-encoded
    // Ed25519 keypair from IndexedDB (`roam/identity/v1`), mint via
    // wasm + persist on first visit. The bytes get passed to the
    // worker so its PeerId stays stable across sessions — same key
    // serves the libp2p identity AND the future v0.4 collection-
    // owner identity.
    //
    // Failure path is HARD-FAIL per "errors are sacred". IDB-locked,
    // private-browsing quota, schema corruption, or the wasm mint
    // export missing all land here — the previous fallback was a
    // silent ephemeral mint, which made a failure look identical to
    // the working path until someone noticed the PeerId rotating on
    // every reload. We refuse to bring the network up at all and let
    // the user see the red dot + the log line; reload retries.
    _bootstrapIdentityToWorker({
      load: loadOrMintIdentity,
      postMessage: (m) => netWorker.postMessage(m),
      setNetState,
      bootstrap: bootstrapList,
      log: (cls, msg) => (cls === 'error' ? logError(msg, new Error('identity bootstrap failed')) : logEvent(cls, msg)),
    });

    // JsLibp2pProvider callbacks routed through the worker.
    const selfPeerIdFn = () => workerIdentity;
    const publishFn = (topic, bytes) => {
      if (!workerReady) return;
      // bytes is a Uint8Array view over wasm memory — copy into a
      // structured-clonable array before postMessage so the wasm
      // buffer isn't aliased into the worker's address space.
      const arr = Array.from(bytes);
      netWorker.postMessage({ cmd: 'publish', topic, bytes: arr });
    };
    const subscribeFn = (topic) => {
      if (!workerReady) return;
      netWorker.postMessage({ cmd: 'subscribe', topic });
    };
    const unsubscribeFn = (topic) => {
      if (!workerReady) return;
      netWorker.postMessage({ cmd: 'unsubscribe', topic });
    };
    const drainEventsFn = () => {
      if (incomingMessageBuffer.length === 0) return '[]';
      const json = JSON.stringify(incomingMessageBuffer);
      incomingMessageBuffer.length = 0;
      return json;
    };

    // Eager Net init. The first subscribe command queued here arrives
    // at the worker before it's done with wasm init; the worker's
    // onmessage handler drops it (`if (!initialized) break;`). The
    // 'ready' branch above resends the subscribe once it actually has
    // a provider. Publishes that go out before 'ready' are also
    // dropped, but `publishFn` short-circuits with `if (!workerReady)
    // return;` so they fail silently — gossipsub on the rust side will
    // pick up at the first published position once the worker is up.
    try {
      roam_net_init(selfPeerIdFn, publishFn, subscribeFn, unsubscribeFn, drainEventsFn);
      logEvent('info', `net: seam initialized via net-worker (worker init in flight), bootstrap=${bootstrapList.length} addr(s)`);
    } catch (err) {
      logError('roam_net_init (worker path)', err);
    }
  }

  // Color palette is Rust-owned. We capture the memory handle + table
  // pointer here, but every read goes through `paletteBytes()` which
  // re-acquires a `Uint8Array` view against the CURRENT buffer — wasm
  // heap growth detaches old buffers, so a cached view goes stale.
  wasmMemoryRef = wasmMemory;
  colorTablePtr = roam_color_table_ptr();
  const COLOR_TABLE_LEN = roam_color_table_len();
  if (COLOR_TABLE_LEN !== PALETTE_LEN) {
    logError(
      'palette',
      new Error(`Rust color table length ${COLOR_TABLE_LEN} disagrees with JS PALETTE_LEN ${PALETTE_LEN}`),
    );
  }

  // Viewport layout — locked by roam::viewport's `#[repr(C)]` structs.
  // If those structs change in Rust, the const assertions there fail
  // at compile time before the new bytes can reach this side.
  const VIEWPORT_HEADER_SIZE = 32;
  const VIEWPORT_TILE_SIZE = 8;
  const VIEWPORT_OFF_TILE_KIND = 0;
  const VIEWPORT_OFF_ELEV = 1;
  const VIEWPORT_OFF_HAS_FLOWER = 2;
  const VIEWPORT_OFF_PETAL_CENTER = 3;
  const VIEWPORT_OFF_PETAL_EDGE = 4;
  const VIEWPORT_OFF_CORE_CENTER = 5;
  const VIEWPORT_OFF_CORE_EDGE = 6;
  const VIEWPORT_OFF_PETAL_COUNT = 7;

  const tick     = roam_tick;
  const state    = roam_state;
  const setPos   = roam_set_position;
  const drainTr  = roam_drain_trace;
  const drainErr = roam_drain_errors;

  // Restore last known position from localStorage if present. Wasm
  // re-snaps z to the local surface, so even if terrain generation
  // changed between sessions the player can't land inside a wall.
  const SAVE_KEY = 'roam_player_pos_v1';
  const SESSION_KEY = 'roam_session_v1';
  // setPos takes WORLD-PIXEL coordinates; tile-center = (tile + 0.5) *
  // PIXELS_PER_TILE. PIXELS_PER_TILE is already in scope from the
  // earlier `roam_pixels_per_tile()` call (line ~904) — reuse that.
  // Expose a tile-coord helper so JS (URL params, buttons, future
  // commands) speaks tiles end-to-end.
  const tileToPixel = (t) => (t + 0.5) * PIXELS_PER_TILE;
  window.roamTeleport = function (tx, ty, facing) {
    const f = Number.isFinite(facing) ? facing : 4;
    const px = tileToPixel(tx);
    const py = tileToPixel(ty);
    setPos(px, py, f);
    logEvent('info', `teleport → tile (${tx}, ${ty}) px (${px.toFixed(1)}, ${py.toFixed(1)}) f=${f}`);
  };

  // URL teleport: `?x=NNN&y=NNN[&f=N]` — x and y are TILE coordinates,
  // not world pixels. (Earlier version of this code took pixels and
  // confused everyone who tried it, including the developer who wrote
  // it.) Overrides the localStorage restore. Spawn is tile (16, 16).
  const _url = new URLSearchParams(location.search);
  const _tx = _url.get('x');
  const _ty = _url.get('y');
  if (_tx !== null && _ty !== null) {
    const tx = parseFloat(_tx);
    const ty = parseFloat(_ty);
    const f = parseInt(_url.get('f') ?? '4', 10);
    if (Number.isFinite(tx) && Number.isFinite(ty)) {
      window.roamTeleport(tx, ty, Number.isFinite(f) ? f : 4);
    } else {
      logError('URL teleport', new Error(`invalid x/y: x=${_tx} y=${_ty}`));
    }
  } else {
    try {
      const raw = localStorage.getItem(SAVE_KEY);
      if (raw) {
        const p = JSON.parse(raw);
        if (typeof p.x === 'number' && typeof p.y === 'number') {
          setPos(p.x, p.y, typeof p.f === 'number' ? p.f : 4);
          logEvent('info', `restored position (${p.x.toFixed(1)}, ${p.y.toFixed(1)}) f=${p.f ?? 4}`);
        }
      }
    } catch (err) {
      logError('localStorage restore', err);
    }
  }
  // Restore picked-set + inventory so flowers stay picked across reload.
  try {
    const raw = localStorage.getItem(SESSION_KEY);
    if (raw) {
      roam_restore_session(raw);
      logEvent('info', `restored session snapshot (${raw.length} bytes)`);
    }
  } catch (err) {
    logError('localStorage session restore', err);
  }
  let lastSave = 0;
  const SAVE_INTERVAL_MS = 1000;

  // Facing index → (dx, dy) unit vector for the player indicator.
  const FACING_VEC = [
    [0, -1], [0.707, -0.707], [1, 0], [0.707, 0.707],
    [0, 1], [-0.707, 0.707], [-1, 0], [-0.707, -0.707],
  ];

  let lastPost = 0;
  let last = performance.now();
  let lastInfoUpdate = 0;

  // Dirty-flag render. The frame loop runs at RAF rate to keep input
  // and wasm physics flowing, but the canvas only repaints when
  // something visible has changed. `lastRenderFp` is a short
  // fingerprint of every input the render depends on; mismatch → paint.
  // An idle heartbeat (4 fps when nothing moves) handles day-night
  // brightness drift so the world doesn't freeze in time.
  let lastRenderFp = '';
  let lastRenderAt = 0;
  const IDLE_RENDER_MIN_MS = 250;

  // Performance HUD. Two independent counters, both measured honestly:
  //   raf Hz   — frame() callbacks per second (input responsiveness)
  //   gl Hz    — actual paints per second (visible motion)
  // Plus average elapsed time per frame and per paint, rolling
  // every-second so the numbers are stable enough for a human to read
  // without lying about quiet moments. The previous version measured
  // "FPS" only on the paint path which collapsed to ~0 when the
  // dirty-flag was skipping; that lied about frame-loop liveness.
  const perfHud = document.createElement('div');
  perfHud.id = 'perf-hud';
  perfHud.style.cssText = [
    'position: absolute',
    'top: 6px',
    'right: 10px',
    'font: 12px/1.4 ui-monospace, Menlo, monospace',
    'color: #fff',
    'background: rgba(0, 0, 0, 0.55)',
    'padding: 4px 8px',
    'border-radius: 4px',
    'pointer-events: none',
    'white-space: pre',
    'z-index: 5',
    'text-align: right',
  ].join(';');
  canvas.parentElement?.appendChild(perfHud);

  let perfWindowStartMs = performance.now();
  let perfRafCount = 0;
  let perfRenderCount = 0;
  let perfFrameMsSum = 0;
  let perfRenderMsSum = 0;

  // Day/night cycle mirrors teranos::day_phase + teranos::brightness.
  // Keep these in sync with the Rust constants — they're load-bearing.
  const WORLD_CIRC_X_TILES = 4096;
  const DAY_LENGTH_SECS_JS = 18000; // 5h
  const NIGHT_FLOOR = 0.25; // midnight isn't pitch black; vision system handles real darkness

  // Vision radii (tiles). Mirror teranos::VISION_R_*.
  const VISION_R_DAY = 12.0;
  const VISION_R_NIGHT = 4.0;
  const VISION_R_UNDERGROUND = 3.0;
  function dayBrightness(playerXPixels, tilePixels) {
    const nowSecs = Date.now() / 1000;
    const tileX = playerXPixels / tilePixels;
    const lonFrac = (((tileX % WORLD_CIRC_X_TILES) + WORLD_CIRC_X_TILES) % WORLD_CIRC_X_TILES) / WORLD_CIRC_X_TILES;
    const phase = ((nowSecs / DAY_LENGTH_SECS_JS) + lonFrac) % 1.0;
    const theta = (phase - 0.25) * Math.PI * 2;
    const c = (Math.cos(theta) + 1.0) * 0.5; // 0..1, peak at noon
    return NIGHT_FLOOR + (1.0 - NIGHT_FLOOR) * c;
  }

  // Zoom multiplies the rendered tile size. Wider viewport at low zoom
  // (more world tiles fit on screen); larger tiles at high zoom.
  let zoom = 1.0;
  const ZOOM_STEP = 1.25;
  const ZOOM_MIN = 0.4;
  const ZOOM_MAX = 4.0;
  window.addEventListener('keydown', (e) => {
    if (e.key === '-' || e.key === '_') {
      zoom = Math.max(ZOOM_MIN, zoom / ZOOM_STEP);
      e.preventDefault();
    } else if (e.key === '+' || e.key === '=') {
      zoom = Math.min(ZOOM_MAX, zoom * ZOOM_STEP);
      e.preventDefault();
    }
  });

  function frame(now) {
    const frameStartMs = performance.now();
    const dt = Math.min(now - last, 100);
    last = now;
    perfRafCount += 1;

    tick(inputBits(), dt);

    // Drive the Rust-owned network state: drain incoming events,
    // update the peer table, prune stale peers. Cheap when nothing's
    // queued. `Date.now()` is the same epoch the shim stamps on
    // incoming messages, so prune math compares like for like.
    try {
      roam_net_tick(Date.now());
    } catch (err) {
      logError('roam_net_tick', err);
    }

    // Binary player state — 16 bytes. Replaces the per-frame
    // JSON.parse(state()) which was a measurable cost in the
    // profiler. Layout is defined in roam::wasm_ffi alongside the
    // FFI export. Inventory + libp2p HUD updates still read the
    // JSON shape but only on the throttled 500ms cadence below.
    const psPtr = roam_player_state_ptr();
    const psView = new DataView(wasmMemoryRef.buffer, psPtr, roam_player_state_len());
    const s = {
      x: psView.getFloat32(0, true),
      y: psView.getFloat32(4, true),
      z: psView.getInt32(8, true),
      f: psView.getUint8(12),
    };

    // Drain rare narrative events (Init, Note, Overflow) from the
    // trace bus. Per-frame Tick / StateRead / ViewportRead are no
    // longer events — they're atomic counters in Rust, read on
    // demand below when the HUD updates. Removing the per-frame
    // events from the bus is the biggest single CPU win in the
    // observability path.
    try {
      const traceJson = drainTr();
      const events = JSON.parse(traceJson);
      for (const e of events) {
        const ev = e.event;
        logEvent('info', `rust:${ev.kind} seq=${e.seq} ${JSON.stringify(ev)}`);
      }
    } catch (err) {
      logError('wasm trace drain/parse', err);
    }

    // Drain wasm-side typed Errors and forward each to Elm through
    // the sacred-error port. Anchor falls back to the last click
    // position when the Rust side didn't supply one.
    try {
      const errs = JSON.parse(drainErr());
      for (const e of errs) {
        if (e && e.context && !e.context.anchor && __roamLastClickAnchor) {
          e.context.anchor = __roamLastClickAnchor;
        }
        if (typeof window.roamPushError === 'function') {
          window.roamPushError(e);
        }
      }
    } catch (err) {
      logError('wasm error drain/parse', err);
    }

    // Persist position + session snapshot to localStorage every
    // SAVE_INTERVAL_MS so the player returns to their last known
    // spot, picked flowers stay picked, and inventory survives.
    if (now - lastSave > SAVE_INTERVAL_MS) {
      try {
        localStorage.setItem(SAVE_KEY, JSON.stringify({ x: s.x, y: s.y, f: s.f }));
      } catch (err) {
        logError('localStorage save', err);
      }
      try {
        localStorage.setItem(SESSION_KEY, roam_session_snapshot());
      } catch (err) {
        logError('localStorage session save', err);
      }
      lastSave = now;
    }

    if (now - lastPost >= POST_INTERVAL_MS) {
      // Cross-browser position broadcast runs through the network
      // seam: Rust holds the application-layer `Net` (in World),
      // calls `provider.publish` which dispatches to `net-shim.js`,
      // which calls `pubsub.publish`. Wire format owned by Rust.
      roam_net_publish_position();
      lastPost = now;
    }

    // Stale-peer pruning lives in Rust now (`Net::tick`), driven by
    // the `roam_net_tick(Date.now())` call above. No JS-side peer
    // table to walk.

    // Status text reflects the actual connection state every frame —
    // String template is cheap; render unconditionally so the
    // #status line never lies about state between sample windows.
    // Network lives entirely in the Web Worker; the FFI exposes
    // peer count via `roam_net_peer_count()`.
    const peerCount = roam_net_peer_count();
    status.textContent = `me=${PEER_ID} — rust-libp2p peers=${peerCount}`;

    if (now - lastInfoUpdate > 500) {
      lastInfoUpdate = now;
      // Inventory — provider-agnostic. Was nested inside the
      // `if (libp2p)` block which meant `?provider=rust` (sentinel-
      // skips libp2p) silently dropped every inventory repaint.
      // The state JSON is owned by Rust's `world::state_json` and
      // doesn't depend on which network substrate is running.
      let sJson = null;
      try {
        sJson = JSON.parse(state());
      } catch (err) {
        logError('wasm state JSON parse (hud)', err);
      }
      const inv = (sJson && Array.isArray(sJson.inv)) ? sJson.inv : [];
      try {
        renderInventory(inv);
      } catch (err) {
        logError('renderInventory', err);
      }
      // Net dot reconciliation (provider-agnostic). Peer count is
      // Rust-owned and reflects whichever substrate is active:
      // JsLibp2pProvider for `?provider=js`, worker proxy for
      // `?provider=rust`. Once we're connected to a peer, stay
      // green; if we lose them all, drop back to cyan (substrate
      // healthy, just nobody else on the line). Doesn't override
      // an `error` state — that needs a reload.
      try {
        const npc = roam_net_peer_count();
        if (npc > 0) setNetState('peers');
        else if (netState === 'peers') setNetState('ready');
      } catch {}
      // Worker-substrate panels: counters + sparkline + last peer_down.
      const lastDown = lastPeerDownReason
        ? `<div style="color:#fbb;background:#310;border-left:3px solid #f44;padding:2px 6px;margin-top:4px">last peer_down: ${lastPeerDownAt} — ${escapeHtml(lastPeerDownReason)}</div>`
        : '';
      connsEl.innerHTML =
        `peers: <b>${rustPeerCount}</b>  conns: <b>${rustConnCount}</b>  heartbeats: ${rustHeartbeatCount}  ticks: ${rustTickDebugCount}\n` +
        sparkRender() +
        lastDown;
      // M5 — the peer table with did:key per peer. The trust line is
      // libp2p's gossipsub Strict validation, which already proved
      // the signed source matches each PeerId; the did:key here is
      // that same key re-encoded for application-layer routing.
      // Empty (Err on Rust side) → blank slot + sacred error in log.
      try {
        const peersRaw = roam_net_peers_json();
        const peers = JSON.parse(peersRaw);
        if (peers.length === 0) {
          meshEl.textContent = '(no remote peers)';
        } else {
          meshEl.textContent = peers.map((p) => {
            const did = p.did_key ? p.did_key : '(did decode failed)';
            const age = Math.max(0, Math.floor((Date.now() - p.last_seen_ms) / 100) * 100);
            return `${did}\n  peerId: ${p.peer_id}\n  pos: (${p.x.toFixed(1)}, ${p.y.toFixed(1)}) facing=${p.facing} last_seen=${age}ms ago`;
          }).join('\n');
        }
      } catch (err) {
        logError('roam_net_peers_json render', err);
        meshEl.textContent = '(peers panel error — see log)';
      }
      const lastDownLine = lastPeerDownReason
        ? `last peer_down: ${lastPeerDownAt}  ${lastPeerDownReason}\n`
        : '';
      selfEl.textContent =
        `display id: ${PEER_ID}\n` +
        `libp2p (rust): ${workerReady ? 'ready' : 'starting'}\n` +
        `worker peerId: ${workerIdentity || '(awaiting ready signal)'}\n` +
        `did:key:    ${workerDidKey || '(awaiting ready signal)'}\n` +
        `peers: ${rustPeerCount}  conns: ${rustConnCount}  heartbeats: ${rustHeartbeatCount}\n` +
        lastDownLine +
        `tsot bridge: ${__tsotBridge.state}${__tsotBridge.state === 'ready' ? ` (${__tsotBridge.cardCount} cards)` : ''}\n` +
        `rust trace: ticks=${roam_tick_count()} stateReads=${roam_state_read_count()} viewportReads=${roam_viewport_read_count()} collisions=${roam_tick_blocked_count()}\n` +
        `log: ${logLines.length} entries (press l=copy last 20, k=clear, i=screenshot)`;
    }

    // Camera centers on player. World wraps in x (cylinder) — wasm
    // handles the wrap on its side; remote peers near the seam don't
    // yet render adjacent to a player on the other side. v0.3.5 ship.
    const camCenterX = canvas.width / 2;
    const camCenterY = canvas.height / 2;

    // Dirty-flag fingerprint. Includes every input the render
    // depends on; a string mismatch triggers a repaint. Peer
    // positions are summed into one rolling value to keep the
    // fingerprint short. The integer truncation on x/y means
    // sub-pixel jitter doesn't force a redraw, but any actual
    // movement does.
    // Peer-table state lives in Rust; a single monotonic counter
    // bumps on every insert / remove / position update / prune. Fold
    // it into the fingerprint instead of walking a JS Map.
    const peerSeq = workerReady ? roam_net_peer_state_seq() : 0;
    const fp = `${s.x | 0},${s.y | 0},${s.z},${s.f},${zoom},${canvas.width},${canvas.height},${peerSeq}`;
    const idleHeartbeat = now - lastRenderAt > IDLE_RENDER_MIN_MS;
    if (fp === lastRenderFp && !idleHeartbeat) {
      // Honest accounting: still counts as a RAF tick (input is alive)
      // but not a paint. raf Hz stays > 0 even when nothing renders.
      perfFrameMsSum += performance.now() - frameStartMs;
      maybeUpdatePerfHud();
      requestAnimationFrame(frame);
      return;
    }
    lastRenderFp = fp;
    lastRenderAt = now;
    const renderStartMs = performance.now();

    // Dynamic viewport size based on canvas + zoom. +2 margin for
    // partially-visible edge tiles; rounded up to even so half-width
    // splits cleanly around the player tile.
    const tilePx = PIXELS_PER_TILE * zoom;
    const wNeed = Math.ceil(canvas.width / tilePx) + 2;
    const hNeed = Math.ceil(canvas.height / tilePx) + 2;
    const viewW = wNeed + (wNeed & 1);
    const viewH = hNeed + (hNeed & 1);

    // Ask Rust to (re)write the typed viewport buffer. The GL
    // renderer reads it directly out of wasm memory; JS never
    // touches the bytes.
    const vbLen = roam_viewport_write(viewW, viewH);
    if (vbLen === 0) {
      requestAnimationFrame(frame);
      return;
    }

    // Phase 2d: Rust owns the entire frame AND the peer table.
    // `roam_render_frame` reads peers from `Net.peers` internally;
    // no `roam_set_peers` call from JS. Pixel decisions live in Rust.
    const dayB = dayBrightness(s.x, PIXELS_PER_TILE);
    try {
      roam_render_frame(s.x, s.y, s.f, zoom, canvas.width, canvas.height, dayB);
    } catch (err) {
      logError('roam_render_frame', err);
    }

    // Status text overlay: rendered by the browser as DOM, not by GL.
    // Bridge composes the format string from Rust-supplied data only.
    // The network lives entirely in the Web Worker; the HUD reads
    // its state from `rustPeerCount` / `rustConnCount` (maintained
    // by the worker-message handler from `net::peer_up` / `net::peer_down`).
    const libStatus = !workerReady
      ? 'libp2p (rust) starting'
      : rustConnCount > 0
        ? `libp2p (rust) conns=${rustConnCount}`
        : 'libp2p (rust) ready';
    worldHud.textContent =
      `me=${PEER_ID} (${s.x.toFixed(1)}, ${s.y.toFixed(1)}, z=${s.z}) f=${s.f}  ` +
      `zoom=${zoom.toFixed(2)}  peers=${rustPeerCount}  ${libStatus}`;

    // Honest perf accounting: this branch is the paint path.
    perfRenderCount += 1;
    const renderEndMs = performance.now();
    perfFrameMsSum += renderEndMs - frameStartMs;
    perfRenderMsSum += renderEndMs - renderStartMs;
    maybeUpdatePerfHud();

    requestAnimationFrame(frame);
  }

  function maybeUpdatePerfHud() {
    const nowMs = performance.now();
    const elapsedMs = nowMs - perfWindowStartMs;
    if (elapsedMs < 1000) return;
    const rafHz = (perfRafCount * 1000) / elapsedMs;
    const glHz = (perfRenderCount * 1000) / elapsedMs;
    const frameMs = perfRafCount > 0 ? perfFrameMsSum / perfRafCount : 0;
    const glMs = perfRenderCount > 0 ? perfRenderMsSum / perfRenderCount : 0;
    perfHud.textContent =
      `game loop  ${rafHz.toFixed(0)}/s   ${frameMs.toFixed(1)}ms each\n` +
      `repaints   ${glHz.toFixed(0)}/s   ${glMs.toFixed(1)}ms each`;
    perfWindowStartMs = nowMs;
    perfRafCount = 0;
    perfRenderCount = 0;
    perfFrameMsSum = 0;
    perfRenderMsSum = 0;
  }

  status.textContent = `me=${PEER_ID} — bootstrapping libp2p…`;
  requestAnimationFrame(frame);
}).catch((err) => {
  logError('wasm load', err);
  status.textContent = 'wasm load failed (see event log)';
});
