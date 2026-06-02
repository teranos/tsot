# Slots — symbol and hole geometry

Canonical reference for where symbols and transparent holes live on a card. Replaces the binary "transparent color/frame" model: transparent is no longer a property of the whole card, it's a positioned hole at a specific slot.

This document is the source of truth for slot mechanics. RULES.md will reference it once the engine catches up.

## The 15-slot grid

Each card has 15 named slot positions, arranged 5 rows × 3 columns:

```
TL  T  TR
UL  U  UR
L   C   R
DL  D  DR
BL  B  BR
```

The center slot `C` is the default — every card has it, every other slot is opt-in. The inner 3×3 ring around `C` (`UL U UR / L C R / DL D DR`) is what most cards will use; the outer top and bottom rows (`TL T TR`, `BL B BR`) extend vertically only — the grid stays 3 columns wide, no outer-left or outer-right.

Naming: top / upper / center / lower / bottom for rows; left / (none) / right for columns. Center row uses bare `L C R`; other rows compose the row letter with the column letter (`UL UR DL DR TL TR BL BR`) plus the column-less ones (`T U D B`).

## Symbols

A card's symbols live at specific slots. The data shape: `symbols = { [slot] = "glyph" }`, e.g. `symbols = { C = "꩜" }` for a standard one-symbol card; `symbols = { C = "꩜", T = "≡", B = "⨳" }` for a three-symbol card.

**Default**: any card without an explicit per-slot `symbols` block fills slots by spiraling out from `C` in this canonical order:

```
C, U, UR, R, DR, D, DL, L, UL, TL, T, TR, BR, B, BL
```

Clockwise from center through the inner 8, then clockwise through the outer 6. So:
- `symbols = {"X"}` → `X` at `C`
- `symbols = {"X", "Y"}` → `X` at `C`, `Y` at `U`
- `symbols = {"X", "Y", "Z"}` → `X` at `C`, `Y` at `U`, `Z` at `UR`
- etc.

Cards that want specific placement use the long form `symbols = { C = "X", T = "Y" }`.

**Rule (replaces C.1)**: a card's symbol set is the union of glyphs across every occupied slot.

## Holes

Transparent cards have holes at specific slots. The data shape: `holes = { slot, slot, ... }`, e.g. `holes = { C }` for a single center hole, `holes = { C, T, B }` for three holes.

**Default**: cards without a `holes` block have no holes (fully opaque).

**Rule (replaces C.13)**: a slot occupied by a hole cannot also carry a symbol on the same card — you can't print a glyph on a hole.

## See-through (the slot-alignment minigame)

When card A sits above card B (the natural cases: A is on top of B in the DECK; A is attached above B; A is the topmost card in a face-down stack), and A has a hole at slot S:

- If B has a symbol at slot S, that symbol is revealed (visible from above through the hole).
- If B has no symbol at slot S, the hole reveals nothing.
- If B itself has a hole at slot S, the rule applies recursively to whatever's under B.

The match is **per-slot exact**: a hole at `C` reveals only symbols at `C` on the card below. A hole at `T` doesn't reveal a symbol at `C`. This is what makes hole placement design-meaningful — a card with a hole at the center can be defeated by tucking it over a card whose symbols are all in the outer slots, even if both cards are otherwise compatible.

**Rule (replaces V.8)**: a card with holes on top of the DECK reveals the symbols at matching slots from the next-down card. Multiple holes reveal multiple symbols. The reveal walks through the deck per-slot until each hole-slot finds either a symbol or an opaque (hole-less) card that blocks it.

## Interactions with existing rules

- **C.1** (symbols are on the back of the card): unchanged; symbol-slot data describes WHERE on the back they sit.
- **C.13** (transparent cards have no symbols): subsumed by the per-slot rule (a hole slot can't carry a symbol).
- **C.14** (transparent ↔ transparent attachment): re-evaluate when implementation lands. A card with holes might still attach to any host; the symbol-reveal mechanic is independent of attachment legality.
- **V.8** (deck-top transparent reveals next): generalized to per-slot reveals through any hole-bearing card.
- **V.9** (glow visibility through non-transparent stack): glow is a separate surface treatment, no slot involvement; its visibility rule stands as written when we re-frame "non-transparent" → "non-hole-bearing at the relevant slots".

## Status

Design only. No engine, schema, or loader support yet. Existing transparent cards stay on `frame = "transparent"` (whole-card hole) until the per-slot system lands. Migration plan deferred until the slot data shape ships in `Card`.

## Open questions

- **Multi-hole cards**: same-slot-on-different-cards collisions when stacking — straightforward, just nesting. Multiple holes on the same card revealing different symbols on different cards in the stack — also straightforward, each slot evaluates independently.
- **Hole at a slot above a glyph at the same slot on a stacked hole-card**: the hole "passes through" both holes until an opaque slot is hit. Consistent recursion.
- **Symbol-grant statics** (gfp / mcherry / sparkle): if a static grants a symbol to a host, which slot does it land at? Probably `C` unless the static names a slot. Pin once the static system needs to know.
- **Color identity**: still a separate axis (printed colors, P.12a-anchorable). Slots don't touch color.
- **Glow / surface treatments**: also a separate axis, no slot semantics. Whether glow stays in `colors` or moves to its own attribute is a parallel decision.
