# ccg — Error Inventory

The concrete migration state of the [Sacred Error Axiom](../ERROR.md)
inside ccg. The axiom (definition, visual contract, what it forbids,
what it requires) lives in the root `ERROR.md`; this file tracks the
per-site progress and the still-open ccg work.

## Refresh 2026-07-18 — staleness audit

The inventory below was last swept 2026-06-18. A month on, a freshness
pass found:

- **The cited `let _ =` count is stale.** The Engine-internals note
  claims "138" (`grep -rE '^[[:space:]]*let _ =' src/`). The true
  recursive count is now **265**. Growth is mostly test code + new
  features (tap, P.41, palette, absent-control) and is not per-se a
  violation, but the number in the doc is wrong — don't trust it.

- **The CI gate was doubly broken (`sacred-error-check.yml`) — [x] FIXED
  2026-07-18.** Two bugs in how it globbed from shell variables:
  (1) `**` collapsed to `*` (runner bash has `globstar` off) so the Rust
  grep only descended one directory — it saw 170 where truth is 265;
  (2) brace expansion (`{js,html}`) never applies to a glob stored in a
  variable, so the JS patterns matched nothing and `grep -r` fell back
  to scanning the whole tree. Fix: replaced the `**`/`{}` shell globs
  with `grep -r <root> --include=<glob>`, which recurses every depth and
  filters by extension with no shell-glob subtlety. Baselines set to the
  correctly-scoped counts: **rust 265, rust-roam 14, js 14, js-roam 2,
  elm 0** (the JS baselines 14/2 turn out to match the original
  pre-bug values — those were right; only the Rust counts had genuinely
  drifted). Verified: the workflow's exact mechanism now equals every
  baseline.

