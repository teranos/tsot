# Comprehensive DRY Analysis

Date: 2026-06-28
Scope: full monorepo — `ccg/` (~43k LOC Rust + TS/JS), `roam/` (~12k LOC
Rust + TS/JS), `rave/`, `crates/`, `universe/`.

This report has two parts: **within-project** duplication (the bulk, below) and
a dedicated **[cross-project](#cross-project-analysis)** section at the end. The
root `CLAUDE.md` calls the subprojects independent, but that boundary is worth
testing empirically — and the cross-project pass found the single largest
duplication in the whole tree (~370–400 lines of Bevy scaffold shared verbatim
between `rave` and `universe`), plus byte-identical identity-crypto across `roam`
and `rave`. Where a parallel is genuinely intentional (different domains,
different FFI targets, documented axioms), it's labelled as such rather than
flagged as debt.

Each finding lists concrete `file:line` sites, the extent of repetition, and a
proposed fix. Severity: **high** = substantial (>15 lines or >3 sites) with a
clear safe fix; **medium** = moderate; **low** = minor/stylistic.

---

## Executive summary

The tree is, on the whole, a disciplined codebase: shared error types are
already factored (`crates/sacred-error`), single-sources-of-truth exist for the
dangerous stuff (`CastRouting`, `world_hash`, `tile_palette`, `report_style`),
and genuinely distinct algorithms (UCT vs one-ply MCTS, attacker vs blocker
selection) are correctly kept separate rather than wrongly merged. The real debt
is **localized and safe to remove** — almost all of it is mechanical, test-
covered, and concentrated in a handful of well-understood shapes.

### Top opportunities, ranked by lines removed × safety

| # | Area | What | Approx. lines | Sites | Sev |
|---|------|------|---------------|-------|-----|
| 1 | ccg CLI/reports | Compute-aggregates-then-render duplicated stdout vs HTML | ~250 | 2 files | high |
| 2 | ccg sim/run.rs | Pattern-B cost-fill + tap-substitution copied across cost branches | ~200 | 3–4 | high |
| 3 | rave tests | Two integration tests are ~2/3 identical relayer harness | ~180 | 2 | high |
| 4 | ccg CLI | Deck-directory JSON loading reimplemented | ~150 | 7–8 | high |
| 5 | roam render_gl | WebGL program-build + per-frame draw boilerplate | ~140 | 4–5 | high |
| 6 | ccg game | Cost-aggregation / sacrifice-validation / death-trigger blocks | ~130 | 2–4 | high |
| 7 | ccg play_tests | `PlayChoices { … }` full literal instead of `..Default::default()` | ~350 | 44 | high |
| 8 | ccg wasm_ffi | FFI export wrappers + envelope prologue/epilogue + AI-parse | ~175 | 9/4/3 | high |
| 9 | ccg CLI | `--opponent-ai`→`AiKind` resolution + Jaccard clustering | ~120 | 5/3 | high |

### Cross-cutting themes (each recurs in 2+ areas)

1. **AI-name → `AiKind` parsing** — duplicated **8× across ccg alone**: 5× in
   CLI binaries (`parse_opponent_ai`-shaped) and 3× in `wasm_ffi.rs`
   (`parse_ai_kind`-shaped). These are in different crates-of-concern but the
   same domain type; a single `tsot::sim::ai::parse_ai_kind(name, seed)` could
   serve both.
2. **Compute-then-render-twice** — every report computes its aggregates once for
   ANSI stdout and again for HTML/maud (`cli_matchup_evolved`,
   `cli_champions_report`/`champions_report`). The fix is always the same:
   compute into a struct, feed both renderers.
3. **Error-envelope construction** — the same ~13-field `Error { … }` /
   trace-breadcrumb literal recurs within each project (ccg `error.rs`×2 +
   `trace.rs`×2; rave `net_glue`/`chat` ×15; roam JS error shape ×8).
4. **FFI export wrappers** — null-check/decode/match shims repeat per-export in
   both `ccg/src/wasm_ffi.rs` (9×) and `roam/src/wasm_ffi.rs` (3×); macro-shaped.
5. **Counter/telemetry restatement** — adding one counter is a 3–4-site edit in
   `roam/src/perf.rs` and the ccg `card_play_turns` telemetry block (3×).
6. **Test-harness boilerplate** — the single largest raw line count
   (rave relayer harness, ccg sim setup ~450 lines, ccg `PlayChoices` ~350
   lines), but the lowest risk.

### Suggested sequencing

Do the **safe mechanical wins first** (no behavior change, test-covered):
#7 `..Default::default()` in play_tests, #4 shared deck loader, the telemetry/
counter helpers, `PlayerId::index()` substitution, `..Default` for `GameStats`.
Then the **structural extractions** that also remove correctness risk: the
`NetEvent`→trace mapping in roam (already drifted), the Pattern-B cost blocks,
the GY-anchor picker/resolver split in ccg game. Defer anything entangled with
deprecated code: ccg `run_game_continue` is slated for D8 deletion, which
resolves a chunk of the sim duplication for free.

For cross-project work the ranking is separate — see the
[cross-project section](#cross-project-analysis). The headline there: extract a
`crates/bevy-observability` plugin (rave+universe), a `crates/p2p-identity` crate
(roam+rave), one root `flake.nix`, and one reusable deploy workflow.

---

## ccg — `game/` module

### [high] Death-detection + on_die/OnCreatureDies broadcast duplicated
- `ccg/src/game/combat.rs:502-567`, `ccg/src/game/play.rs:1421-1495`
  (`cleanup_zero_y_deaths`), `ccg/src/game/play.rs:1375-1413`
  (`cleanup_b8_damage_deaths`); broadcast sub-block also at `play.rs:891-916`.
- The "snapshot boards → filter creatures → death predicate → Board→Graveyard →
  fire OnDie → broadcast OnCreatureDies → exile_remaining_attached" sequence is
  implemented 3× (the fan-out tail 4×). Only the predicate and Pending-handling
  differ. ~40–65 lines × 3.
- Extract `fn fire_death_triggers(dying_iid, ctx) -> Result<(), ChoicePending>`
  for the tail and `fn collect_creature_deaths(predicate)` for the scan.

### [high] Per-source cost aggregation loop duplicated 4×
- `play.rs:245-264`, `play/activate.rs:117-137`, `play/activate.rs:397-411`,
  `state.rs:722-733`.
- The `for c in &cost { amount = if c.is_x {x} else {…}; match c.source { Hand =>
  …, Mill => …, Graveyard => …, Sacrifice => … } }` accumulation. ~12–18 lines × 4.
- Extract `struct CostNeeds { … }` + `fn aggregate_cost_needs(components, x)`.

### [high] SACRIFICE-payment validation duplicated between cast and activate
- `play.rs:697-730`, `play/activate.rs:147-181`. ~33 lines × 2; differ only in
  `PlayError::*` vs `ActivateError::*`.
- Extract `fn validate_sacrifice_ids(...) -> Result<(), SacError>`; each caller
  maps `SacError` into its own enum (the `WrongSacrificeCount`/
  `SacrificePaymentInvalid`/`DuplicateSacrifice` triad already exists in both).

### [high] `PlayChoices { … }` full literal in play_tests.rs (44×)
- `ccg/src/game/play_tests.rs` — 44 all-fields literals (e.g. `:37-45`, `:87-95`).
  `PlayChoices` already derives `Default`. ~350 lines of boilerplate.
- Use `PlayChoices { hand_payment_ids: vec![p], ..Default::default() }`. Pure
  mechanical win; makes each test's intent visible and kills the 44-site edit
  cost of adding a field.

### [medium] `PlayerId → 0/1` match re-inlined 8× instead of `index()`
- `state.rs:28-29,365-366,553-554,1040-1041,1069-1070`,
  `journal.rs:227-228,248-249`, `play.rs:168-171`. `PlayerId::index()` exists at
  `state.rs:26`. Pure substitution.

### [medium] `zone_mut` zone-match duplicated 5×
- `movement.rs:13-21` and `journal.rs:211-219` are **byte-identical** private
  `zone_mut` fns; re-inlined at `state.rs:827-833,974-980,996-1002`.
- Promote one `PlayerState::zone_mut(zone) -> &mut Vec<InstanceId>` + read-only
  variant.

### [medium] GY color-anchor pitch selection duplicated (picker vs resolver)
- `play.rs:401-444` (`auto_gy_pitch`) vs `play/payments.rs:296-346`
  (`resolve_graveyard_payment`). ~40 lines × 2; the surrounding comments warn this
  exact picker/resolver split previously caused rollout hangs.
- Have `play_card_inner` call `resolve_graveyard_payment` (the documented single
  source of truth) instead of re-deriving `auto_gy_pitch`. **Correctness risk.**

### [medium] `card.colors → lowercased set` extraction repeated 4–6×
- `play.rs:287-297,457-467`, `play/payments.rs:305-315,320-330`,
  `state.rs:1352-1360,1362-1370`. Extract `fn printed_colors_lc(iid) ->
  BTreeSet<String>`.

### [medium] `ActivatedAbility { … }` literal in tests (6×)
- `play/activate.rs:489-498,568-582,645-659,713-722,771-780`,
  `lua_api.rs:1693-1702`. Add `fn test_activation(effect, cost, zones)`.

### [low] `lua_api` fire-helpers share an error tail; `Eot*` predicate spelled 2×
- `lua_api.rs:1405-1638` (extract just the error tail; bodies differ by arity so
  full unification is awkward); `state.rs:879-884` vs `:891-897` — extract
  `fn is_eot_modifier(m)`, which also fixes a latent disagreement with
  `journal.rs:534` (retains only `EotStatBoost`).

---

## ccg — `sim/` module

### [high] Triplicated jewel/crystal/symbol tap-substitution
- `run.rs:430-458`, `:499-529`, `:578-606` (~28 lines × 3); a 4th near-clone in
  `ai.rs:315-360` (`can_pay_instant_cost`).
- Extract `fn apply_tap_substitution(...)`; share the coverage math with `ai.rs`.

### [high] Parallel hand/gy cost-fill across Pattern-B branches
- `run.rs:481-550` (Creature) vs `:559-627` (Spell/Artifact/Mutation/Unspecified),
  ~60 lines × 2, plus the X-branch `:394-479` as a third variant. Collapse the
  arms; only the trailing mutation-target pick is conditional.

### [high] play_card resolve lifecycle duplicated (StepEngine vs run_game_continue)
- `sim/step/main_phases.rs:728-944` (`step_resolve`) vs `run.rs:1097-1265`; the
  card-played telemetry block is **byte-identical** at `run.rs:1177-1209`,
  `main_phases.rs:441-471`, `:882-912`; sacrifice-telemetry identical at
  `run.rs:1134-1138`, `main_phases.rs:357-370`, `:803-816`.
- Extract `record_card_played(...)` and the sacrifice-telemetry loop now (safe).
  The broader flow resolves when `run_game_continue` (deprecated, D8) is deleted.

### [medium] `pick_play` (mcts) vs `pick_play_uct` share candidate preamble
- `mcts.rs:153-178` vs `uct.rs:386-425` — dedup + empty/single fast-paths +
  truncate. Extract `fn prepare_candidates(...) -> CandidatePrep`. **Do not**
  merge the search cores below this point.

### [medium] `emit_mcts_ai_pick` / `emit_uct_ai_pick` identical but for the tag
- `mcts.rs:224-245` vs `uct.rs:597-618` (only `"Mcts"` vs `"Uct"`). Single
  `fn emit_ai_pick(ai, scored, chosen, t0)`; fold in the heuristic picker's inline
  copy at `ai.rs:155-171`.

### [medium] `GameStats { … }` initializer written out twice
- `sim/step/mod.rs:288-327` (`fresh_game_stats`) vs `run.rs:804-841`. ~40 fields.
  `impl Default for GameStats` (or call `fresh_game_stats`); removes the S3
  parity hazard.

### [medium] Mirror-game A/B shuffle-and-play duplicated in fitness.rs
- `fitness.rs:142-159` vs `:160-177`. Extract `fn play_one_side(..., my_seat)`.

### [medium] Test boilerplate — registry-load + mirror-deck + trace extraction
- `sim/step/tests.rs` 8 setup sites + ~18 mirror-deck idioms;
  `trace_tests.rs`/`ai_trace_tests.rs` ~23 `drain → find_map(match …)` blocks; 6
  inline trace-resets despite `fresh_trace()` existing; 11 step-loops despite
  `drive_to_prompt` (`tests.rs:664-694`) existing. ~450 lines, low risk.
- Add `vanilla_engine()`, `mirror_state(template)`, generic `extract<T>(events,
  pred)`; route loops through `drive_to_prompt`.

### [low] `CostSource → char` in snapshot.rs (2×); stats `bump_*` + deck→ids folds
- `snapshot.rs:186-193` vs `:208-215`; `stats.rs:81-120`; deck→`Vec<String>` at
  `parallel_eval.rs:100`, `genome.rs:149/213/238`, `fitness.rs` (7×); count-into-
  BTreeMap at `ops.rs:115/287`, `genome.rs:174/298`, `evolve.rs:352`. Add
  `deck_to_ids(...)` + `count_ids(...)` (several sites are non-test).

---

## ccg — core domain (`card`, `choice`, `trace`, `error`, …)

### [high] Per-intent target-scoring functions share a scaffold (7×)
- `choice.rs:577-606,634-656,664-676,682-700,710-731,740-759,767-788`. The
  `OnAttachedAsCost` `contains_key` block alone is copied **6×** verbatim. Extract
  scoring primitives (`body_weight`, `cost_weight`, `handler_density`,
  `has_pitch_payoff`, `card_or_zero`); each intent fn becomes a weighted sum.

### [high] "Sequence of lowercased strings" parse duplicated 4× in loader.rs
- `card/loader.rs:327-341` (`colors`) + `:342-356` (`face`) + `:233-246`
  (`subtypes`) + `:247-260` (`colors`). Extract `fn read_lowercased_string_seq(
  parent, key, ctx)`; fold with `read_string_vec` (`:49-57`).

### [medium] Array-or-slot-map parsing duplicated (symbols vs colors)
- `loader.rs:758-806` vs `:66-132` (the comments even note they "mirror" each
  other). Extract `fn read_array_or_slot_map(t, key, field, normalize)`.

### [medium] `error::emit` / `emit_region` near-identical; same literal in trace.rs
- `error.rs:72-100` vs `:104-133` differ only by `region: None` vs `Some(...)`;
  the same ~13-field `Error { … }` recurs at `trace.rs:454-476` and `:499-520`.
  Make `emit` delegate to an inner `emit_inner(…, region: Option<String>)`; add a
  shared `Error` constructor. Also extract `trace_breadcrumb()` + `origin_at_us()`
  (the `try_borrow` boilerplate is identical at `trace.rs:445-453` vs `:489-497`).

### [medium] CardRegistry VM-sandbox setup + id lookups
- `card.rs:27-38` vs `:44-55` (7-line sandbox bring-up) → `fn new_sandboxed_vm()`;
  `replay.rs:24-32,73-80` re-implement `registry.get(id)` (`card.rs:68-70`) — call
  it instead.

### [low] `ScriptedOracle` / `RecordingOracle` `choose_*` boilerplate; main.rs counts
- `choice.rs:893-940` and `:974-1013` (trait-signature-limited; a macro could
  collapse); `main.rs:91-102` (display-only).

---

## ccg — CLI binaries & reporting

### [high] Deck-directory JSON loading reimplemented 7–8×
- `cli_balance_probe.rs:134-162` (`load_baselines`), `cli_evolve.rs:251-285`,
  `cli_prune_champions.rs:209-229` + `:232-270`, `cli_curate.rs:79-114`,
  `cli_matchup_evolved.rs:44-73`, `cli_champions_report.rs:300-315`,
  `cli_matchup_mcts.rs:231-242`. Same `read_dir → filter json → sort →
  EvolvedDeck::load → to_cards` + copy-pasted error strings.
- Promote `load_baselines` to a shared `decks.rs`: `fn load_deck_dir(registry,
  dir)` + `fn json_paths_sorted(dir)`.

### [high] `--opponent-ai`→`AiKind` resolution duplicated 5×
- `cli_balance_probe.rs:204-215`, `cli_evolve.rs:188-208`,
  `cli_curve_sample.rs:125-136`, `cli_curate.rs:127-138`,
  `cli_matchup_mcts.rs:167-190`. Extract `fn parse_opponent_ai(name, iters, c,
  seed) -> Result<AiKind>` (return `Err`, don't `exit(2)` — errors are sacred).
  See cross-cutting theme #1: unify with `wasm_ffi.rs` `parse_ai_kind`.

### [high] Live win-rate `evaluate` closure duplicated
- `cli_prune_champions.rs:319-345`, `cli_curate.rs:142-166`; cousins at
  `cli_champions_report.rs:343-371`, `cli_matchup_evolved.rs:91-106`. The
  one-seeded-game inner (`play_one`) already exists at `cli_matchup_mcts.rs:120-133`.
  Extract `fn live_winrate(...)` + `fn play_seeded(...)`.

### [high] Single-linkage Jaccard union-find clustering 3× (4 `find` copies)
- `cli_prune_champions.rs:282-315`, `cli_curate.rs:288-317`,
  `cli_champions_report.rs:225-255`. Extract `fn jaccard_clusters(sets,
  threshold) -> Vec<Vec<usize>>` into `sim::diversity` (which already exports
  `jaccard`).

### [high] GameStats aggregation + table emission twice (stdout vs HTML)
- `cli_matchup_evolved.rs:110-410` vs `:443-659`. ~250 lines computed twice
  (`fn avg` defined at `:121` and `:451`; top-cards BTreeMap byte-identical at
  `:294-328` vs `:565-589`; etc.). Compute once into a `MatchupAggregates` struct;
  feed both renderers.

### [medium] HTML report scaffolding repeated per report (5×)
- `cli_matchup_evolved.rs:661-676`, `cli_balance_probe.rs:417-459`,
  `cli_prune_champions.rs:102-141`, `champions_report.rs:153-170`,
  `evolve_report.rs:93-123`. Add `report_style::page(title, meta, body)` +
  `meta_row(pairs)`; move per-report inline `<style>` blobs into `report_style`.

### [medium] Champion presence + fitness-correlation computed twice
- `cli_champions_report.rs:100-135,173-219` vs `champions_report.rs:78-109,119-151`.
  Extract `fn card_presence_table(...)` + `fn fitness_correlation(...)`.

### [medium] Per-generation EA progress callback duplicated
- `cli_evolve.rs:353-377` vs `cli_balance_probe.rs:251-267`. Extract a
  `ProgressTracker` (holds `t_start`/`t_prev`/`prev_best`, `.tick(...) -> Line`).

### [medium] `variant_hero` reimplements `report_style` formatters
- `cli_balance_probe.rs:315-410` re-derives the color-class match,
  `format_cost`, kind/meta labels that already exist in `report_style.rs:311-419`.
  Make those `pub(crate)` and call them; extract `fn color_class(card) ->
  &'static str` (a 3rd copy lives in `mini_card`).

### [low] Repeated clap field decls; divergent heat-cell color fns
- `seed`/`baselines`/`games`/`opponent-uct-*` field decls (+ doc comments) across
  5 structs → `#[command(flatten)]` sub-structs. `cli_matchup_evolved.rs:444-449`
  (`rate_color`) vs `evolve_report.rs:60-66` (`cell_color`) — different formulas
  for the same intent → one `report_style::heat_color(t)`.

---

## ccg — WASM FFI & frontend

### [high] `#[no_mangle]` extern wrapper boilerplate (9×)
- `wasm_ffi.rs:853,873,926,958,990` (string-arg, ~12 lines each) +
  `:889,900,911,975` (no-arg, ~4 lines each). Two `macro_rules!`:
  `ffi_export_str!` / `ffi_export_noarg!`.

### [medium] AI-name→`AiKind` parsing 3× in wasm_ffi
- `wasm_ffi.rs:299-312`, `:487-498`, `:685-696`. `fn parse_ai_kind(name, seed)`
  — and unify with the CLI copies (cross-cutting #1).

### [medium] Trace/error envelope prologue+epilogue per `_impl`
- prologue at `:65-68,98-101,291-294,412-415,585-596,625-634`; epilogue at
  `:384-397,516-525,606-615,642-651`. `wrap_result_envelope` (`:41-52`) already
  abstracts the one-shot case — extend to a `with_ffi_envelope("label", |…|)`
  guard; only the `prompt` vs `result` key varies.

### [medium] JS marshaling + dispatch in workers; IndexedDB + Blob-download
- `tsot-worker.js:108-114` + `:116-122` (`callWasm`/`callWasmNoArgs`) → merge;
  9 hand-written dispatch arms (`:133-200`) → a `ROUTES` table. `js-bridge.js`
  five IndexedDB store-access helpers (`:118,128,143,153,163`) → `tsotTx(store,
  mode, fn)`; Blob-download dance (`:481-491` + `:780-788`) → `downloadBlob(...)`.

### [medium/low] Manually-synced Rust↔TS types — DRY *risk*
- `CardView`: `sim/snapshot.rs:53-76` vs `frontend-garden/src/tsot-card-types.ts:
  8-41` (the TS header already flags it — wire `ts-rs`). `symbols.ts:11-37`
  mirrors an out-of-repo Go file by hand, and the glyphs are re-typed in
  `main.ts:88`. Add a CI field-list check until codegen lands; have `main.ts`
  reference the `SegDef` constants.

### [low] JS error-envelope shape 3×; under-reused `el()` helper
- `js-bridge.js:340-350` vs `tsot-worker.js:59-69` + `:217-230` →
  `makeErrorEnvelope(...)`. `glyphs.ts:4-9` defines `el()` but `tsot-card.ts:
  50-92` and `js-bridge.js` open-code createElement ~20× — promote `el()`.

---

## roam

### [high] WebGL program builders repeat VAO/quad/index/uniform boilerplate (4–5×)
- `render_gl/tile.rs:101-158`, `flower.rs:153-222`, `card.rs:87-145`,
  `marker.rs:71-128`, + `line.rs:79-116`. Same `unit_quad`/index buffer/camera-
  uniform lookups. Add `build_unit_quad_vao(gl)`, `get_camera_uniforms(gl, prog)`,
  `create_buffer(gl, label)` to `helpers.rs`.

### [high] Per-frame "bind→upload→set camera→draw" repeated (4–5×)
- `render_gl/mod.rs:385-413,416-442,444-471,522-545,549-566`. Add
  `upload_stream(gl, target, scratch)` + a camera-uniform struct with
  `set_camera(...)`; flower/card instanced-draw tail → `draw_instanced_quads(...)`.

### [high] `publish_position` / `publish_pickup` structurally identical
- `net/state.rs:302-348` vs `:363-392`. Extract `fn publish_counted(topic, bytes,
  &PublishCounters)`.

### [high] Per-counter `fetch_add`/snapshot/JSON triplicated in perf.rs
- `perf.rs:40-64` (`note_tag_emit`), `:122-143` (`snapshot`), `:148-179`
  (`snapshot_json`), `:227-246` (test keys). 18 counters × ~3 restatements. A
  `macro_rules!` over `(field, json_key)` rows generates all three; tag routing →
  a `&[(&str, &AtomicU64)]` slice.

### [high] `NetEvent`→trace mapping duplicated and **already drifted**
- `net/state.rs:224-271` vs `wasm_ffi.rs:771-797`. Same tag strings + `format!`
  bodies; `state.rs` is **missing** the `net::sub_change` arm. Extract
  `fn net_event_to_trace(ev) -> Option<TraceEvent>`. **Correctness risk — do first.**

### [medium] Worker-provider FFI wrappers identical but for the verb (3×)
- `wasm_ffi.rs:670-688,691-709,712-730`. Macro `worker_provider_fn!(name, verb)`
  or a `with_provider(|p| …)` helper centralizing the feature-gate fallback.

### [medium] `fractal_2d`/`fractal_3d` + `value_noise_2d`/`_3d` share octave loop
- `teranos/noise.rs:88-103` vs `:105-127`; `:34-49` vs `:54-79`. Have `fractal`
  take a `sample` closure; extract the period-wrap helper.

### [medium] `flower_color_rgb`/`flower_core_rgb` discriminant→rgb→warn pattern
- `render_gl/flower.rs:224-246` vs `:248-264` (+ `core_edge_from_u8:266-273`,
  `mod.rs:506-515`). Generic `enum_rgb_or_magenta<E>(...)` or `TryFrom<u8>` on the
  teranos enums.

### [medium] JS error-shape extraction across all JS files
- `net-worker.js:72,113,176,221` (+ stack twin `43,52,114`), `identity.js:87`,
  `js-bridge.js:379,1570`. Add `errInfo(err) -> {message, stack}` + `postError(
  where, err)`.

### [low] Native NetworkProvider stubs (3–4×); `read_u32_le`/`read_i32_le`
- `net/rust_libp2p.rs:35-58`, `net/worker_bridge.rs:181-198`, test stubs
  `net/state.rs:469-488,677-700`; `render_gl/mod.rs:587-597`. Minor.

---

## rave, crates, universe

### [high] Two integration tests are ~2/3 identical relayer harness
- `crates/rave-positions-test/tests/positions_via_relayer.rs:10-79` vs
  `chat_via_relayer.rs:10-69` (`ClientBehaviour`, `build_client_swarm`,
  `pick_free_port` byte-identical); bodies `:87-233` vs `:78-221` differ only by
  topic/payload/asserts. ~180 lines × 2. The project convention ("a sibling test
  per gossipsub topic") makes this the load-bearing reuse surface — extract a
  `tests/common/mod.rs` with `spawn_relayer()`, `await_mesh()`, `await_message()`.

### [medium] `error::emit_region(... format!("{e:?}"))` boilerplate in rave (15×)
- `net_glue.rs:76,92,103,113,127,137,147,203,212,259`,
  `chat.rs:112,135,147,176,185`. Add `error::emit_err(sev, region, title,
  &impl Debug)` and/or a `res.or_emit("region","title")?` extension. (Fix lives in
  `rave/src/error.rs`; sacred-error deliberately leaves wrappers per-consumer.)

### [medium] rave `Cmd` send wrappers repeat `map_err` shape
- `net.rs:244-269` (`publish`/`subscribe`/`unsubscribe`) + `:306-327`. One
  `fn send_cmd(cmd, label)`.

### [low] Protocol/topic string literals hardcoded in both prod and tests
- `"/rave/1.0.0"` at `net.rs:165` + tests `:25`/`:27`; topic literals in
  `net_glue.rs:31`, `chat.rs:17`, + each test. Promote to shared `pub const`s so a
  rename can't desync production from its CI proof. (Swarm-builder bodies genuinely
  differ — wasm vs native transport — leave separate.)

### [low] rave/web `showErr`-guarded bridge install + JSON-parse repeated
- `identity-bridge.ts:28-60`, `chat-overlay.ts:55-62`, `screenshot.ts:8-29`,
  `error-bridge.ts:20-39`. Add `safeParse<T>(json, label)` + `guard(label, fn)`.

### Verified clean / out of scope
- `crates/tsot-card` (25-line newtype), `crates/sacred-error` (the canonical
  shared error; rave correctly `pub use`s it — no reimplementation found).
- **Cross-project parallels** (rave↔universe drawer/scaffold, roam↔rave identity,
  build_info ×3) are now fully analyzed in the dedicated
  [cross-project section](#cross-project-analysis) below.

---

## Cross-project analysis

The subprojects do **not** share a workspace (no root `Cargo.toml`; four separate
`Cargo.lock` files), and the root `CLAUDE.md` frames them as independent. That
framing holds for *domain* code — and the analysis confirms it (roam's "card"
files are an unrelated render quad + worldgen index; ccg's engine types live only
in ccg; the two trace buses share a shape but no code, by design). But it does
**not** hold for *generic infrastructure*: observability scaffold, identity
crypto, the error wire-shape, and build/CI config are duplicated across project
lines, and several copies have already drifted. The team has already set the
precedent that genuinely-shared infra gets a crate (`crates/sacred-error` is
consumed by ccg+roam+rave precisely so the wire shape can't drift) — these
findings extend that same logic.

Each finding is tagged **SHOULD-SHARE** (extract it), **INTENTIONAL-PARALLEL**
(same shape, divergence is correct — leave it), **SHALLOW** (too small to be
worth it), or **FALSE-POSITIVE** (coincidental, not real duplication).

### Cross-project opportunities, ranked

| # | What | Projects | Lines | Verdict |
|---|------|----------|-------|---------|
| C1 | Bevy drawer + observability + run-scaffold + panic-hooks | rave, universe | ~370–400 | SHOULD-SHARE |
| C2 | Identity keypair load/mint/encode + IDB persistence | roam, rave | ~90 + did:key ~110 | SHOULD-SHARE |
| C3 | GitHub Actions deploy workflow (wasm→S3→CloudFront) | rave, universe, roam | ~75% of ~90 ×3 | SHOULD-SHARE |
| C4 | `flake.nix` boilerplate + `flake.lock` (3 drifted pins) | root, roam, rave, universe | ~21 ×4 + 4 locks | SHOULD-SHARE |
| C5 | sacred-error 14-field `Error{}` literal / panic helper | ccg, roam, rave | 6 sites | SHOULD-SHARE |
| C6 | Typed-error wire shape defined independently | ccg, roam, rave | ~10 fields ×3 | SHOULD-SHARE |
| C7 | libp2p `=0.56.0` pin set hand-synced | roam, rave, relayers, tests | ~10 ×5 | SHOULD-SHARE |
| C8 | Bevy `[profile.*]` + feature blocks | rave, universe | ~25 ×2 | SHOULD-SHARE |
| C9 | JS error-shape extraction (`normalizeError`) | ccg, roam (bridges+workers) | ~3 ×~20 | SHOULD-SHARE |
| C10 | `tsot-card` crate claims dual-consumer, only roam uses it | (ccg), roam | n/a | latent |

### [high] C1 — Bevy drawer/observability/run-scaffold (rave ↔ universe)
- `rave/src/drawer.rs:1-266` + `rave/src/observability.rs:1-160` vs
  `universe/src/lib.rs:437-753`; boot scaffold `rave/src/lib.rs:69-144` vs
  `universe/src/lib.rs:54-127`.
- Near-verbatim: `CaptureLayer`/`MessageVisitor`/`install_capture_layer`,
  `Severity`/`ErrorEntry`/`ErrorLog`+`emit`, `PANIC_QUEUE`/`LOG_QUEUE` +
  `drain_panics`/`drain_logs`, `update_fps`/`update_error_list`/
  `toggle_log_drawer`/`update_clock`, the `setup_drawer` node tree, and the
  `#[wasm_bindgen(start)] run()` boot (pre-App panic hook → `App::new()` +
  identical `ClearColor` → `DefaultPlugins.set(WindowPlugin/AssetPlugin/LogPlugin)`
  → wrap-after-LogPlugin second hook → diagnostics + Update system tuple).
- **Already drifting:** rave's `ErrorLog::emit` has an `ERROR_LOG_CAP=50` ring-
  buffer trim; universe's `ErrorLog` grows unbounded. The subtle two-stage panic-
  hook ordering is load-bearing and copied by hand.
- **Verdict: SHOULD-SHARE.** The independence axiom is about roam-vs-ccg
  *architecture*; it does not forbid two genuine Bevy apps from sharing a Bevy
  plugin. Extract `crates/bevy-observability` exposing an `ObservabilityPlugin`
  (owns `ErrorLog`/`Severity`/`CaptureLayer`/queues/drain+update+toggle systems +
  a `setup_drawer` parameterized by app-name/watermark/toggle-keys) and a boot
  helper that installs both panic hooks + the LogPlugin custom layer given a JS
  error-sink namespace. Keep **roam out** — it's egui/WebGL, not Bevy (C-low).

### [high] C2 — Identity keypair handling (roam ↔ rave)
- Rust: `roam/src/identity.rs:166-191` vs `rave/src/identity.rs:44-68`
  (`load_or_generate_keypair` + `generate_identity_protobuf` are **byte-identical**,
  including the `NetError::ProviderInternal` error text and the "no silent
  ephemeral fallback" rationale comment). JS/TS: `roam/assets/src/identity.js:25-65`
  vs `rave/web/src/identity-bridge.ts:14-59` (same IDB open→get→mint-Ed25519-
  protobuf→put flow; only db/store/key names differ).
- The W3C did:key/PeerId multicodec codec (`roam/src/identity.rs:45-152`, ~110
  lines + tests) lives only in roam, but `rave/src/identity.rs:5-6` explicitly
  defers the same did:key surfacing — i.e. it's the next thing to be copy-pasted.
- **Verdict: SHOULD-SHARE.** Pure generic p2p-identity glue, not domain logic.
  Extract `crates/p2p-identity` (keypair load/mint/encode + the did:key codec, so
  rave pulls it in instead of re-deriving) and a shared `idb-identity.js`
  parameterized by `{dbName, store, key}`.

### [high] C3 — Deploy workflows (rave / universe / roam)
- `.github/workflows/deploy-rave.yml` vs `deploy-universe.yml` vs `deploy-roam.yml`.
- ~75% shared: `checkout` → `install-nix-action` (identical config) → `rust-cache`
  → `nix develop -c make wasm` → verify-dist → `configure-aws-credentials` (OIDC)
  → `aws s3 sync --delete` + `aws s3 cp index.html` (identical cache-control) →
  CloudFront create-invalidation + wait. Only bucket/distribution/role values and
  per-project verify-globs differ; roam adds `bun install`, rave adds the
  integration-test steps.
- **Verdict: SHOULD-SHARE.** Largest verbatim infra duplication in the repo.
  Convert to a reusable `workflow_call` (inputs: bucket/distribution/role/workdir)
  or a composite action for the build→verify→sync→invalidate tail; per-project
  extras become conditional steps.

### [high] C4 — flake.nix / flake.lock (root, roam, rave, universe)
- Four `flake.nix` with **byte-identical** `inputs` blocks (nixpkgs nixos-unstable
  + flake-utils + rust-overlay) and `eachDefaultSystem`/overlay scaffolding;
  roam/rave/universe share the same `rust-bin.stable.latest … targets =
  ["wasm32-unknown-unknown"]`. Four `flake.lock` pin the same input graph but have
  **already drifted to 3 distinct toolchain pins** (root & roam share nixpkgs but
  differ on rust-overlay; rave & universe share a newer nixpkgs).
- **Verdict: SHOULD-SHARE.** One root `flake.nix` exposing per-project devShells
  (`nix develop .#roam`) collapses 4 lockfiles → 1 and stops the silent toolchain
  divergence. DevShell *contents* still differ per project — "one flake, multiple
  shells," not one shell. (ccg has no flake of its own; the root flake is ccg's.)

### [medium] C5 — sacred-error `Error{}` literal + panic helper (ccg, roam, rave)
- The 14-field `Error { … }` struct literal (9 fixed `None`/`vec![]`/`false`
  defaults) is hand-built at 6 sites: `ccg/src/error.rs:72-133` (emit/emit_region)
  + `ccg/src/trace.rs:454-476,499-520` (snapshot_panic/emit_error),
  `roam/src/error.rs:53-115`, `rave/src/error.rs:50-78`. Only `at:` and
  region/surface vary. Panic-payload downcast (`payload_to_string`) is identical
  in `ccg/src/trace.rs:437-443` and `roam/src/trace.rs:202-210`.
- **Verdict: SHOULD-SHARE (the constructor only).** Add `Error::new(severity,
  context, title, why)` + `.at()/.region()` setters and a `payload_to_string(&dyn
  Any)` helper **to the existing `sacred-error` crate**. Removes the lockstep-edit
  hazard (add a field → 6 hand-edits) without any shared runtime state. The
  thread-local error *bus* (`push`/`drain`/`next_id`, ~25 lines ×3) stays
  per-project — `sacred-error/src/lib.rs:8-28` documents why (thread-locals can't
  be runtime-parameterized; id-prefix + clock differ). That's **INTENTIONAL**.

### [medium] C6 — Typed-error wire shape (ccg, roam, rave)
- The sacred-error record `{id, severity, context:{surface,region,anchor}, title,
  why, trace, at}` crossing wasm→JS is described independently three times:
  `rave/web/src/error-bridge.ts:9-18` (`TypedError` interface),
  `roam/assets/src/js-bridge.js:380-391`, `ccg/assets/js-bridge.js:582-599` (Elm).
  ccg internally carries *three* competing vocabularies (legacy
  `{message,recent_trace}`, unified `{why,trace}`, Elm-typed) — the exact drift
  the "errors are sacred" axiom exists to prevent.
- **Verdict: SHOULD-SHARE.** Pin the wire shape once and generate/share a `.d.ts`
  so a field rename trips a type error in all three consumers. (Pairs naturally
  with C5: one Rust source of truth, one TS declaration.)

### [medium] C7 — libp2p `=0.56.0` pin set (roam, rave, relayers, tests)
- `libp2p = "=0.56.0"` + partner pins (`tokio=1.47.1`, `futures=0.3.32`,
  `futures-timer=3.0.4`) + the browser/native feature shapes are replicated across
  `roam/Cargo.toml`, `roam/relayers/Cargo.toml`, `rave/Cargo.toml`,
  `crates/rave-positions-test/Cargo.toml` — with comments that literally say
  "Mirrors roam/Cargo.toml's pinned versions." Hand-sync is exactly what
  `[workspace.dependencies]` removes.
- **Verdict: SHOULD-SHARE** via a narrow workspace over the libp2p-using crates;
  each keeps only its feature selection (`workspace = true`).

### [medium] C8 — Bevy `[profile.*]` + features (rave, universe)
- Identical profile stack (`dev.package."*" opt-level=3`, `release opt-level="s"
  lto codegen-units=1 strip`), `[features] dev=["bevy/dynamic_linking"]`, the same
  13 Bevy features, and the same wasm dep pins (`js-sys=0.3.98`,
  `wasm-bindgen=0.2.121`) in `rave/Cargo.toml` vs `universe/Cargo.toml`.
- **Verdict: SHOULD-SHARE** via a rave+universe workspace inheriting `[profile.*]`
  + `[workspace.dependencies]` for bevy/serde/wasm-bindgen. (ccg/roam have
  deliberately different profiles — `panic="abort"`, opt-level "z" — so this is a
  2-crate workspace, not repo-wide. A single monolithic workspace is the **wrong**
  shape: ccg is emscripten + build-std, incompatible toolchain.)

### [medium] C9 — JS error-shape extraction (ccg, roam)
- `x && x.message ? x.message : String(x)` (+ the `.stack` twin) is inlined ~5×
  in each of `ccg/assets/js-bridge.js`, `roam/assets/src/js-bridge.js`, and both
  workers (`tsot-worker.js`, `net-worker.js`). No shared `normalizeError` helper.
- **Verdict: SHOULD-SHARE.** A single `js-glue/error.js` `normalizeError(e) ->
  {message, stack, name}` (+ an `installGlobalErrorCapture(sink)` for the
  duplicated `window.onerror`/`unhandledrejection` wiring) imported by both
  bridges and both workers.

### [low / latent] C10 — `tsot-card` is single-consumer despite claiming otherwise
- `crates/tsot-card/Cargo.toml:7` + `lib.rs:1-11` say "consumed by `tsot` and
  `roam`," but only `roam/Cargo.toml:22` depends on it; ccg — the *authority* for
  the card slug that `CardId(String)` wraps — never imports it (`ccg/src/card.rs:560`
  stores `id: String` bare). The roam→ccg engine integration ("v0.5 resolves PvP
  encounters") is not yet wired in code.
- **Verdict: latent / deferred.** Correct to keep separate until the integration
  lands; at that point ccg should adopt `tsot_card::CardId` on `Card.id` so the
  seam shares one definition. For now, fix the crate's doc/description to stop
  asserting a sharing relationship that doesn't exist. The `{ok, result, log,
  trace, errors}` envelope roam reads from ccg's wasm
  (`ccg/src/wasm_ffi.rs:41-52` → `roam/assets/src/js-bridge.js:734-744`) is the
  one **sanctioned integration seam** — harden it with a shared `.d.ts`, don't
  collapse it.

### Confirmed INTENTIONAL-PARALLEL / FALSE-POSITIVE (no action)
- **trace.rs (ccg ~1248 lines vs roam ~278):** same shape, ~0 shared code —
  domain-specific event vocabularies, serde vs deliberately serde-free FFI, drain
  vs bounded-ring buffers. roam's CLAUDE.md mandates the mirrored *pattern*.
- **wasm_ffi.rs structure (ccg vs roam):** emscripten `extern "C"`/`CStr`/JSON
  envelope vs `wasm-bindgen` native marshalling — converging them would violate
  roam's "no stringly-typed FFI" axiom. (The ~10 repeated shims are an *internal*
  ccg macro opportunity, already listed in the ccg FFI section.)
- **roam "card" files:** `render_gl/card.rs` (a rendered quad) and
  `teranos/card.rs` (worldgen index) share only the filename with ccg's `Card`
  schema — FALSE-POSITIVE.
- **build_info.rs ×3, error.rs thread-local bus ×3, `.cargo/config.toml`,
  Caddyfile, tsconfig:** SHALLOW — small boilerplate around deliberately divergent
  content; would only fall out for free under a root workspace/flake, not worth
  extracting on their own.
- **roam `perf.rs`:** no ccg analogue (ccg `sim/instrument.rs` is a hang
  watchdog, different purpose) — not duplication.

### Side observation (not duplication, flagged for safety)
`universe/Cargo.toml` pins `getrandom 0.3 features=["wasm_js"]` but has **no**
`.cargo/config.toml` setting `--cfg getrandom_backend="wasm_js"` (roam and rave
both do). This looks like a latent build inconsistency, not a DRY issue — worth a
separate check.

### Cross-project verdict
Domain independence is real and well-kept; **infrastructure** independence is not,
and three copies have already drifted (universe's uncapped `ErrorLog`, three
toolchain pins, ccg's three error vocabularies). The high-value, low-risk moves —
each consistent with the existing `sacred-error` precedent — are: a
`crates/bevy-observability` plugin (rave+universe, ~370 lines), a
`crates/p2p-identity` crate (roam+rave, ~90+110 lines), a single root `flake.nix`
(4 lockfiles → 1), and a reusable deploy workflow (3 → 1). On the Cargo side, do
**not** make one repo-wide workspace — use two narrow ones (`crates/`, and
rave+universe) plus `[workspace.dependencies]` for the hand-synced libp2p/Bevy
pins. Everything else is either intentional parallelism driven by genuinely
different runtimes/domains, or boilerplate too shallow to be worth extracting.
