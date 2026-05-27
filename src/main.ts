import {
  configureGlyphs,
  createCursorElement,
  attachCursorToMouse,
  glyphRun,
} from '@qntx/glyphs';
import { I, AM, IX, AX, PALETTE_ORDER } from './symbols';
import {
  loadState,
  persistence,
  markManifested,
  isManifested,
  markOpenedFirstTime,
} from './persist';
import { createSegGlyph } from './glyphs';
import type { SegDef } from './symbols';

const TRAY_ORDER: ReadonlyArray<SegDef> = [AM, IX, AX];

const { isFirstEver } = await loadState();

configureGlyphs({
  logSegment: 'TSOT',
  persistence,
  logger: {
    debug: (seg, msg) => console.debug(`[${seg}] ${msg}`),
    info: (seg, msg) => console.info(`[${seg}] ${msg}`),
    warn: (seg, msg) => console.warn(`[${seg}] ${msg}`),
    error: (seg, msg) => console.error(`[${seg}] ${msg}`),
  },
});

const cursor = createCursorElement(I.symbol, I.command);
document.body.appendChild(cursor);
attachCursorToMouse(cursor);

glyphRun.init();

if (isFirstEver) {
  markManifested(AM.command);
}

function nextLockedIn(): SegDef | null {
  for (const def of TRAY_ORDER) {
    if (!isManifested(def.command)) return def;
  }
  return null;
}

function addToTray(def: SegDef): void {
  const glyph = createSegGlyph(def);
  glyphRun.add(glyph);
  const dot = document.querySelector(`[data-glyph-id="${glyph.id}"]`);
  if (!dot) return;
  const onFirstOpen = () => {
    if (!markOpenedFirstTime(def.command)) return;
    const next = nextLockedIn();
    if (!next) return;
    markManifested(next.command);
    addToTray(next);
  };
  dot.addEventListener('click', onFirstOpen, { once: true });
}

for (const def of TRAY_ORDER) {
  if (isManifested(def.command) && !glyphRun.has(`tsot:${def.command}`)) {
    addToTray(def);
  }
}

console.info(
  `[TSOT] ready — palette: ${PALETTE_ORDER.map((d) => d.symbol).join(' ')}`,
);
