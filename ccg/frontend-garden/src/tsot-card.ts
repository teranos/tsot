// tsot card — glyph manifestation of a tsot CCG card.
//
// A card has two faces (back: 15-slot symbol grid per SLOTS.md;
// front: color identity + name + cost + abilities, design TBD). The
// `createCardGlyph` factory wraps both faces in a `Glyph` so the
// garden's runtime morphs it (dot → window) like any other glyph.
//
// The renderer is data-driven: it consumes a `CardView` (the engine's
// serialized card shape) and produces DOM. No parallel schema.

import type { Glyph } from '@qntx/glyphs';
import type { CardView, CardFace } from './tsot-card-types';
import { wrapRenderContent } from './debug';

/** Visual row-major slot order (per SLOTS.md, 5 rows × 3 cols). */
const SLOT_GRID = [
  'TL', 'T', 'TR',
  'UL', 'U', 'UR',
  'L',  'C', 'R',
  'DL', 'D', 'DR',
  'BL', 'B', 'BR',
] as const;

/** Spiral default fill order for array-form symbols (SLOTS.md). */
const SLOT_SPIRAL = [
  'C', 'U', 'UR', 'R', 'DR', 'D', 'DL', 'L', 'UL',
  'TL', 'T', 'TR', 'BR', 'B', 'BL',
] as const;

type Slot = (typeof SLOT_GRID)[number];

/** Spiral-fill the flat `card.symbols` array to slot positions. */
function symbolsBySlot(card: CardView): Partial<Record<Slot, string>> {
  const out: Partial<Record<Slot, string>> = {};
  for (let i = 0; i < card.symbols.length && i < SLOT_SPIRAL.length; i++) {
    out[SLOT_SPIRAL[i] as Slot] = card.symbols[i];
  }
  return out;
}

/** Slots that are transparent (holes) on this card. */
function transparentSlots(card: CardView): Set<Slot> {
  // Until SLOTS.md ships in the engine, `frame = "transparent"` means
  // every slot is a hole — the whole-card limit case.
  if (card.frame === 'transparent') return new Set(SLOT_GRID);
  return new Set();
}

/** Render the back face: 3×5 grid with symbols and transparent slots. */
export function renderCardBack(card: CardView): HTMLElement {
  const back = document.createElement('div');
  back.className = 'tsot-card-face tsot-card-back';
  const bySlot = symbolsBySlot(card);
  const holes = transparentSlots(card);
  for (const slot of SLOT_GRID) {
    const cell = document.createElement('div');
    cell.className = 'tsot-card-cell';
    if (holes.has(slot)) cell.classList.add('is-transparent');
    cell.dataset.slot = slot;
    const glyph = bySlot[slot];
    if (glyph) cell.textContent = glyph;
    back.appendChild(cell);
  }
  return back;
}

/** Placeholder front face: color identity + name. Refine when designed. */
const COLOR_BG: Record<string, string> = {
  red:    '#9c3a3a',
  blue:   '#3a5e9c',
  green:  '#3a8c4f',
  white:  '#d4cdb4',
  black:  '#2a2228',
  purple: '#6b3a8c',
  brown:  '#6b5239',
  orange: '#c46a2e',
  yellow: '#c0a839',
  pink:   '#b85b8e',
  azure:  '#3a8ca0',
};

export function renderCardFront(card: CardView): HTMLElement {
  const front = document.createElement('div');
  front.className = 'tsot-card-face tsot-card-front';
  const c = card.colors[0]?.toLowerCase();
  if (c && COLOR_BG[c]) front.style.background = COLOR_BG[c];
  const name = document.createElement('div');
  name.className = 'tsot-card-front-name';
  name.textContent = card.name;
  front.appendChild(name);
  return front;
}

/**
 * Render the flippable card primitive — both faces inside a 3D
 * flipper. `face` controls which side is showing; mutate the
 * `data-face` attribute on the returned element to flip (CSS
 * transition runs the animation).
 */
export function renderCard(card: CardView, face: CardFace = 'back'): HTMLElement {
  const wrap = document.createElement('div');
  wrap.className = 'tsot-card';
  wrap.dataset.face = face;
  wrap.dataset.iid = card.iid;
  const flipper = document.createElement('div');
  flipper.className = 'tsot-card-flipper';
  flipper.appendChild(renderCardBack(card));
  flipper.appendChild(renderCardFront(card));
  wrap.appendChild(flipper);
  return wrap;
}

/** Toggle the visible face on a rendered card element. */
export function setCardFace(el: HTMLElement, face: CardFace): void {
  el.dataset.face = face;
}

/**
 * Build a `Glyph` that manifests as a window showing one card. Dot
 * shows the card's first symbol (the C-slot glyph after spiral fill);
 * window opens to the card's back face by default, with a click to
 * flip to the front.
 */
export function createCardGlyph(card: CardView): Glyph {
  const id = `tsot:card:${card.iid}`;
  return {
    id,
    title: card.name || 'card',
    symbol: card.symbols[0] ?? '·',
    manifestationType: 'window',
    initialWidth: '280px',
    initialHeight: '500px',
    // Wrapped so the debug counter sees each invocation. Per the
    // stash pattern this must fire ONCE per glyph lifetime — if the
    // counter for this id exceeds 1 the stash was bypassed.
    renderContent: wrapRenderContent(id, () => {
      const root = document.createElement('div');
      root.className = 'tsot-card-window';
      const cardEl = renderCard(card, 'back');
      cardEl.addEventListener('click', () => {
        const cur = (cardEl.dataset.face as CardFace) ?? 'back';
        setCardFace(cardEl, cur === 'back' ? 'front' : 'back');
      });
      root.appendChild(cardEl);
      return root;
    }),
  };
}
