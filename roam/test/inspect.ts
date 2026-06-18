// Attach to a running Chrome via the Chrome DevTools Protocol.
//
// You launch Chrome once with a remote-debugging port:
//
//   /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
//     --remote-debugging-port=9222 \
//     --user-data-dir=/tmp/roam-chrome-profile \
//     http://localhost:8083/?provider=rust
//
// Then `bun run test/inspect.ts` attaches and dumps:
//   - every console message (with stack traces for errors)
//   - every page error
//   - every uncaught promise rejection
//   - the current text of the in-page event log (`#log`) periodically
//
// Use this instead of "send me a screenshot" while we're chasing
// the worker / provider state.

import { chromium } from 'playwright';

const POLL_LOG_MS = 1500;
const ROAM_URL = process.env.ROAM_URL || 'http://localhost:8083/?provider=rust';

// Launch Playwright's bundled Chromium directly — CDP is built-in,
// no `--remote-debugging-port` dance, no macOS port-binding quirks,
// no fighting the user's primary Chrome process. The user's session
// stays on Firefox; this Chromium is the dev observation surface.
const browser = await chromium.launch({
  headless: false,
  args: [
    '--disable-background-timer-throttling',
    '--disable-renderer-backgrounding',
    '--disable-backgrounding-occluded-windows',
  ],
});
const context = await browser.newContext();
const roamPage = await context.newPage();
console.log(`[inspect] navigating to ${ROAM_URL}`);
await roamPage.goto(ROAM_URL);
console.log(`[inspect] watching page: ${roamPage.url()}`);

// Wire console + pageerror to stdout. Console messages from workers
// also surface here under Playwright's exposed events.
roamPage.on('console', (msg) => {
  console.log(`[console.${msg.type()}] ${msg.text()}`);
});
roamPage.on('pageerror', (err) => {
  console.log(`[pageerror] ${err.message}`);
  if (err.stack) console.log(err.stack);
});

// Poll the event-log panel so the in-page sacred-error stream
// also lands here, even if it bypasses console.
let lastLogLen = 0;
setInterval(async () => {
  try {
    const txt = await roamPage!.$eval('#log', (el) => (el as HTMLElement).innerText || '');
    if (txt.length > lastLogLen) {
      const newPart = txt.slice(lastLogLen);
      lastLogLen = txt.length;
      for (const line of newPart.split('\n')) {
        if (line) console.log(`[#log] ${line}`);
      }
    }
  } catch {
    /* page navigated away or not ready */
  }
}, POLL_LOG_MS);

// Hold open. Ctrl-C to exit.
console.log('[inspect] streaming — Ctrl-C to stop');
await new Promise(() => {});
