// Headless cross-substrate probe.
//
// Drives `make wasm-serve`'s output through two Chromium tabs — one
// default (js-libp2p) and one with `?provider=rust` (rust-libp2p) —
// and dumps each tab's event log + sacred-error trace to stdout.
//
// Replaces the "reload + screenshot + paste" loop. Reads the same
// `#log` panel the user reads; the value-add is just that it's
// scriptable, runs unattended, and the output is greppable.
//
// Prerequisites:
//   - `make wasm` has run (dist/ contains js-bridge.js, roam_bg.wasm, ...)
//   - bun deps installed (`bun install` — Playwright fetches Chromium
//     into ~/.cache/ms-playwright on first run; ~150MB one-time).
//   - `caddy` on PATH (already provided by the Nix dev shell).
//
// Run:
//   bun run test/headless-net.ts
//
// Output is interleaved per-tab. Lines tagged `[tab-js]` / `[tab-rust]`
// are from each browser; `[runner]` is from this script.
//
// The relay + caddy + browser get cleaned up on exit including
// abnormal exit via SIGINT/SIGTERM handlers.

import { chromium, type Browser, type Page } from 'playwright';

const RELAY_MULTIADDR_FILE = './dist/relay-multiaddr.txt';
const SERVE_PORT = 8084;             // sidesteps `make wasm-serve` (8083)
const SOAK_MS = 150_000;             // total observation window (worker wasm init can take 40s)
const POLL_MS = 2_000;               // how often we tail #log to stdout

function tag(prefix: string, line: string): void {
  for (const l of line.split('\n')) {
    if (l.length > 0) console.log(`${prefix} ${l}`);
  }
}