- **`fitness.rs` failure-detail swallow — [x] FIXED 2026-07-18.**
  `fitness_breakdown` drained `drain_failures()` and kept only the count
  (`failed_games_total += 1`), discarding the detail strings. Now the
  drained details ride out on `FitnessBreakdown.failure_details` (tagged
  with game seed + seat), because the return value is the only channel
  that crosses the rayon worker→main boundary — both the error bus and
  the failure sink are thread-local and die unread on the worker
  (emitting there would have just relocated the drop). The
  picker/resolver sweep test now prints the details on failure, so a
  disagreement is diagnosable from the assert message. The one remaining
  `let _ = drain_failures()` at `fitness.rs` is legitimate (pre-clears a
  prior genome's stale sink on the reused worker; those were already
  attributed to that genome's breakdown).

- **Newly-exposed ccg sites triaged 2026-07-18 — [x] no real swallows.**
  With the gate honest, the ~95 previously-invisible ccg `let _ =` sites
  (top-level `src/*.rs` + two-deep `src/game/play/`, `src/sim/step/`)
  were categorized:
  - `writeln!(buf, …)` to in-memory Strings, `crate::trace::drain()` /
    `crate::error::drain()` / `instrument::drain_failures()` housekeeping,
    `clear_session()` / `remove_file()` / `tsot_*_impl()` test cleanup —
    the legitimate bulk (same classes as the 2026-06-18 sweep);
  - `let _ = self.move_card_or_emit(…)` (7) — already routed; the
    `let _` is on a Result that has *already* surfaced a typed Error;
  - `let _ = cast_iid;` / `let _ = registry;` — unused-param/var
    suppressions, not Results;
  - `oracle.choose_*` / `engine.step(...)` under `#[cfg(test)]`
    (`trace_tests.rs`, `choice.rs` test, `step/tests.rs`) — intentional
    test-state discards.
  No production Result carrying meaningful failure is dropped. The 14 ccg
  JS empty-catches are all the documented `catch(_){}`-around-
  `tsotPushError` defense-in-depth / parse-with-fallback pattern (2026-
  06-18 audit), re-confirmed. **ccg is clean.**
- **roam sweep [ ] open (roam's scope):** roam carries 14 `let _ =` and
  2 JS empty-catches, now correctly visible to the gate but not triaged
  here — TSOT and roam are independent subprojects.

The dated section below is otherwise preserved as-is; treat its
"closed [x]" entries as accurate to 2026-06-18 and its counts as
superseded by the numbers above.

## Unwired — current inventory (2026-06-18)

Sacred-errors is the project's non-negotiable axiom (`CLAUDE.md`).
This section enumerates every known gap so future sessions don't
re-grep to rediscover the work. Tick a box `[x] ~~closed~~` only when
the gap is closed AND user-verified end-to-end (a commit means done
— see `CLAUDE.md`). `[~]` = code shipped but verification debt
remains. `[ ]` = open.

### Hot — panics / active silent drops

- [~] `src/sim/step/combat.rs:116` — `self.state.confirm_attacks().unwrap()`
  **Shipped 2026-06-16**: replaced with match-on-Err that routes
  through `emit_human_refusal` (surface=`"prompt"`, region=
  `"confirm-attacks"`). Cursor advances to DeclareBlockers anyway
  so the game doesn't deadlock. Not yet user-verified end-to-end
  (needs a real refusal to fire).
- [~] `src/sim/step/combat.rs:234` — `self.state.confirm_blocks(...).unwrap()`
  **Shipped 2026-06-16**: same shape, region=`"confirm-blocks"`.
  On Err, skips outcome accounting (no mills/deaths credited) and
  advances cursor. Not user-verified.
- [x] ~~`step_resolve` Err-arm in `src/sim/step/main_phases.rs` missing
  `ChoicePending` intercept. **Shipped 2026-06-16**: intercept
  added; Pending rolls back preview journal, sets cursor via
  `ctx.on_pending(picked, history)`, yields `NeedHuman(...)`.
  Not user-verified end-to-end.~~
  **Verified end-to-end 2026-06-18**: Fireball cast from hand in
  the dev tool — its `on_play` handler calls `game.choose_card`
  (target), the intercept caught the Pending, the dev tool
  surfaced a `NeedHuman(ChooseCard)` prompt, target selected,
  cast resolved, 4 damage landed on opponent. The prompt-instead-
  of-wasm-trap behaviour is the load-bearing observation.

### Engine internals

- [x] ~~**97 `let _ = result;` patterns in non-test code, triaged + helper shipped + full sweep landed 2026-06-18.**~~
  Actual count via `grep -rE '^[[:space:]]*let _ =' src/ --include='*.rs'`
  is 138; the ~80% legitimate are: `writeln!(buf, ...)` to in-memory
  Strings (write never fails), `crate::trace::drain()` / `crate::error::drain()`
  housekeeping at FFI boundaries (idempotent reset), various
  `prompt_tx.send(...)` calls during shutdown (channel closing is
  expected).
  The 21 sites flagged as needing per-site judgment were all
  zone-transition contract violations (`NotInZone` from `move_card`
  / `None` from `remove_from_zone` meant the engine asked to move a
  card that wasn't in the zone it claimed).
  **Helpers shipped:** `GameState::move_card_or_emit(iid, side, from, to, region)`
  AND `GameState::remove_from_zone_or_emit(iid, owner, zone, region)`
  in `src/game/movement.rs`. Both push typed `Severity::Error`
  (`surface="engine"`, region set by caller) on the failure path,
  return the same type as the wrapped method so call sites can still
  branch.
  **Sweep complete:** all 21 sites converted to use the helpers
  with descriptive region labels (`play-mill-cost`,
  `play-gy-cost-auto`, `play-gy-cost-explicit`, `play-jewel-sacrifice`,
  `play-gy-hand-substitute`, `play-sacrifice-cost`, `play-cast-source-remove`,
  `play-self-exile-hand-pay`, `play-hand-payment-discard`,
  `play-hand-payment-attach`, `combat-damage-mill`, `combat-death`,
  `turn-draw-step`, `turn-end-discard`, `cleanup-b8-death`,
  `cleanup-zero-y-death`, `activate-mill-cost`, `activate-graveyard-cost`).
  238 game tests still pass. Future zone-corruption bugs surface
  as typed Errors with their region label naming the call site —
  no more silently-frozen game states.
- [x] ~~`activate_ability` Err paths outside `src/sim/step/`~~
  **Shipped 2026-06-18**: 2 silent-Err arms in `src/sim/run.rs:1025`
  (Main1 human activation) and `src/sim/run.rs:1390` (Main2 human
  activation) now also push a typed `Severity::Error`
  (`surface="engine" region="activate-failed"`) before the existing
  `tee_log` line. The third `activate_ability` call at
  `src/sim/run.rs:1584` is an AI rollout path — its existing log
  capture is the surfacing surface, no additional Error needed.
- [x] ~~`src/card/loader.rs` malformed-card handling~~
  **Shipped 2026-06-18**: `load_cards_dir` and `load_cards_embedded`
  no longer abort on the first malformed card. Each file is
  wrapped in match-on-Err that emits
  `Severity::Warn` `surface="card-loader" region="malformed-card"`
  with the file path + mlua error message, then continues with the
  next file. The corpus boots with known-incomplete state AND the
  developer sees exactly which file rejected.

### JS

- [x] ~~16 of 46 `catch (...)` blocks in `assets/` still silent~~
  **Audited 2026-06-18**: all remaining empty catches in
  `assets/play.html` (14 sites) and `assets/js-bridge.js` (2 sites)
  follow the pattern `try { tsotPushError(...) } catch (_) {}` or
  `}); } catch (_) {}` wrapping a `tsotPushError(...)` call. These
  are documented defense-in-depth per the 2026-06-11 sweep — they
  only fire if `tsotPushError` itself throws (port broken, Elm
  app gone). The 2 remaining `JSON.parse` catches at
  `assets/play.html:246` and `:273` are parse-with-fallback
  patterns where the fallback IS the recovery (errEvent → null
  means no event log entry; info → {message: raw} means show the
  raw line). No real silent drops remain.
- [ ] **ERROR.md Slice 1 deferred bullet, still open:** `LogPanel.ErrorEntry`
  → `Error.view` collapse. The two parallel renderers exist:
  `LogPanel.viewErrorBlock` in `assets/src/LogPanel.elm:92-104`
  and `Error.view` (`assets/src/Error.elm`). The source-specific
  fields (`source`, `ffiCall`, `location`, `jsStack`, `rawStderr`,
  `requires_reload`) that previously blocked the collapse are now
  on the canonical `Error` type (per `crates/sacred-error/src/lib.rs`).
  Migration slice (deferred): convert `LogErrorReceived` in Main.elm
  to decode into `Error.Error`, push to `model.errors` instead of
  `model.log`, delete `LogPanel.ErrorEvent` + `LogPanel.viewErrorBlock`,
  drop the `.log-error*` CSS from `assets/play.html` style block.
  Bigger refactor (~6 sites in Main.elm send to model.log today);
  not done because it crosses every error path and benefits from
  user-verification at each.

### Elm

- [x] ~~53 `Maybe.withDefault` + 3 `Result.withDefault` triage~~
  **Audited 2026-06-18**: 56 total occurrences. The 3
  `Result.withDefault` sites are:
  (1) `Main.elm:552` — inside a doc comment, not live code;
  (2) `Main.elm:820` — `presetCountFromJson |> Result.withDefault -1`
      for a diagnostic line where -1 is the sentinel meaning "see
      the typed-error pushed nearby" — not a swallow;
  (3) `Main.elm:977` — `Result.withDefault GameScreen.LoadingPrompt promptResult`
      ALREADY routed through `maybePushDecodeError` at line 1015.
      The Result.withDefault provides the safe rendering fallback;
      the helper surfaces the typed Error in parallel.
  The 53 `Maybe.withDefault` sites are all "Maybe carries
  'absent', not 'failed'" patterns — Dict lookups, optional
  card fields (iid, printedPower for non-creatures), spectator
  slice when no spectator is connected, decoder defaults for
  optional JSON fields. No axiom violations.
- [x] ~~`LogPanel.elm`, `GameScreen.elm`, `SpectatorBar.elm`, `BuildFooter.elm` unswept~~
  **Audited 2026-06-18**: none of these four modules decode raw
  JSON port payloads (only `Main.elm` receives `errorIn`,
  `gameStateIn`, etc.). Their `case ... of` blocks pattern-match
  on typed data variants, not on `Result e a`. Zero `Err _ ->`
  arms across all four. Their `Maybe.withDefault` uses are
  legit absent-not-failed. No sweep work needed; the surface was
  already concentrated in `Main.elm` by the port-architecture.

### Architectural gaps (need engine work, not just wiring)

Each item carries a tight design slice. Implementation is engine
work, not session-feasible alongside the broader sweep. Mark `[x] ~~closed~~`
when the slice ships AND a real card exercises the path end-to-end.

- [ ] **Graveyard payment human choice.** When a cast has a GY cost
  source and the player has MULTIPLE color-anchor-satisfying cards
  in their graveyard, `resolve_graveyard_payment` picks
  deterministically — the human never gets to choose which card
  pays. Fine for AI rollouts; wrong for human agency.

  **Design slice:** mirror the existing HAND-payment human picker
  (`src/game/play.rs` HAND-payment branch). When `cost.source == Graveyard`
  AND `state.active_player == Human`, yield `NeedHuman(ChooseCard {
  pool: gy_iids_matching_anchor, prompt: "pay from graveyard:",
  optional: false })` BEFORE consuming `gy[0]`. Resume drives the
  consumption from the chosen iid. Add one card-level integration
  test that pins: with 2 GY-eligible cards, the engine yields a
  NeedHuman; with 1, it resolves silently (today's path); with 0,
  the cast fails with the existing "graveyard cost not payable"
  error.

- [ ] **Variable-X cast-time prompt.** No engine path yields
  `NeedHuman(ChooseInt)` for X before `play_card` runs. Read the
  Embers + every X-cost card can only be cast via the Lua-yield
  workaround.

  **Design slice:** card schema gains `cost[i].is_x` (already
  present). When `state.play_card` enters and ANY cost component
  has `is_x == true`, yield `NeedHuman(ChooseInt { min: 0, max:
  player.life, prompt: format!("X for {}:", card.name) })` BEFORE
  the cost-payment loop. The resume binds `x_value` and threads
  through `resolve_cost_components`. Existing tests:
  `src/game/play_tests.rs` has the AI path; mirror with a Human
  test scripting ChooseInt(3).

- [ ] **Cast-time targeting** for spells with declared targets
  (Fireball, every "target a creature" spell). Card schema needs
  `target: Option<TargetSpec>`; engine yields a `NeedHuman(ChooseCard)`
  BEFORE handing to `on_play`; R.1.a response window fires with
  the target locked. Lua reads `game.cast_target()`.

  **Design slice:** add `TargetSpec { zone: Zone, kind: CardKind,
  controller: TargetController, optional: bool }` to the card
  schema. `play_card` checks `card.target.is_some()` — if so,
  yields `NeedHuman(ChooseCard { pool: filter(spec) })` BEFORE
  the on_play handler. The chosen iid stashes in a new
  `state.cast_target: Option<InstanceId>` slot (cleared after
  on_play resolves). Lua's `game.cast_target()` reads from the
  slot. Closes the Fireball "Y/N then choose" workaround.

- [ ] **Activation flow through Main1/Main2.** Engine surfaces a
  typed Error saying activations aren't supported, but doesn't
  yet route a clicked activation through the cursor/oracle path.
  Signal Goblin, jewel hand-pay, etc. block on this.

  **Design slice:** the existing `state.activate_ability(...)`
  surface in `src/game/play/activate.rs` IS the entry point. The
  remaining wiring is the cursor side: the JS-side dispatcher
  currently has no "activate" message (only "pass", "play",
  "respond"). Add `MainPhaseChoice::Activate { iid, ability_index,
  x }` to `src/sim/human.rs` (already exists per grep at
  `src/sim/run.rs:1020`) — but the JS click-handler doesn't yet
  produce it. Wire `assets/play.html`'s board-card click handler:
  if the card has an activated ability and meets validate(),
  show an "Activate" affordance; clicking sends the typed action
  through the existing `tsot_apply_action` FFI. Add a test card
  with a tap-cost activated ability and pin a cast→activate flow.

### Self-enforcement holes

- [x] ~~`every_step_file_references_emit_human_refusal` extension~~
  **Shipped 2026-06-18**: new test
  `every_pipeline_boundary_file_references_typed_error` in
  `src/sim/step/tests.rs:1037`. Coverage list: `src/game/lua_api.rs`,
  `src/game/play.rs`, `src/wasm_ffi.rs`, `src/card/loader.rs`,
  `src/sim/mcts.rs`, `src/sim/run.rs`. Each file must reference
  the typed Error pipeline (`crate::error::emit_region` /
  `emit` / `push` / `emit_human_refusal`); deletion of every
  sweep site in a covered file fails this test. Elm-side
  enforcement is the CI grep below (Elm tests can't grep the
  source tree as easily).
- [x] ~~CI grep that fails the build when a PR introduces a new
  silent-drop pattern~~
  **Shipped 2026-06-18**: `.github/workflows/sacred-error-check.yml`.
  Tracks 5 patterns with baselined counts in
  `.github/sacred-error-baseline/`. PRs that grow a count fail
  the build with a hint at the offending lines. Counterpart to
  the in-tree test above; this catches the Elm + JS surfaces
  the Rust test can't see.
- [x] ~~**Elm port allowlist** — counterpart to `TRACE_STRING_ALLOWLIST`
  for engine port payloads, so port-payload shapes can't drift
  silently the way `outcome: String` did before `OutcomeRepr`.~~
  **Shipped 2026-06-18**: `assets/tests/PortShapeTest.elm` ships
  the scaffold with two ports covered (`errorIn` via `Error.decode`,
  `logErrorIn` via `LogPanel.decodeError`) including the
  axiom-enforcement test: "unknown severity must FAIL decode" passes
  (no silent default). 5 tests pass under `nix develop -c
  elm-test tests/PortShapeTest.elm`. Each port covered by 1-3 test
  cases (canonical sample, minimal-with-optional-omitted sample,
  optional negative case). Adding a port: copy the pattern, add a
  describe block, ship the canonical sample. The remaining inbound
  ports to cover one-by-one as their shapes stabilize:
  `gameStateIn`, `logTextIn`, `spectatorStateIn`, `buildInfoIn`,
  `bootDataIn`, `gameMetaIn`, `uctPreviewIn`, `decisionLogIn`,
  `savedListIn`, `saveStatusIn`, `gamePhaseIn`, `promptTextIn`.
  The 12-port list is the upper bound of future entries; each is
  ~15 lines of test code following the same shape.

### Verification debt (shipped but not user-confirmed end-to-end)

Each entry carries a repro recipe so the user can verify by
following the steps in one sitting. Tick `[x]` only after the
described observation lands.

- [ ] **Build watermark visible on every Error window across all
  surfaces** (only confirmed on Test Panic + Read the Embers refusal).
  Repro: in the dev tool, trigger an error on each of:
  deckbuilder (load broken preset), prompt (refuse confirm
  attacks), game-screen (force decode failure), spectator-bar
  (point at non-existent session), build-footer (corrupt
  build-info port payload). Each Error overlay must show the
  `dev abc123 · 2026-06-18T...` watermark in the bottom-right.

- [ ] **Read the Embers cast completes once `step_resolve`
  ChoicePending intercept lands.** Repro: deck a Read the Embers
  + enough mana, click it from hand, the `on_play` handler calls
  `game.choose_card` for the discard target; the prompt should
  appear (not a wasm trap). Pick a card, the cast resolves,
  damage lands.

- [ ] **Spectate path error surfacing under a real failure.**
  Repro: from the spectator URL, send a malformed
  `spectatorStateIn` payload (e.g. fetch `/api/spectator` and
  intercept the response with a missing field). The Error
  overlay anchors inside the spectator-bar container.

- [ ] **Save / Load error surfacing under a real failure.**
  Repro: click Save, edit the JSON in localStorage to break a
  required field (e.g. delete `card_pool`), reload, click Load.
  The Error surfaces inline at the Save/Load action.

- [ ] **Deckbuilder boot error surfacing on a deliberately broken
  preset.** Repro: edit a card in `cards/*.lua` to reference a
  non-existent card_id in a preset, reload the dev tool. The
  deckbuilder dropdown shows the typed Warn inline at the
  preset name.

### Doc / axiom open items

- [~] **Slice 6 — `localStorage` persistence + bijectivity invariant
  for `Error.id` → DOM node.**

  **Port foundation shipped 2026-06-18**: `assets/src/Main.elm`
  now declares `port errorPersistOut : E.Value -> Cmd msg` and
  `port errorRestoreIn : (D.Value -> msg) -> Sub msg`. Main.elm
  compiles clean (`elm make src/Main.elm --output=/dev/null` →
  `Success! Compiled 1 module.`). Ports are foundation-only — the
  full Slice 6 wiring still needs:
  1. A `RestoreErrors (Result D.Error (List Error.Error))` Msg variant
     + `update` arm that seeds `model.errors` from the decoded list.
  2. A `subscriptions` entry pulling from `errorRestoreIn`.
  3. Every `errors = model.errors ++ [...]` call site also dispatches
     `errorPersistOut (Error.encodeList model.errors')` — likely via
     a helper `Error.persist` to keep the call shape one-line.
  4. JS-side glue in `assets/play.html`:
     `app.ports.errorPersistOut.subscribe(payload => localStorage.setItem('tsot_errors_v1', JSON.stringify(payload)))`
     and at boot: read `localStorage.getItem('tsot_errors_v1')`, parse,
     dispatch through `app.ports.errorRestoreIn.send(...)`.
  5. Cap-at-100-FIFO eviction on the JS side so a runaway error
     producer can't exhaust localStorage.
  6. PortShapeTest entries for the `errorRestoreIn` payload shape
     (an array of Error objects).
  Bijectivity invariant: `Html.Keyed` already groups errors by
  `Error.id`. Since the persisted Errors retain their ORIGINAL id
  on restore (the JS side stores the encoded JSON verbatim), a
  reloaded session sees the prior-session errors at the same DOM
  position they had before — same `id`, same key, same node. The
  invariant is structural: any code path that re-assigns ids on
  restore would break it; the current design preserves ids by
  round-tripping the JSON.

- [ ] **TSOT OBSERVABILITY.md Phase 2 — AI-internals narration
  (O6, O7, O8)** so UCT opponent reasoning surfaces.

  **Design slice:** UCT today logs via `tsot_emit_iteration_event`
  to the worker → main pipe (per `WASM_PLAN.md` D5). The remaining
  internals to narrate: per-node statistics (visits, value
  estimate), the selected expansion path per iteration, and the
  final tree summary at decision time. Three new `TraceEvent`
  variants — `UctNodeStats`, `UctExpansionPath`, `UctTreeSummary`
  — pushed from `src/sim/uct.rs`. Frequency-cap the per-iteration
  ones (every 50th iteration or every 100ms, whichever is rarer)
  to avoid bus overflow during 10k-iteration searches.

- [ ] **TSOT OBSERVABILITY.md Phase 5 — UI filter chips +
  click-to-expand for the LOG.**

  **Design slice:** the LOG today is a single scrolling pre-formatted
  block (`assets/src/LogPanel.elm`). Phase 5 wraps it in a
  filter row (chips per source: `engine`, `lua-handler`, `uct`,
  `ffi`, `error`) and makes each entry click-to-expand (the
  full trace stack shows only when clicked). Implementation:
  `LogPanel.Entry` keeps the data, `LogPanel.viewEntry` gains
  an `expanded: Set Int` model field (which indices are open);
  the filter row is a top-of-component chip set whose
  `(active: Set String)` model field gates which entries
  render. State lives in `Main.Model.logFilter` and
  `Main.Model.logExpanded`. No engine changes.
