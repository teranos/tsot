# Handover — sleeve-conservation branch (`claude/cardless-sleeve-commit-631wle`)

Branch is 6 commits ahead of `master`, full suite green (560 lib passed,
0 failed; all integration binaries green). Roadmap of record for the whole
arc: `CARDLESS_SLEEVE.md` (slices 12.1–12.4, 12.3b, 12.3c). This file lists
only the **remaining** work — each item is scoped and has a success
condition.

---

## 1. Decide the `symbol` field on `cards/white-elephant.lua`

`white-elephant.lua` declares `symbol = "⋈"`, copied from the glassblower
without confirming the convention. The corpus is split: most white
creatures omit `symbol` (binding-knight, companion-hound, glass-mantis,
mortal-bee, temple-rat, trustworthy-lender, vigilant-human), but some
creatures carry one (wayfinder `⋈`, white-monkey `≡`, angry-glassblower
`⋈`). The rule for *when* a non-Symbol card declares a printed symbol is
not written down anywhere and needs the designer's call.

- **Action:** decide the rule; keep, change, or remove the elephant's
  `symbol`.
- **Success condition:** `white-elephant.lua`'s `symbol` matches the
  intended convention, and (if the rule is general) it's stated in a
  sentence in `LUA.md` or `CARD.md` so the next card follows it.

## 2. Remove or mark `cleanup_b8_damage_deaths` as test-only

After 12.3b, `do_damage` no longer calls `cleanup_b8_damage_deaths` — the
B.8 sweep moved to `drain_deferred_events`. The function now has **no
production caller**; only two tests invoke it directly
(`play_tests.rs:3198`, `:3300`).

- **Action:** either delete `cleanup_b8_damage_deaths` and migrate those two
  tests to assert via `damage_lethal_creatures()` (the extracted scan) plus
  the deferred path, **or** keep it with a `// test-only` doc comment
  explaining why.
- **Success condition:** no dead production code path — either the function
  is gone and the suite is green, or its doc comment states it is
  deliberately retained for direct-scan unit tests. `cargo test` green
  either way.

## 3. Zero-Y chained death inside combat (analog to 12.3c)

12.3c made a combat death's `on_die` **burn** (`game.damage`) settle its
victim within the same combat. The combat settle drain only scans
`damage_lethal_creatures()` (damage ≥ Y) — it does **not** run the C.15
zero-Y check. So a combat death whose `on_die` applies a −Y modifier that
drops a bystander to Y ≤ 0 may not die in-combat. Unknown whether this is a
real gap (no card exercises it today).

- **Action:** add a failing-first test — a combat death whose `on_die`
  applies lethal −Y to a bystander — and see whether it dies in-combat and
  appears in `outcome.deaths`.
- **Success condition:** the test passes. If it already passes as written,
  keep it as a regression guard and this item is done; if it fails, extend
  the combat settle (or unify the damage + zero-Y checks behind one
  post-combat settle) until it passes, suite green.

## 4. Canonical rule for the death-replacement window in `RULES.md`

The engine has `OnWouldDie` + `game.prevent_death` / `game.redirect_death`,
but `RULES.md` describes no death-replacement concept (only T.3, the
ownership carve-out, was added). The replacement window is currently
engine-only.

- **Action:** add a canonical rule (B-series or P-series) describing that a
  creature about to leave the BOARD as a death may be subject to a
  card-defined replacement: prevent (survives on board, accumulated damage
  cleared) or redirect (leaves to another zone quietly — no on_die/no
  broadcast/no cascade).
- **Success condition:** the rule exists in `RULES.md` and matches the
  engine's actual behavior (cross-check against
  `game/death_replacement_tests.rs`); no contradiction between the rule text
  and the tests.

## 5. Open the pull request

Branch is ready; no PR has been opened. Suggested title/description are in
the session notes (title: "TSOT: sleeve conservation — mutation shed,
sleeveless cards, death-replacement hook").

- **Action:** open a PR from `claude/cardless-sleeve-commit-631wle` against
  `master`.
- **Success condition:** PR exists, base `master`, containing exactly the 6
  sleeve-conservation commits (`927a4a8`..`f185da8`), body per the drafted
  description.

## 6. (Optional) Put the White Elephant in a playable pool

`cards/white-elephant.lua` is tested but in no starter deck and not in the
EA draftable pool, so it never plays in a real sim game.

- **Action:** add `white-elephant` to a starter deck (e.g. the white
  starter) or the EA draftable id list.
- **Success condition:** a Heuristic-vs-Heuristic `sim/run` game that
  includes the elephant runs to a winner with rollback + determinism
  holding (mirror an existing `sim/run.rs` end-to-end test).

---

## Not small items — future branches

These are multi-slice and specced (not single completable tasks); see
`CARDLESS_SLEEVE.md` "Deferred":

- **Worn / fillable sleeves** — putting cards into empty sleeves, re-sleeving
  (which re-arms a sleeveless card's "shed to survive" ward — noted as a
  watch-out in `CARDLESS_SLEEVE.md`). Enforce the fourth-quadrant invariant
  (`set_sleeveless` guard, done) at the new re-sleeve verbs too.
- **Opaque / colored sleeves** — a sleeve carrying its own color identity,
  able to pay color costs; touches Z.8e visibility.
- **Elm UI** for the new states (shed / sleeveless / fused) under the
  `CARD.md` one-node-per-iid contract; TODO in `ELM_PLAN.md`.
