import type { Glyph } from '@qntx/glyphs';
import type { SegDef } from './symbols';

function el(tag: string, className?: string, text?: string): HTMLElement {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = text;
  return node;
}

function triplet(def: SegDef): HTMLElement {
  const root = el('div', 'triplet');
  const lines: Array<[string, string]> = [
    ['SEG', `"${def.command}" is segment of grammar`],
    ['SYM', `"${def.symbol}" is symbol of SEG "${def.command}"`],
    ['GLYPH', `"${def.command}-glyph" is manifestation of SEG "${def.command}"`],
  ];
  for (const [label, body] of lines) {
    const row = el('div', 'triplet-row');
    row.appendChild(el('span', 'triplet-label', label));
    row.appendChild(el('span', 'triplet-body', body));
    root.appendChild(row);
  }
  return root;
}

function renderContent(def: SegDef): HTMLElement {
  const root = el('div', 'seg-window');

  const head = el('div', 'seg-head');
  head.appendChild(el('span', 'seg-symbol', def.symbol));
  const ident = el('div', 'seg-ident');
  ident.appendChild(el('div', 'seg-command', def.command));
  ident.appendChild(el('div', 'seg-title', def.title));
  head.appendChild(ident);
  root.appendChild(head);

  root.appendChild(el('p', 'seg-meaning', def.meaning));
  root.appendChild(triplet(def));

  return root;
}

export function createSegGlyph(def: SegDef): Glyph {
  return {
    id: `tsot:${def.command}`,
    title: def.title,
    symbol: def.symbol,
    manifestationType: 'window',
    initialWidth: '560px',
    initialHeight: '420px',
    renderContent: () => renderContent(def),
  };
}
