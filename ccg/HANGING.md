# tsot — Hanging Threads

> Single tracker for open requests and unfinished threads. Last refresh:
> 2026-07-18.

**Operating rule (do not violate):** a later request never supersedes an
earlier one. Narrowing the scope of a run, or "ignore that for now," is
*deferral*, not cancellation. Every request stays valid and tracked here
until it is either (a) actually done and verified, or (b) explicitly
dropped by the user. When in doubt, it stays on the list.

---

## Landed this arc (for accounting — not hanging)

So the list below is unambiguously "what's left," here is what shipped:

- `tap` cost source (P.42) — engine + sim-AI affordability + picker/
  resolver agreement (`validate_play`, sweep test). Committed.
- Tap Dance card (P.42, cost + untap/tap effect verified). Committed.
- Ankle Scorcher card + `OnUntapped` engine event (mirror of `OnTapped`).
  Committed. LUA.md event list updated.
- RULES.md ratifications: P.42 (tap), P.41 (cast-from-GY, a–d), A.13
  (tap/untap effect), B.19 (blocking taps) + B.16 (vigilance→blocking).
  **Note: these are the *rules text*. The P.41 and B.19 *engine* are
  still hanging — see below.**
- Probe deck construction constrained to the probed card's color/symbol
  palette (`sim::palette`, probe-only). Committed.
- Probe absent-control cell + Δ-vs-control reporting. Committed.
- CLAUDE.md probe rule fixed ("unless asked").

---

## Hanging

### Rules ratified, engine not built
1. **Cast-from-graveyard (P.41) engine.** RULES.md P.41 a–d is written
   (incl. P.41d: a spell cast from the graveyard is exiled unless it
   would go to the board). No engine support — confirmed: only an
   unrelated `activate_ability`-from-graveyard test exists, no
   cast-a-spell-from-GY cost/play path. **Blocks Spirit Wanderer (#3).**
2. **Blocking taps the blocker (B.19) + vigilance→blocking (B.16).**
   RULES.md written. Not implemented: `declare_blocker` does not tap the
   blocker, and vigilance is not consulted on block. (The existing
   `CombatError::BlockerTapped` is the *separate* "a tapped creature
   can't block" rule, not B.19.)

### Cards requested, not built
3. **Spirit Wanderer** — black/purple creature, subtype spirit-wanderer,
   cost `1 gy + 3 tap`, "may only be cast from the graveyard," haste,
   3/1. Needs the P.41 engine (#1) first. Not started.

### Rules reconciliations flagged, then deferred ("ignore for now")
   These were set aside, NOT cancelled — kept per the operating rule.
4. **P.13 / bitter-dawn reconciliation.** Open.
5. **Whether to add "never tap" to Z.8c's exclusion list.** Open.

### Probe / balance
6. **Power-axis comparison (2 vs 3 power).** The 2×2 grid ("4 variants")
   was requested; a later run was narrowed to "only 1 vs 2 tap cost," and
   the `power2` / `tap2-power2` variants were removed from
   `cards/ankle-scorcher.lua`. Per the operating rule the power-axis
   comparison is still owed — it was narrowed, not dropped. Variants are
   recoverable from git (commits `043f432` / `284ccb6`). Not run.
7. **Resolve the tap 1-vs-2 gap beyond noise.** The daily-fast budget
   (`pop 8 / gens 4 / n 3`, single seed) gave Δbest 0.000 and small
   Δmean — inside the probe's noise band. A heavier run
   (`--pop 24 --gens 12 --n 10`) would settle whether tap 1 vs 2 truly
   differs. Offered, not run.

### Engineering hygiene (identified, not done)
8. **`fitness.rs` failure-detail swallow.** ~`fitness.rs:223/263` drains
   and discards the per-game failure-detail strings — an errors-are-
   sacred violation (a genome can win by exploiting a bug and the score
   won't say). Identified during the flaky-test hunt; not fixed.
9. **Flaky test `diversity_alpha_widens_final_population_diversity`.**
   Root cause still unknown — the crystal-tap picker bug (since fixed)
   was a red herring, present in passing runs too. Blocked partly by #8
   (the real error is swallowed).
10. **DRY: `CostNeeds` 8-site duplication.** Same cost-aggregation shape
    repeated ~8 places; candidate for a shared helper. Not done.
11. **DRY: `substitution_coverage`.** Flagged as a dedup opportunity.
    Not done.
12. **Cyclo / size: split `play_card_inner` (~886 lines).** Now that
    `validate_play` exists as a pure dry-run validator, the validation
    half can be lifted out of the resolver. Not done. (Other large files
    were flagged in the same survey — revisit after this one.)

---

## How to use this

- Adding work? Append to **Hanging** with: what, why, where it lives,
  what "done" means.
- Finishing work? Move it to **Landed** with the commit, or delete it —
  don't leave it ambiguous.
- The user dropping an item explicitly is the *only* other way it leaves
  Hanging.
