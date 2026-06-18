// Dev-tap log forwarder.
//
// Bun server that listens on :9100 for `POST /log` from the bridge.
// Every `logEvent` line in the page is POSTed here and appended to
// `/tmp/roam-dev.log`. Pair with `tail -f /tmp/roam-dev.log` in
// another shell to watch live what the page sees, without
// screenshots, devtools, or copy-paste cycles.
//
// Run via:
//   bun run test/dev-tap.ts
//
// CORS is wide-open (`*`) because the bridge fetches cross-origin
// (localhost:8083 → localhost:9100). The script is dev-only and
// shouldn't be deployed.

const LOG_PATH = '/tmp/roam-dev.log';
const PORT = 9100;

const file = Bun.file(LOG_PATH);
// Truncate on start so each session is its own log.
await Bun.write(LOG_PATH, '');
console.log(`[dev-tap] truncated ${LOG_PATH}; listening on :${PORT}`);

const server = Bun.serve({
  port: PORT,
  async fetch(req) {
    const url = new URL(req.url);
    if (req.method === 'OPTIONS') {
      return new Response(null, {
        headers: {
          'access-control-allow-origin': '*',
          'access-control-allow-methods': 'POST, OPTIONS',
          'access-control-allow-headers': 'content-type',
        },
      });
    }
    if (req.method === 'POST' && url.pathname === '/log') {
      const body = await req.text();
      const ts = new Date().toISOString();
      const line = `${ts} ${body}\n`;
      // Append synchronously so a `tail -f` reader sees lines as they arrive.
      const f = Bun.file(LOG_PATH);
      const existing = await f.exists() ? await f.text() : '';
      await Bun.write(LOG_PATH, existing + line);
      return new Response('ok', {
        headers: { 'access-control-allow-origin': '*' },
      });
    }
    return new Response('not found', { status: 404 });
  },
});

console.log(`[dev-tap] tail with:  tail -f ${LOG_PATH}`);
