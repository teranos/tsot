TSOT — The Symbols of Teranos. A 1v1 collectible card game.

Symbols: ax = ⋈, ix = ⨳, am = ≡, pulse = ꩜, sem = ⊨, delta = δ

Target: WASM (browser). Native CLI for sim + EA work.

---

Cards live in `cards/*.lua`. Card schema and Lua execution model in
@LUA.md; rules in @RULES.md; sim AI + game-runner internals in
@src/sim/README.md.

You can compare alternative versions of a card (cost, stats, effect
magnitude) — `make probe` recognises `variants = { [key] = { overrides } }`
blocks on cards.

Don't push probe results unless asked. Don't ask them whether to
probe. Don't probe a single new card — there's nothing to compare.

---

Balance, power level, "premium cost", card-economy ratios,
win-conditions, archetype viability — not chat answers. You have no
playtest data and no game intuition; don't editorialize. Just write
the card or the mechanic. If the user asks for balance input, point
them at sim output or write the analysis.

Same applies to strategy talk: lines of play, optimal sequencing,
"tempo", matchup analysis, deckbuilding heuristics, archetype labels,
combo identification. You don't play the game and your strategic
intuitions are confabulated from CCG pattern-matching. Don't generate
it. Cards, mechanics, rules, engine — yes. Strategy — no.
