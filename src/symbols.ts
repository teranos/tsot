// Mirrors QNTX/sym/symbols.go — kept in sync manually.
// Source of truth: github.com/teranos/QNTX/sym/symbols.go

export interface SegDef {
  command: string;
  symbol: string;
  title: string;
  meaning: string;
}

export const I: SegDef = {
  command: 'i',
  symbol: '⍟',
  title: 'Self',
  meaning: 'Your vantage point into QNTX.',
};

export const AM: SegDef = {
  command: 'am',
  symbol: '≡',
  title: 'Structure',
  meaning: 'Configuration — system settings and state.',
};

export const IX: SegDef = {
  command: 'ix',
  symbol: '⨳',
  title: 'Ingest',
  meaning: 'Import external data.',
};

export const AX: SegDef = {
  command: 'ax',
  symbol: '⋈',
  title: 'Expand',
  meaning: 'Query and surface related context.',
};

export const PALETTE_ORDER: ReadonlyArray<SegDef> = [I, AM, IX, AX];
