// roam v0.3 — cross-browser peer visibility via js-libp2p over WebRTC.
// Bundled by `bun build` into dist/js-bridge.js.
//
// Errors-as-first-class (see CLAUDE.md): no silent catches. Every
// caught error pushes a TraceEvent to the on-page log with the
// message + stack. window.onerror + unhandledrejection capture
// anything we missed.

import { createLibp2p } from 'libp2p';
import { webSockets } from '@libp2p/websockets';
import { webRTC } from '@libp2p/webrtc';
import { circuitRelayTransport } from '@libp2p/circuit-relay-v2';
import { bootstrap } from '@libp2p/bootstrap';
import { noise } from '@chainsafe/libp2p-noise';
import { yamux } from '@chainsafe/libp2p-yamux';
import { identify } from '@libp2p/identify';
import { gossipsub } from '@chainsafe/libp2p-gossipsub';
import { multiaddr } from '@multiformats/multiaddr';
import initWasm, {
  roam_init,
  roam_tick,
  roam_state,
  roam_viewport,
  roam_set_position,
  roam_drain_trace,
  roam_drain_errors,
} from '/roam.js';

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

// Public IPFS bootstrap nodes. Connectivity-only — they route DHT/Bitswap
// traffic but do NOT join our `roam-positions/v1` topic, so they can't
// introduce two browsers to each other for pubsub. The local relay
// (loaded below from /relay-multiaddr.txt if present) is what actually
// makes cross-browser gossip work.
const PUBLIC_BOOTSTRAP = [
  '/dns4/sjc-1.bootstrap.libp2p.io/tcp/443/wss/p2p/QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN',
  '/dns4/sv15.bootstrap.libp2p.io/tcp/443/wss/p2p/QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt',
  '/dns4/ewr1.bootstrap.libp2p.io/tcp/443/wss/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa',
  '/dns4/am6.bootstrap.libp2p.io/tcp/443/wss/p2p/QmSoLer265NRgSp2LA3dPaeykiS1J6DifTC88f5uVQKNAd',
];

