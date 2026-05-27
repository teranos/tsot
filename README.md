# tsot — The Symbols of Teranos

A browser-based collection garden that teaches the canonical symbols of QNTX. No opponent, no losing. Each symbol-card is one of QNTX's SEG operators (`sym/symbols.go`), manifested through `@qntx/glyphs`'s morphing continuum: collapsed dot → proximity-expanded → full window.

## v1 scope

- Cursor is `⍟` (Self) — fixed, follows the mouse. System cursor hidden.
- 3 tray slots in palette order: `≡ am`, `⨳ ix`, `⋈ ax`.
- First-ever load manifests `≡ am` — opening the game IS the act of being.
- Opening a card's window for the first time unlocks the next slot.
- State persists to IndexedDB. Subsequent loads restore.

Deferred: the remaining 18 canonical symbols (building blocks, derived types, system symbols); meld/composition mechanics; QNTX plugin integration (gRPC, attestation writes).

## Stack

- **Bun** runtime + bundler.
- **TypeScript**, no framework.
- **`@qntx/glyphs`** — glyph runtime: tray (`glyphRun`), cursor manifestation, proximity engine, morph transactions, window manifestation.

## Running

```sh
bun install
bun run dev    # http://localhost:5180
bun run build  # one-shot bundle to dist/
```

The `@qntx/glyphs` dependency is pinned to the local checkout at `../QNTX/packages/glyphs`. Bun resolves it via `file:` link.

## Layout

```
tsot/
├── package.json
├── tsconfig.json
├── dev-server.ts        Bun.serve dev with SSE live reload
├── build.ts             Bun.build to dist/
├── index.html
└── src/
    ├── main.ts          entry: configureGlyphs, cursor, tray, opening ritual
    ├── symbols.ts       4 SEG defs mirrored from QNTX/sym/symbols.go
    ├── persist.ts       IndexedDB-backed GlyphPersistence + game state
    ├── glyphs.ts        createSegGlyph factory (window manifestation)
    └── styles.css
```

## The 4 symbols (v1)

| Sym | Cmd | Role | First appearance |
|---|---|---|---|
| `⍟` | `i`  | Cursor — Self, your vantage point | On every load. Never in tray. |
| `≡` | `am` | Configuration — the act of being | First-ever load. |
| `⨳` | `ix` | Ingest — import external data | Unlocks on first open of `am`. |
| `⋈` | `ax` | Expand — query and surface context | Unlocks on first open of `ix`. |

Each window manifestation surfaces the SEG/SYM/GLYPH attestable-grammar triplet from `docs/vision/glyphs.md:200-209`.
