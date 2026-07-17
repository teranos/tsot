# Handover — `claude/cardless-sleeve-commit-631wle`

Context lives in `CARDLESS_SLEEVE.md` (roadmap, slices 12.x), the commit
log (`927a4a8`..`f185da8`), and `game/death_replacement_tests.rs`. This file
is only the remaining actions — each with a success condition. Don't
re-read the mechanics here; follow the pointers.

1. **`symbol` on `cards/white-elephant.lua`** — set to `⋈` on a guess; the
   convention for when a non-Symbol card declares a symbol is undocumented
   and the corpus is split (`grep symbol cards/*.lua`). Decide it.
   *Done when:* the field matches the intended rule and the rule is stated
   in `LUA.md`/`CARD.md`.

2. **`cleanup_b8_damage_deaths`** — production-dead since 12.3b; only
   `play_tests.rs:3198,:3300` call it. Delete (migrate those tests to
   `damage_lethal_creatures()`) or mark `// test-only`.
   *Done when:* no dead production path; `cargo test` green.

3. **Zero-Y chained combat death** — 12.3c settles a chained *burn* in
   combat but the settle scans damage-lethal only, not the C.15 zero-Y
   check. Unknown if a real gap.
   *Done when:* a test (combat death whose `on_die` applies lethal −Y to a
   bystander) passes — as-is (keep as guard) or after extending the settle.

4. **Canonical death-replacement rule in `RULES.md`** — engine has the
   window (`OnWouldDie` / prevent / redirect), rules don't describe it (only
   T.3 was added).
   *Done when:* a rule exists that matches `death_replacement_tests.rs`.

Larger future branches (worn/opaque sleeves, Elm UI): `CARDLESS_SLEEVE.md`
"Deferred".