const status = document.getElementById('status');
const canvas = document.getElementById('c');
const ctx = canvas.getContext('2d');
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
function logEvent(cls, line) {
  const t = new Date().toISOString().slice(11, 23); // ms precision
  logLines.push({ cls, t, line });
  // Render only the tail to keep DOM cheap; full buffer is in logLines.
  const tail = logLines.slice(-LOG_RENDER_TAIL);
  logEl.innerHTML = tail.map((e) => `<span class="ev-${e.cls}">${e.t}  ${escapeHtml(e.line)}</span>`).join('\n');
  logEl.scrollTop = logEl.scrollHeight;
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

// Press `l` to copy the FULL log buffer (not just the rendered tail)
// to clipboard. Counterpart to `i` for screenshot.
window.addEventListener('keydown', async (e) => {
  if (e.key !== 'l' || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
  const ae = document.activeElement;
  if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'TEXTAREA' || ae.isContentEditable)) return;
  const txt = logLines.map((e) => `${e.t} [${e.cls}] ${e.line}`).join('\n');
  try {
    await navigator.clipboard.writeText(txt);
    logEvent('info', `log → clipboard (${logLines.length} entries, ${(txt.length / 1024).toFixed(1)}KB)`);
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
const keys = new Set();
window.addEventListener('keydown', (e) => {
  const k = e.key.toLowerCase();
  if ('wasd'.includes(k)) { keys.add(k); e.preventDefault(); }
});
window.addEventListener('keyup', (e) => {
  const k = e.key.toLowerCase();
  if ('wasd'.includes(k)) { keys.delete(k); e.preventDefault(); }
});
function inputBits() {
  let i = 0;
  if (keys.has('w')) i |= INPUT_W;
  if (keys.has('a')) i |= INPUT_A;
  if (keys.has('s')) i |= INPUT_S;
  if (keys.has('d')) i |= INPUT_D;
  return i;
}

// --- remote peer table (fed by both transports) ---
const remotePeers = new Map(); // id -> { x, y, z, f, lastSeen, source }
function ingest(msg, source) {
  if (!msg || !msg.peer_id) {
    logEvent('error', `${source}: ignoring msg without peer_id: ${JSON.stringify(msg).slice(0, 80)}`);
    return;
  }
  if (msg.peer_id === PEER_ID) return; // self-echo, fine
  if (typeof msg.x !== 'number' || typeof msg.y !== 'number') {
    logEvent('error', `${source}: ignoring msg with non-number coords from ${msg.peer_id}: ${JSON.stringify(msg).slice(0, 80)}`);
    return;
  }
  const z = typeof msg.z === 'number' ? msg.z : 0;
  const f = typeof msg.f === 'number' ? msg.f : 4; // default south
  remotePeers.set(msg.peer_id, { x: msg.x, y: msg.y, z, f, lastSeen: performance.now(), source });
}

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

// --- local relay discovery ---
// The relay (relay/relay.ts) writes its multiaddr to dist/relay-multiaddr.txt
// on startup. Fetch it at boot and prepend to the bootstrap list so the
// browser tries the relay first. If the file is missing (404), the relay
// isn't running — log it (info, not error) and continue with IPFS bootstrap.
let bootstrapList = [...PUBLIC_BOOTSTRAP];
try {
  const res = await fetch('/relay-multiaddr.txt', { cache: 'no-store' });
  if (res.ok) {
    const lines = (await res.text()).trim().split('\n').filter(Boolean);
    bootstrapList = [...lines, ...bootstrapList];
    logEvent('info', `loaded ${lines.length} local relay multiaddr(s)`);
  } else {
    logEvent('info', `no local relay (HTTP ${res.status} on /relay-multiaddr.txt)`);
  }
} catch (err) {
  logError('relay-multiaddr fetch', err);
}

// Raw-WebSocket probe of each local relay BEFORE libp2p init. Tells
// us if the browser-native WS API can talk to the relay independent
// of any libp2p code. Runs in parallel; we don't await them all (one
// failing probe shouldn't delay libp2p), but each result lands in the
// log when it completes.
for (const addrStr of bootstrapList) {
  if (!addrStr.includes('127.0.0.1') && !addrStr.includes('localhost')) continue;
  probeRawWebSocket(addrStr); // fire and observe
}

// --- libp2p init (cross-browser) ---
let libp2p = null;
let pubsub = null;
let libp2pErr = null;
let publishFailRate = 0; // running count for HUD
logEvent('info', 'creating libp2p node…');
try {
  libp2p = await createLibp2p({
    addresses: { listen: ['/webrtc'] },
    transports: [
      webSockets(),
      webRTC(),
      circuitRelayTransport({ discoverRelays: 1 }),
    ],
    connectionEncrypters: [noise()],
    streamMuxers: [yamux()],
    peerDiscovery: [bootstrap({ list: bootstrapList })],
    // Default gater denies dials to private/loopback addresses in
    // browser mode (security — don't let random pages probe LAN).
    // For local dev we explicitly allow them; revert to default in prod.
    connectionGater: { denyDialMultiaddr: async () => false },
    services: {
      identify: identify(),
      pubsub: gossipsub({ allowPublishToZeroTopicPeers: true, emitSelf: false }),
    },
  });

  libp2p.addEventListener('peer:connect', (e) => logEvent('connect', `peer:connect ${short(e.detail)}`));
  libp2p.addEventListener('peer:disconnect', (e) => logEvent('disconnect', `peer:disconnect ${short(e.detail)}`));
  libp2p.addEventListener('connection:open', (e) =>
    logEvent('connect', `connection:open ${short(e.detail.remotePeer)} via ${e.detail.remoteAddr.toString()}`));
  libp2p.addEventListener('connection:close', (e) =>
    logEvent('disconnect', `connection:close ${short(e.detail.remotePeer)}`));
  libp2p.addEventListener('connection:prune', (e) =>
    logEvent('disconnect', `connection:prune count=${e.detail?.length || 0}`));
  libp2p.addEventListener('peer:discovery', (e) => {
    const addrs = e.detail?.multiaddrs?.map((a) => a.toString()).join(', ') || '(no addrs)';
    logEvent('info', `peer:discovery ${short(e.detail.id)} addrs=${addrs}`);
  });
  libp2p.addEventListener('peer:identify', (e) =>
    logEvent('info', `peer:identify ${short(e.detail?.peerId)} protocols=${(e.detail?.protocols || []).length}`));
  libp2p.addEventListener('self:peer:update', () => {
    const addrs = libp2p.getMultiaddrs().map((a) => a.toString()).join(', ') || '(none)';
    logEvent('info', `self:peer:update addrs=${addrs}`);
  });
  libp2p.addEventListener('transport:listening', (e) =>
    logEvent('info', `transport:listening ${e.detail?.toString?.() || ''}`));
  libp2p.addEventListener('transport:close', (e) =>
    logEvent('info', `transport:close ${e.detail?.toString?.() || ''}`));

  await libp2p.start();
  logEvent('info', `libp2p started, peerId ${libp2p.peerId.toString()}`);

  pubsub = libp2p.services.pubsub;
  pubsub.subscribe(TOPIC);
  logEvent('sub', `subscribed to ${TOPIC}`);

  // Force-dial any local relay addresses. Bootstrap discovery alone
  // doesn't trigger a dial — the connection manager sees the 2 IPFS
  // bootstrap connections, considers itself satisfied, and never
  // contacts the relay. Without the relay subscribed to our topic,
  // the gossipsub mesh never forms.
  // 5s timeout to unblock module init. CRITICAL: the inner libp2p.dial
  // promise is kept alive after the timeout fires, so its eventual
  // resolution (success OR error) lands in the log even if it comes
  // 60s later. Previously we wrapped and discarded — the real cause
  // arrived after our wrapper rejected and was lost.
  const DIAL_TIMEOUT_MS = 5000;
  for (const addrStr of bootstrapList) {
    if (!addrStr.includes('127.0.0.1') && !addrStr.includes('localhost')) continue;
    const ma = multiaddr(addrStr);
    logEvent('info', `force-dialing relay ${addrStr} (timeout ${DIAL_TIMEOUT_MS}ms; inner promise kept alive)`);
    const dialT0 = performance.now();
    const dialPromise = libp2p.dial(ma);
    // Keep observing the inner promise no matter what the wrapper does.
    dialPromise.then(
      (conn) => {
        const ms = (performance.now() - dialT0).toFixed(0);
        logEvent('connect', `relay dial settled (eventually): OK after ${ms}ms peer=${short(conn.remotePeer)}`);
      },
      (err) => {
        const ms = (performance.now() - dialT0).toFixed(0);
        logError(`relay dial settled (eventually) after ${ms}ms`, err, { popover: false });
      },
    );
    try {
      const conn = await Promise.race([
        dialPromise,
        new Promise((_, rej) => setTimeout(
          () => rej(Object.assign(new Error(`wrapper timeout after ${DIAL_TIMEOUT_MS}ms (${addrStr})`), { name: 'DialTimeoutError' })),
          DIAL_TIMEOUT_MS,
        )),
      ]);
      logEvent('connect', `relay dial OK (within timeout): ${short(conn.remotePeer)}`);
    } catch (err) {
      logEvent('info', `dial wrapper threw for ${addrStr}: ${err.name}: ${err.message}`);
      const diag = await probeRelayHttp(addrStr);
      err.diagnostic = diag;
      logError(`dial wrapper ${addrStr}`, err, { popover: false });
    }
  }

  // Redial driver: keep configured bootstrap addresses connected.
  // Every 5 s, check each addr's peer id against libp2p.getConnections();
  // if not connected, attempt dial with exponential backoff per address
  // (5 s → 10 s → 20 s → 40 s, capped at 60 s). On successful dial the
  // delay resets to 5 s.
  const REDIAL_TICK_MS = 5000;
  const REDIAL_BASE_MS = 5000;
  const REDIAL_MAX_MS = 60000;
  const redialState = new Map();
  for (const addrStr of bootstrapList) {
    redialState.set(addrStr, { nextAt: Date.now() + REDIAL_TICK_MS, delayMs: REDIAL_BASE_MS });
  }
  setInterval(async () => {
    if (!libp2p) return;
    const now = Date.now();
    const conns = libp2p.getConnections();
    const connectedPeers = new Set(conns.map((c) => c.remotePeer.toString()));
    for (const addrStr of bootstrapList) {
      const state = redialState.get(addrStr);
      if (!state) continue;
      const m = addrStr.match(/\/p2p\/([^/]+)/);
      if (!m) continue;
      const peerId = m[1];
      if (connectedPeers.has(peerId)) {
        state.delayMs = REDIAL_BASE_MS;
        state.nextAt = now + REDIAL_TICK_MS;
        continue;
      }
      if (now < state.nextAt) continue;
      try {
        await libp2p.dial(multiaddr(addrStr));
        logEvent('connect', `redial OK: ${peerId.slice(-12)}`);
        state.delayMs = REDIAL_BASE_MS;
        state.nextAt = now + REDIAL_TICK_MS;
      } catch (err) {
        state.delayMs = Math.min(state.delayMs * 2, REDIAL_MAX_MS);
        state.nextAt = now + state.delayMs;
        logError(`redial ${peerId.slice(-12)} (next in ${state.delayMs}ms)`, err, { popover: false });
      }
    }
  }, REDIAL_TICK_MS);

  // Liveness check: poll /relay-multiaddr.txt for peer-id changes
  // (signals a relay restart while the tab was open).
  let lastSeenRelayAddr = bootstrapList.find(
    (a) => a.includes('127.0.0.1') || a.includes('localhost'),
  );
  setInterval(async () => {
    try {
      const res = await fetch('/relay-multiaddr.txt', { cache: 'no-store' });
      if (!res.ok) return;
      const lines = (await res.text()).trim().split('\n').filter(Boolean);
      const current = lines[0];
      if (current && current !== lastSeenRelayAddr) {
        logEvent('info', `relay multiaddr changed: ${lastSeenRelayAddr ? short(lastSeenRelayAddr) : '(none)'} → ${short(current)}`);
        lastSeenRelayAddr = current;
      }
    } catch {}
  }, 30000);

  pubsub.addEventListener('subscription-change', (e) => {
    const subs = (e.detail.subscriptions || []).map(s => `${s.subscribe ? '+' : '-'}${s.topic}`).join(' ');
    logEvent('sub', `subscription-change peer=${short(e.detail.peerId)} ${subs}`);
  });

  // Dump advertised protocols + per-connection stream state + gossipsub
  // internal state 4s and 10s after boot. `streamsOutbound.size > 0`
  // tells us gossipsub successfully opened its meshsub stream to the
  // peer (the step that fails silently when something is wrong).
  const dumpState = async (tag) => {
    try {
      const gs = libp2p.services.pubsub;
      const gsOut = gs?.streamsOutbound?.size ?? 'N/A';
      const gsIn = gs?.streamsInbound?.size ?? 'N/A';
      const gsPeers = gs?.peers?.size ?? 'N/A';
      const gsTopics = gs?.subscriptions ? Array.from(gs.subscriptions).join(',') : '(no subscriptions)';
      logEvent('info', `${tag} gossipsub: outStreams=${gsOut} inStreams=${gsIn} peers=${gsPeers} subs=${gsTopics}`);
      const myRec = await libp2p.peerStore.get(libp2p.peerId).catch(() => null);
      const myProtos = myRec?.protocols || [];
      logEvent('info', `${tag} my protocols (${myProtos.length}): ${myProtos.join(', ')}`);
      for (const conn of libp2p.getConnections()) {
        try {
          const rec = await libp2p.peerStore.get(conn.remotePeer);
          const meshsub = (rec.protocols || []).filter(p => p.includes('meshsub') || p.includes('floodsub'));
          const streams = conn.streams || [];
          const streamProtos = streams.map((s) => s.protocol || '?').join(', ');
          logEvent('info', `${tag} peer ${short(conn.remotePeer)} dir=${conn.direction} limits=${conn.limits ? JSON.stringify(conn.limits) : 'none'} protos=${rec.protocols.length} meshsub=[${meshsub.join(',')}] streams=${streams.length} [${streamProtos}]`);
        } catch (e) {
          logError(`peerStore.get(${short(conn.remotePeer)})`, e);
        }
      }
    } catch (err) {
      logError(`${tag} dump`, err);
    }
  };
  setTimeout(() => dumpState('t+4s'), 4000);
  setTimeout(() => dumpState('t+10s'), 10000);

  pubsub.addEventListener('message', (e) => {
    if (e.detail.topic !== TOPIC) return;
    try {
      ingest(JSON.parse(new TextDecoder().decode(e.detail.data)), 'libp2p');
    } catch (err) {
      logError('libp2p msg parse', err);
    }
  });
} catch (err) {
  libp2pErr = err;
  logError('libp2p init', err);
}

const PEER_ID = libp2p
  ? libp2p.peerId.toString().slice(-8)
  : crypto.randomUUID().slice(0, 8);

// --- BroadcastChannel (same-browser fallback) ---
const channel = new BroadcastChannel('roam');
channel.addEventListener('message', (e) => {
  try { ingest(e.data, 'broadcast'); }
  catch (err) { logError('broadcast ingest', err); }
});
channel.addEventListener('messageerror', (e) =>
  logError('BroadcastChannel messageerror', e.data));

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

moduleP.then(() => {
  roam_init();

  const tick     = roam_tick;
  const state    = roam_state;
  const viewport = roam_viewport;
  const setPos   = roam_set_position;
  const drainTr  = roam_drain_trace;
  const drainErr = roam_drain_errors;

  // Restore last known position from localStorage if present. Wasm
  // re-snaps z to the local surface, so even if terrain generation
  // changed between sessions the player can't land inside a wall.
  const SAVE_KEY = 'roam_player_pos_v1';
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
  let lastSave = 0;
  const SAVE_INTERVAL_MS = 1000;

  // Tile kind char → base RGB triple. Matches teranos::TileKind ordering.
  // Pre-split so per-tile shading is a multiply, not a hex parse.
  const TILE_RGB = {
    '0': null,             // Air — no fill (skybox bg shows through)
    '1': [58, 125, 68],    // Grass
    '2': [102, 102, 102],  // Rock
    '3': [92, 160, 200],   // ShallowWater
    '4': [26, 74, 127],    // DeepWater
  };
  // Elevation char → signed surface_z. Mirrors world.rs::elev_char.
  // '!'=-32, 'A'=0, 'a'=32. Decode: code - 33 - 32.
  function decodeElev(ch) {
    return ch.charCodeAt(0) - 33 - 32;
  }
  // Facing index → (dx, dy) unit vector for the player indicator.
  const FACING_VEC = [
    [0, -1], [0.707, -0.707], [1, 0], [0.707, 0.707],
    [0, 1], [-0.707, 0.707], [-1, 0], [-0.707, -0.707],
  ];

  let lastPost = 0;
  let last = performance.now();
  let lastInfoUpdate = 0;
  let rustTickCount = 0;
  let rustStateReadCount = 0;
  let rustCollisionCount = 0;
  let rustViewportReadCount = 0;

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
    const dt = Math.min(now - last, 100);
    last = now;

    tick(inputBits(), dt);
    let s;
    try {
      s = JSON.parse(state());
    } catch (err) {
      logError('wasm state JSON parse', err);
      requestAnimationFrame(frame);
      return;
    }

    // Drain wasm-side trace bus and merge counts into the HUD. Tick,
    // StateRead, and ViewportRead happen at frame rate; logging each
    // one would bury gossipsub diagnostics in noise. The HUD's
    // `rust trace:` line is sufficient; rare events (Init/Note) still
    // get full entries.
    try {
      const traceJson = drainTr();
      const events = JSON.parse(traceJson);
      for (const e of events) {
        const ev = e.event;
        if (ev.kind === 'Tick') {
          rustTickCount++;
          if (ev.blocked_x || ev.blocked_y) rustCollisionCount++;
        } else if (ev.kind === 'StateRead') {
          rustStateReadCount++;
        } else if (ev.kind === 'ViewportRead') {
          rustViewportReadCount++;
        } else {
          logEvent('info', `rust:${ev.kind} seq=${e.seq} ${JSON.stringify(ev)}`);
        }
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

    // Persist position to localStorage every SAVE_INTERVAL_MS so
    // the player returns to their last known spot on reload.
    if (now - lastSave > SAVE_INTERVAL_MS) {
      try {
        localStorage.setItem(SAVE_KEY, JSON.stringify({ x: s.x, y: s.y, f: s.f }));
      } catch (err) {
        logError('localStorage save', err);
      }
      lastSave = now;
    }

    if (now - lastPost >= POST_INTERVAL_MS) {
      const msg = { peer_id: PEER_ID, x: s.x, y: s.y, z: s.z, f: s.f };
      channel.postMessage(msg);
      if (pubsub) {
        const bytes = new TextEncoder().encode(JSON.stringify(msg));
        pubsub.publish(TOPIC, bytes).catch((err) => {
          // Every failure is logged — no rate-limiting. To control
          // log volume we BATCH by second: identical error messages
          // within a 1s window aggregate into one entry with a count.
          // The information of "how many failures, when" is preserved.
          const m = err && err.message ? err.message : String(err);
          batchedError('pubsub.publish', m, err);
        });
      }
      lastPost = now;
    }

    for (const [id, p] of remotePeers) {
      if (now - p.lastSeen > PEER_TIMEOUT_MS) remotePeers.delete(id);
    }

    if (now - lastInfoUpdate > 500) {
      lastInfoUpdate = now;
      if (libp2p) {
        const conns = libp2p.getConnections();
        // Status text reflects the actual connection state — was stuck
        // on "bootstrapping libp2p…" forever before because nothing
        // overwrote the initial value once libp2p connected.
        const meshPeerCount = (pubsub && pubsub.getMeshPeers)
          ? (pubsub.getMeshPeers(TOPIC) || []).length : 0;
        if (conns.length === 0) {
          status.textContent = `me=${PEER_ID} — bootstrapping libp2p…`;
        } else {
          status.textContent =
            `me=${PEER_ID} — libp2p conns=${conns.length} mesh=${meshPeerCount} remote-peers=${remotePeers.size}`;
        }
        connsEl.textContent = conns.length === 0
          ? '(no connections)'
          : conns.map(c => `${short(c.remotePeer)}  ${c.remoteAddr.toString()}`).join('\n');
        if (pubsub) {
          const subs = pubsub.getSubscribers(TOPIC) || [];
          const topics = (pubsub.getTopics && pubsub.getTopics()) || [];
          const meshPeers = (pubsub.getMeshPeers && pubsub.getMeshPeers(TOPIC)) || [];
          const peerList = (pubsub.getPeers && pubsub.getPeers()) || [];
          meshEl.textContent =
            `topic: ${TOPIC}\n` +
            `subscribers (announced): ${subs.length}\n` +
            (subs.length ? subs.map((p) => '  ' + short(p)).join('\n') + '\n' : '') +
            `mesh peers (grafted): ${meshPeers.length}\n` +
            (meshPeers.length ? meshPeers.map((p) => '  ' + short(p)).join('\n') + '\n' : '') +
            `all pubsub peers (any topic): ${peerList.length}\n` +
            (peerList.length ? peerList.map((p) => '  ' + short(p)).join('\n') + '\n' : '') +
            `my subscribed topics: ${topics.length === 0 ? '(none)' : topics.join(', ')}`;
        } else {
          meshEl.textContent = '(pubsub OFF)';
        }
        selfEl.textContent =
          `display id: ${PEER_ID}\n` +
          `libp2p peerId: ${libp2p.peerId.toString()}\n` +
          `multiaddrs:\n  ${libp2p.getMultiaddrs().map(a => a.toString()).join('\n  ') || '(none yet — waiting for relay reservation)'}\n` +
          `tsot bridge: ${__tsotBridge.state}${__tsotBridge.state === 'ready' ? ` (${__tsotBridge.cardCount} cards)` : ''}\n` +
          `rust trace: ticks=${rustTickCount} stateReads=${rustStateReadCount} viewportReads=${rustViewportReadCount} collisions=${rustCollisionCount}\n` +
          `log: ${logLines.length} entries (press l=copy, k=clear, i=screenshot)`;
      } else {
        selfEl.textContent = `display id: ${PEER_ID}\nlibp2p: OFF (${libp2pErr ? libp2pErr.message : 'unknown'})`;
      }
    }

    // Camera centers on player. World wraps in x (cylinder) — wasm
    // handles the wrap on its side; remote peers near the seam don't
    // yet render adjacent to a player on the other side. v0.3.5 ship.
    const camCenterX = canvas.width / 2;
    const camCenterY = canvas.height / 2;

    // Dynamic viewport size based on canvas + zoom. +2 margin for
    // partially-visible edge tiles; rounded up to even so half-width
    // splits cleanly around the player tile.
    const tilePx = 32 * zoom; // matches PIXELS_PER_TILE in world.rs
    const wNeed = Math.ceil(canvas.width / tilePx) + 2;
    const hNeed = Math.ceil(canvas.height / tilePx) + 2;
    const viewW = wNeed + (wNeed & 1);
    const viewH = hNeed + (hNeed & 1);

    let V;
    try {
      V = JSON.parse(viewport(viewW, viewH));
    } catch (err) {
      logError('wasm viewport JSON parse', err);
      requestAnimationFrame(frame);
      return;
    }

    ctx.fillStyle = '#1a1a1f';
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    const tileWorld = V.tile;
    const tileScreen = tileWorld * zoom;
    const halfW = (V.view_w / 2) | 0;
    const halfH = (V.view_h / 2) | 0;
    const playerZ = s.z;
    const dayB = dayBrightness(s.x, tileWorld);

    // Pre-parse elevations into a typed array so cliff-edge neighbor
    // checks are a single index lookup instead of a charCodeAt per side.
    const elevArr = new Int8Array(V.view_w * V.view_h);
    for (let i = 0; i < elevArr.length; i++) {
      elevArr[i] = V.elev.charCodeAt(i) - 33 - 32;
    }

    // Pass 1: tile colors with elevation shading × day/night brightness.
    for (let vy = 0; vy < V.view_h; vy++) {
      for (let vx = 0; vx < V.view_w; vx++) {
        const i = vy * V.view_w + vx;
        const ch = V.tiles[i];
        const rgb = TILE_RGB[ch];
        if (!rgb) continue;
        const tileZ = elevArr[i];
        const elevDiff = tileZ - playerZ;
        const b = Math.max(0.4, Math.min(1.4, 1.0 + elevDiff * 0.04)) * dayB;
        const r = Math.min(255, (rgb[0] * b) | 0);
        const g = Math.min(255, (rgb[1] * b) | 0);
        const bl = Math.min(255, (rgb[2] * b) | 0);
        const worldTx = V.center_tx + vx - halfW;
        const worldTy = V.center_ty + vy - halfH;
        const sx = (worldTx * tileWorld - s.x) * zoom + camCenterX;
        const sy = (worldTy * tileWorld - s.y) * zoom + camCenterY;
        ctx.fillStyle = `rgb(${r},${g},${bl})`;
        ctx.fillRect(sx, sy, tileScreen + 1, tileScreen + 1);
      }
    }

    // Pass 2: cliff demarcation. Same threshold as movement (|Δz| > 1).
    // Drawn on the boundary between two tiles where movement is blocked.
    const edgeThickness = Math.max(2, zoom * 2);
    ctx.fillStyle = 'rgba(0, 0, 0, 0.55)';
    for (let vy = 0; vy < V.view_h; vy++) {
      for (let vx = 0; vx < V.view_w; vx++) {
        const i = vy * V.view_w + vx;
        const e = elevArr[i];
        const worldTx = V.center_tx + vx - halfW;
        const worldTy = V.center_ty + vy - halfH;
        const sx = (worldTx * tileWorld - s.x) * zoom + camCenterX;
        const sy = (worldTy * tileWorld - s.y) * zoom + camCenterY;
        if (vx + 1 < V.view_w) {
          const eR = elevArr[i + 1];
          if (Math.abs(eR - e) > 1) {
            ctx.fillRect(sx + tileScreen - edgeThickness / 2, sy, edgeThickness, tileScreen + 1);
          }
        }
        if (vy + 1 < V.view_h) {
          const eD = elevArr[i + V.view_w];
          if (Math.abs(eD - e) > 1) {
            ctx.fillRect(sx, sy + tileScreen - edgeThickness / 2, tileScreen + 1, edgeThickness);
          }
        }
      }
    }

    // Player + peer marker size scales with zoom, clamped to stay
    // visible at extreme zooms.
    const markerSize = Math.max(8, Math.min(32, 14 * zoom));

    for (const [, p] of remotePeers) {
      const sx = (p.x - s.x) * zoom + camCenterX;
      const sy = (p.y - s.y) * zoom + camCenterY;
      ctx.fillStyle = p.source === 'libp2p' ? '#f6a' : '#fa6';
      ctx.fillRect(sx - markerSize / 2, sy - markerSize / 2, markerSize, markerSize);
    }

    ctx.fillStyle = '#6cf';
    ctx.fillRect(camCenterX - markerSize / 2, camCenterY - markerSize / 2, markerSize, markerSize);
    const [fdx, fdy] = FACING_VEC[s.f] || [0, 1];
    ctx.strokeStyle = '#cfc';
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(camCenterX, camCenterY);
    ctx.lineTo(camCenterX + fdx * markerSize, camCenterY + fdy * markerSize);
    ctx.stroke();

    const conns = libp2p ? libp2p.getConnections().length : 0;
    const libStatus = libp2p ? `libp2p conns=${conns}` : 'libp2p off';
    ctx.fillStyle = '#888';
    ctx.font = '11px ui-monospace, Menlo, monospace';
    ctx.fillText(
      `me=${PEER_ID} (${s.x.toFixed(1)}, ${s.y.toFixed(1)}, z=${s.z}) f=${s.f}  zoom=${zoom.toFixed(2)}  peers=${remotePeers.size}  ${libStatus}`,
      8, 14
    );

    requestAnimationFrame(frame);
  }

  status.textContent = libp2pErr
    ? `me=${PEER_ID} — libp2p failed; BroadcastChannel only`
    : `me=${PEER_ID} — bootstrapping libp2p…`;
  requestAnimationFrame(frame);
}).catch((err) => {
  logError('wasm load', err);
  status.textContent = 'wasm load failed (see event log)';
});
