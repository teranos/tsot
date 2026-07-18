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
- **P.13 / bitter-dawn** resolved: `hand` cost is now legal on any card
  type; on a spell the payment discards to the graveyard (the engine
  already did this). Rule amended, card unchanged. RULES.md P.13 + P.42c.
- **Spirit Wanderer** — black/purple spirit, `1 gy + 3 tap`,
  graveyard-only cast, haste, 3/1. Built + tested (casts from graveyard
  paying gy+tap, refused from hand). Now in the live corpus.
- Save-compat guard: picker skips hand-casting graveyard-only cards
  (P.41b, keeps picker/resolver agreement); `from_graveyard` given
  `#[serde(default)]` + a legacy-load regression test so old saves load.
- **Error-swallow audit** — fixed the `fitness.rs` failure-detail swallow
  (details ride out on `FitnessBreakdown.failure_details`, sweep surfaces
  them); fixed the sacred-error CI gate (was doubly broken: `**` needed
  globstar, `{}` never brace-expands from a variable → switched to
  `grep -r --include`, re-based baselines to correct counts). Swept every
  swallow shape (gated + ungated) across ccg — **no production swallow
  remains; ccg swallow audit complete** (see ERROR_INVENTORY 2026-07-18).
  roam's now-visible sites are roam's scope.
- **Blocking taps the blocker (B.19) + vigilance→blocking (B.16)** —
  `declare_blocker` taps the blocker at declaration (unless vigilant) and
  fires `OnTapped`, mirroring the attack tap. TDD, committed.
- Cast-from-graveyard (P.41 a–d) **engine** — `cast_zones` card schema,
  P.41a cast from GY, P.41b graveyard-only refused from hand
  (`NotCastableFromZone`), P.41c no self-payment, P.41d spell exiles on
  resolution / counter (board destinations unchanged). TDD, committed.

---

## Hanging

### Sim-AI / picker coverage
13. **Sim-AI does not offer graveyard casts.** The P.41 engine accepts a
    cast from the graveyard when the resolver is driven with it, but the
    sim-AI picker (`sim/run.rs` choice builder + `sim/ai.rs`
    affordability) only enumerates HAND casts, so `make evolve` / probe
    never *try* casting from the graveyard. No picker/resolver
    disagreement (the picker just doesn't offer it), but graveyard-cast
    cards are under-explored until the builder learns the zone. Same
    shape as the tap-cost sim-AI work already done.

### Rules reconciliations flagged, then deferred ("ignore for now")
   These were set aside, NOT cancelled — kept per the operating rule.
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
9. **Flaky test `diversity_alpha_widens_final_population_diversity`.**
   Root cause still unknown — the crystal-tap picker bug (since fixed)
   was a red herring, present in passing runs too. The diagnostic blocker
   is now removed: #8 (the fitness swallow) is fixed and the
   picker/resolver sweep surfaces failure detail, so the next recurrence
   shows the *why* instead of a bare count. Root cause still to be caught
   in the act.
10. **DRY: `CostNeeds` 8-site duplication.** Same cost-aggregation shape
    repeated ~8 places; candidate for a shared helper. Not done.
11. **DRY: `substitution_coverage`.** Flagged as a dedup opportunity.
    Not done.
12. **Cyclo / size: split `play_card_inner` (~886 lines).** Now that
    `validate_play` exists as a pure dry-run validator, the validation
    half can be lifted out of the resolver. Not done. (Other large files
    were flagged in the same survey — revisit after this one.)

---

## Dropped (WONTDO)

Explicitly closed by the user — recorded so they don't resurface.

- **Quorum** (2026-07-18) — the earlier card design that was rejected and
  respecified as Ankle Scorcher. Not a separate card; WONTDO.

---

## How to use this

- Adding work? Append to **Hanging** with: what, why, where it lives,
  what "done" means.
- Finishing work? Move it to **Landed** with the commit, or delete it —
  don't leave it ambiguous.
- The user dropping an item explicitly is the *only* other way it leaves
  Hanging.