async function pollUntil<T>(
  fn: () => Promise<T | undefined>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const v = await fn();
    if (v !== undefined && v !== null) return v as T;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timeout after ${timeoutMs}ms waiting for: ${label}`);
}

let relayProc: ReturnType<typeof Bun.spawn> | undefined;
let caddyProc: ReturnType<typeof Bun.spawn> | undefined;
let browser: Browser | undefined;

async function cleanup(): Promise<void> {
  tag('[runner]', 'cleanup: closing browser + killing relay/caddy');
  try { await browser?.close(); } catch (e) { tag('[runner]', `browser close err: ${(e as Error).message}`); }
  try { caddyProc?.kill(); } catch {}
  try { relayProc?.kill(); } catch {}
}

process.on('SIGINT', async () => { await cleanup(); process.exit(130); });
process.on('SIGTERM', async () => { await cleanup(); process.exit(143); });

async function pipeProcess(proc: ReturnType<typeof Bun.spawn>, prefix: string): Promise<void> {
  // Bun gives us ReadableStream<Uint8Array> for stdout/stderr.
  for (const [stream, label] of [
    [proc.stdout, `${prefix} out`],
    [proc.stderr, `${prefix} err`],
  ] as const) {
    if (!stream) continue;
    (async () => {
      const reader = stream.getReader();
      const decoder = new TextDecoder();
      let buf = '';
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        let nl;
        while ((nl = buf.indexOf('\n')) >= 0) {
          tag(`[${label}]`, buf.slice(0, nl));
          buf = buf.slice(nl + 1);
        }
      }
      if (buf.length > 0) tag(`[${label}]`, buf);
    })();
  }
}

async function readLog(page: Page): Promise<string> {
  return page.$eval('#log', (el) => (el as HTMLElement).innerText || '').catch(() => '');
}

async function readStatus(page: Page): Promise<string> {
  return page.$eval('#status', (el) => (el as HTMLElement).innerText || '').catch(() => '');
}

async function main(): Promise<void> {
  // Spawn relay. Don't touch `dist/relay-multiaddr.txt` — the bridge
  // now reads `RELAY_MULTIADDR` from source, not from that file, so
  // writing to it would just pollute the developer's dev environment
  // without affecting probe behavior. The probe currently dials the
  // hardcoded production relay; an `?relay=` URL override would let
  // probes point at a local relay again (not yet implemented).
  tag('[runner]', 'spawning relay (bun run relay/relay.ts)');
  relayProc = Bun.spawn(['bun', 'run', 'relay/relay.ts'], {
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      ...process.env,
      ROAM_RELAY_PUBLISH_METRICS: '0',
      ROAM_RELAY_WRITE_DIST: '0',                           // don't pollute dist/
      ROAM_RELAY_LISTEN_PORT: '9002',                       // sidestep the user's :9001
      // Bypass `/dns4/localhost/...` so the browser doesn't go through
      // a DNS lookup that, in headless Chromium, blocks ~10s on IPv6
      // (AAAA timeout, then IPv4 fallback) — long enough that rust-
      // libp2p's transport-upgrade timer expires before the WebSocket
      // open completes. /ip4/127.0.0.1/... is resolved synchronously.
      // Discriminates DNS-slowness from libp2p-protocol-compat as the
      // root cause; matches `LISTEN_HOST` so the relay binds and
      // announces the same address.
      ROAM_RELAY_ANNOUNCE: '/ip4/127.0.0.1/tcp/9002/ws',
    },
  });
  pipeProcess(relayProc, 'relay');

  // Wait for the relay to log its peerId line (signal that it's up
  // and accepting connections). Reads relayProc.stdout — pipeProcess
  // above already attached its own reader, so we tap the same byte
  // stream via a small "log line saw" pattern below.
  // For now: 3-second blunt-instrument sleep. The probe doesn't
  // currently use the local relay's address (bridge dials the
  // hardcoded production relay), so this is just letting the
  // process spin up before caddy boots.
  await new Promise((r) => setTimeout(r, 3_000));

  // 4. Spawn caddy on SERVE_PORT (separate from the dev port).
  tag('[runner]', `spawning caddy on :${SERVE_PORT}`);
  caddyProc = Bun.spawn(['caddy', 'run', '--config', 'Caddyfile', '--adapter', 'caddyfile'], {
    stdout: 'pipe',
    stderr: 'pipe',
    env: { ...process.env, ROAM_SERVE_PORT: String(SERVE_PORT) },
  });
  pipeProcess(caddyProc, 'caddy');

  // 5. Wait for caddy to answer.
  await pollUntil(async () => {
    try {
      const r = await fetch(`http://localhost:${SERVE_PORT}/`);
      return r.ok ? true : undefined;
    } catch { return undefined; }
  }, 10_000, 'caddy ready');
  tag('[runner]', 'caddy ready');

  // 6. Launch headless Chromium.
  tag('[runner]', 'launching chromium (headless)');
  // Headless Chromium aggressively throttles timers + async work in
  // backgrounded tabs (a tab without focus is "background"). All our
  // tabs run unfocused — every timer + WebSocket-event delivery would
  // be delayed past the rust-libp2p upgrade timeout. These flags
  // disable that throttling so the test sees realistic timing.
  browser = await chromium.launch({
    headless: true,
    args: [
      '--disable-background-timer-throttling',
      '--disable-renderer-backgrounding',
      '--disable-backgrounding-occluded-windows',
    ],
  });

  // 7. Two contexts so they don't share cookies / localStorage (which
  //    would let one tab restore the other's session and confuse us).
  const ctxJs = await browser.newContext();
  const ctxRust = await browser.newContext();
  const pageJs = await ctxJs.newPage();
  const pageRust = await ctxRust.newPage();

  // 8. Pipe every console + pageerror to stdout, tagged.
  pageJs.on('console', (m) => tag('[tab-js console]', `${m.type()}: ${m.text()}`));
  pageRust.on('console', (m) => tag('[tab-rust console]', `${m.type()}: ${m.text()}`));
  pageJs.on('pageerror', (e) => tag('[tab-js pageerror]', e.message));
  pageRust.on('pageerror', (e) => tag('[tab-rust pageerror]', e.message));

  // 9. Navigate. Default URL = js-libp2p path; ?provider=rust = rust-libp2p.
  tag('[runner]', `navigating tab-js  → http://localhost:${SERVE_PORT}/`);
  tag('[runner]', `navigating tab-rust → http://localhost:${SERVE_PORT}/?provider=rust`);
  await Promise.all([
    pageJs.goto(`http://localhost:${SERVE_PORT}/`),
    pageRust.goto(`http://localhost:${SERVE_PORT}/?provider=rust`),
  ]);

  // 10a. Drive both tabs with periodic movement so the position
  //      payload actually changes — otherwise gossipsub's local message
  //      cache rejects identical bytes as duplicates and we can't tell
  //      "publish never went out" from "publish was a real duplicate".
  //      Arrow keys correspond to the bridge's WASD/arrow input bits.
  const MOVE_KEYS = ['ArrowRight', 'ArrowDown', 'ArrowLeft', 'ArrowUp'];
  let moveTick = 0;
  const moveInterval = setInterval(async () => {
    const key = MOVE_KEYS[moveTick % MOVE_KEYS.length];
    moveTick += 1;
    try {
      await Promise.all([
        pageJs.keyboard.press(key, { delay: 50 }),
        pageRust.keyboard.press(key, { delay: 50 }),
      ]);
    } catch {}
  }, 500);

  // 10b. Tail #log on each tab every POLL_MS. Diff against previous so
  //     each new line is printed once, not on every poll.
  let lastJs = '';
  let lastRust = '';
  const start = Date.now();
  while (Date.now() - start < SOAK_MS) {
    const [logJs, logRust] = await Promise.all([readLog(pageJs), readLog(pageRust)]);
    if (logJs.length > lastJs.length) {
      tag('[tab-js #log]', logJs.slice(lastJs.length));
      lastJs = logJs;
    }
    if (logRust.length > lastRust.length) {
      tag('[tab-rust #log]', logRust.slice(lastRust.length));
      lastRust = logRust;
    }
    await new Promise((r) => setTimeout(r, POLL_MS));
  }
  clearInterval(moveInterval);

  // 11. Final state snapshot.
  const [statusJs, statusRust] = await Promise.all([readStatus(pageJs), readStatus(pageRust)]);
  tag('[runner]', '=== FINAL ===');
  tag('[runner]', `tab-js  status: ${statusJs.trim()}`);
  tag('[runner]', `tab-rust status: ${statusRust.trim()}`);

  await cleanup();
  process.exit(0);
}

main().catch(async (e) => {
  tag('[runner]', `FATAL: ${(e as Error).message}`);
  await cleanup();
  process.exit(1);
});
