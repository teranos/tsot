# tsot ccg — Handover

> Cold-pickup handover for the `claude/ccg-directory-payment-gb1s8y` line
> of work. Last updated 2026-07-18. Everything actionable and open is a
> checkbox below; ticked items are context so nothing gets redone.

## How to resume

- [x] Branch `claude/ccg-directory-payment-gb1s8y`, pushed to origin
      (HEAD `5dd5ba0`). No PR opened (none was requested).
- [x] Full lib suite green at handover: **588 passed, 0 failed, 2
      ignored**. Verify with `cd ccg && cargo test --lib` (write to a
      file; the run is ~5 min — never `| tail`/`| head` it).
- **Operating rule (carry it forward):** a later request never supersedes
  an earlier one. Narrowing scope or "ignore for now" is *deferral*, not
  cancellation. An item leaves the open list only when done + verified,
  or explicitly dropped by the user (see Dropped).

## Open — actionable

### Rules decisions (user's call, not code)
- [ ] **Z.8c "never tap" exclusion list.** Decide whether `tap` (P.42)
      should be added to Z.8c's exclusion list. Deferred earlier
      ("ignore for now"), never cancelled. Needs a ruling before any code.

### Cards & probe
- [ ] **Power-axis probe comparison (2 vs 3 power) — owed.** The 2×2 grid
      ("4 variants") was requested, then a run was narrowed to "only 1 vs
      2 tap cost" and the `power2` / `tap2-power2` variants were removed
      from `cards/ankle-scorcher.lua`. Narrowed ≠ dropped. Next step:
      restore those two variants (recoverable from commits `043f432` /
      `284ccb6`) and run `balance-probe ankle-scorcher`.
- [ ] **Resolve the tap 1-vs-2 gap beyond noise.** The daily-fast budget
      (`pop 8 / gens 4 / n 3`, single seed) gave Δbest 0.000 and a small
      Δmean — inside the probe's noise band. Next step: rerun
      `balance-probe ankle-scorcher --pop 24 --gens 12 --n 10`. (Probe
      only when asked — per CLAUDE.md.)

### Sim-AI / picker
- [ ] **Picker doesn't offer graveyard casts (P.41 follow-up).** The
      engine accepts a cast from the graveyard, but the picker
      (`sim/run.rs` `build_pattern_b_choices` + `sim/ai.rs`
      affordability / `enumerate_playable_in_hand`) only enumerates HAND
      casts. So `make evolve` / probe never *try* graveyard casts, and
      **Spirit Wanderer is inert to the sim AI** until this lands. No
      picker/resolver disagreement today (the picker already skips
      hand-casting graveyard-only cards), just under-exploration. Same
      shape as the tap-cost sim-AI work already done. Must preserve
      picker/resolver agreement when added.

### Engineering hygiene
- [ ] **Flaky test `diversity_alpha_widens_final_population_diversity`.**
      Root cause still unknown (the crystal-tap picker bug was a red
      herring, present in passing runs too). The diagnostic blocker is
      now GONE — the fitness failure-detail swallow is fixed and the
      picker/resolver sweep surfaces detail — so the next recurrence
      shows the *why*. Next step: catch it in the act (loop the test to a
      file, read the surfaced detail; do not guess).
- [ ] **DRY: `CostNeeds` 8-site duplication.** Same cost-aggregation
      shape repeated ~8 places; extract a shared helper.
- [ ] **DRY: `substitution_coverage`.** Flagged dedup opportunity.
- [ ] **Cyclo / size: split `play_card_inner` (~886 lines).**
      `validate_play` now exists as a pure dry-run validator, so the
      validation half can be lifted out of the resolver. Other large
      files were flagged in the same survey — revisit after this.

### Errors (ERROR.md axiom)
- [x] **ccg swallow audit — complete.** Every swallow shape (gated +
      ungated) swept; no production swallow remains in ccg. Details in
      `ERROR_INVENTORY.md` (2026-07-18 refresh).
- [ ] **roam error sweep (roam's scope).** roam carries 14 `let _ =`, 2
      paren empty-catches, and 2 arrow `.catch(()=>{})` (one a comment,
      one justified inline) — now correctly visible to the fixed CI gate
      but not triaged here. TSOT and roam are independent subprojects.
- [ ] **Optional gate hardening.** Add an arrow-form `.catch(()=>{})`
      pattern to `sacred-error-check.yml` (ERROR.md forbids it; the gate
      only checks the paren form). ccg baseline would be 0. Deferred —
      there are none in ccg today.
- [ ] **Older ERROR_INVENTORY open items** (pre-this-arc, dated
      2026-06-18 — see that file, don't duplicate): architectural gaps
      (graveyard-payment human choice, variable-X cast prompt, cast-time
      targeting, activation flow through Main1/Main2); verification-debt
      repro items (build watermark, Read-the-Embers, spectate / save-load
      / deckbuilder error surfacing); doc/axiom items (Error Slice 6
      localStorage persistence, `LogPanel.ErrorEntry`→`Error.view`
      collapse, OBSERVABILITY Phase 2/5).

## Landed this arc (context — do not redo)

- [x] `tap` cost source (P.42): engine + sim-AI affordability +
      picker/resolver agreement (`validate_play`, sweep test).
- [x] Tap Dance card (P.42, cost + untap/tap effect).
- [x] `OnUntapped` engine event (mirror of `OnTapped`) + Ankle Scorcher
      (tap cost + discard-on-untap). LUA.md event list updated.
- [x] RULES ratifications: P.42 (tap), P.41 (cast-from-GY), A.13
      (tap/untap effect), B.19 (blocking taps) + B.16 (vigilance→block);
      P.13 amended (spells may use `hand`; Bitter Dawn led). P.42c fixed.
- [x] Probe deck construction constrained to the probed card's
      color/symbol palette (`sim::palette`, probe-only).
- [x] Probe absent-control cell + Δ-vs-control reporting.
- [x] CLAUDE.md probe rule fixed ("unless asked").
- [x] **P.41 cast-from-graveyard engine** (a–d) + **Spirit Wanderer**
      (real-card tests) + save-compat guard (`serde(default)` +
      legacy-load test; picker skips hand-casting graveyard-only cards).
- [x] **B.19 blocking taps the blocker** (+ B.16 vigilance→blocking).
- [x] **Error audit:** fixed `fitness.rs` failure-detail swallow (details
      on `FitnessBreakdown.failure_details`); fixed the sacred-error CI
      gate (was doubly broken — `**`/`{}` shell globs → `grep -r
      --include`, baselines corrected).

## Dropped (WONTDO)

- [x] **Quorum** (2026-07-18) — the earlier design rejected and
      respecified as Ankle Scorcher. Not a separate card.

## Related trackers

- `ERROR_INVENTORY.md` — error-axiom migration state (the error-specific
  open items above live there in full).
- `LIMITATIONS.md` — what the engine cannot do today.
- `RULES.md` — rule text (P.41, P.42, B.19, B.16, P.13 are current).
