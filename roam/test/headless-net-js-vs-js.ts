// JS-vs-JS baseline.
//
// Same as `headless-net.ts` but both tabs use the default URL (no
// `?provider`), so both are JS-libp2p. Answers the question: does
// even the JS-to-JS path deliver messages browser-to-browser through
// the relay? If yes, the rust path's "0 messages" is rust-specific.
// If no, the bug is in the relay / gossipsub layer (or browser
// gossipsub config) regardless of substrate.

import { chromium, type Browser, type Page } from 'playwright';
import { unlinkSync } from 'node:fs';

const RELAY_MULTIADDR_FILE = './dist/relay-multiaddr.txt';
const SERVE_PORT = 8084;
const SOAK_MS = 60_000;
const POLL_MS = 2_000;

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
  tag('[runner]', 'cleanup');
  try { await browser?.close(); } catch {}
  try { caddyProc?.kill(); } catch {}
  try { relayProc?.kill(); } catch {}
}

process.on('SIGINT', async () => { await cleanup(); process.exit(130); });
process.on('SIGTERM', async () => { await cleanup(); process.exit(143); });

async function pipeProcess(proc: ReturnType<typeof Bun.spawn>, prefix: string): Promise<void> {
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
  try { unlinkSync(RELAY_MULTIADDR_FILE); } catch {}

  relayProc = Bun.spawn(['bun', 'run', 'relay/relay.ts'], {
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      ...process.env,
      ROAM_RELAY_PUBLISH_METRICS: '0',
      ROAM_RELAY_LISTEN_PORT: '9002',
      ROAM_RELAY_ANNOUNCE: '/ip4/127.0.0.1/tcp/9002/ws',
    },
  });
  pipeProcess(relayProc, 'relay');

  const multiaddr = await pollUntil(async () => {
    try {
      const t = await Bun.file(RELAY_MULTIADDR_FILE).text();
      const first = t.trim().split('\n')[0];
      return first && first.length > 0 ? first : undefined;
    } catch { return undefined; }
  }, 10_000, 'relay multiaddr file');
  tag('[runner]', `relay listening: ${multiaddr}`);

  caddyProc = Bun.spawn(['caddy', 'run', '--config', 'Caddyfile', '--adapter', 'caddyfile'], {
    stdout: 'pipe',
    stderr: 'pipe',
    env: { ...process.env, ROAM_SERVE_PORT: String(SERVE_PORT) },
  });
  pipeProcess(caddyProc, 'caddy');
  await pollUntil(async () => {
    try { const r = await fetch(`http://localhost:${SERVE_PORT}/`); return r.ok ? true : undefined; } catch { return undefined; }
  }, 10_000, 'caddy ready');
  tag('[runner]', 'caddy ready');

  browser = await chromium.launch({
    headless: true,
    args: [
      '--disable-background-timer-throttling',
      '--disable-renderer-backgrounding',
      '--disable-backgrounding-occluded-windows',
    ],
  });

  // Two JS-libp2p tabs — both at the default URL.
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const pageA = await ctxA.newPage();
  const pageB = await ctxB.newPage();

  pageA.on('pageerror', (e) => tag('[tab-A pageerror]', e.message));
  pageB.on('pageerror', (e) => tag('[tab-B pageerror]', e.message));

  tag('[runner]', `tab-A → http://localhost:${SERVE_PORT}/   (js-libp2p)`);
  tag('[runner]', `tab-B → http://localhost:${SERVE_PORT}/   (js-libp2p)`);
  await Promise.all([
    pageA.goto(`http://localhost:${SERVE_PORT}/`),
    pageB.goto(`http://localhost:${SERVE_PORT}/`),
  ]);

  let lastA = '';
  let lastB = '';
  const start = Date.now();
  while (Date.now() - start < SOAK_MS) {
    const [logA, logB] = await Promise.all([readLog(pageA), readLog(pageB)]);
    if (logA.length > lastA.length) {
      tag('[tab-A #log]', logA.slice(lastA.length));
      lastA = logA;
    }
    if (logB.length > lastB.length) {
      tag('[tab-B #log]', logB.slice(lastB.length));
      lastB = logB;
    }
    await new Promise((r) => setTimeout(r, POLL_MS));
  }

  const [statusA, statusB] = await Promise.all([readStatus(pageA), readStatus(pageB)]);
  tag('[runner]', '=== FINAL ===');
  tag('[runner]', `tab-A status: ${statusA.trim()}`);
  tag('[runner]', `tab-B status: ${statusB.trim()}`);

  await cleanup();
  process.exit(0);
}

main().catch(async (e) => {
  tag('[runner]', `FATAL: ${(e as Error).message}`);
  await cleanup();
  process.exit(1);
});
