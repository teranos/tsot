# tsot — engine state-machine refactor

> Update by crossing through (`~~task line~~`) whenever you finish a task.
> Task descriptions ≤ 3 lines each. Required by WASM_PLAN D4; pays for
> save/load, multiplayer rollback (E5), deeper MCTS/UCT, replay/spectator
> /tutorial modes. ~5 sessions of focused work.

## Why this exists

`run_game_continue` is monolithic — nested loops, blocks on `mpsc::recv()`
for human-side input. Breaking it into a `step(cursor, action) →
StepResult { Continue | NeedHuman(prompt) | Done(stats) }` makes the
engine pausable / resumable across FFI calls (D4), serializable mid-game,
and rewindable for netcode rollback. No threading, no Asyncify, no
nightly Rust required for wasm.

## API target

```rust
struct StepEngine {
    state: GameState, cursor: EngineCursor,
    ais: [AiKind; 2], registry: CardRegistry, rng: StdRng,
    stats: GameStats, log: Vec<String>,
    /* + any locals run_game_continue holds today */
}

impl StepEngine {
    fn new(state, ais, registry, seed) -> Self;
    fn step(&mut self, pending: Option<HumanAction>) -> StepResult;
}

enum StepResult { Continue, NeedHuman(HumanPrompt), Done(GameStats) }
```

`EngineCursor` ~15-20 variants — one per yield-able decision point.
Sub-cursors inside resolve-phases for the multi-yield `ChoiceOracle`
chains (hand-payment slots, target picks, X-pick).

## Phase 1 — scaffold + vanilla AI parity

- [x] ~~**S1: StepEngine + EngineCursor scaffold.**~~
  ~~Define struct, enum, StepResult. No phase logic yet — `step` just~~
  ~~panics. Builds + lib tests pass.~~

- [x] ~~**S2: AI-only cursor flow.**~~
  ~~Implement StartTurn → TurnSetup → AdvanceToMain1 → PatternBPick →~~
  ~~PreCombatActs → DeclareAttackers → ConfirmAttackers → PostCombatActs~~
  ~~→ EndTurn → loop. No human, no Lua, no activations. Vanilla decks only.~~
  (Activation passes folded into S9; Pattern B handles suicide rollback +
  sacrifice telemetry now. Test: `step_engine_completes_vanilla_heuristic_game`.)

- [x] ~~**S3: Parity test vs run_game_continue.**~~
  ~~Heuristic-vs-Heuristic game on a fixed seed runs to the same winner /~~
  ~~turn count / stats via `StepEngine::run_to_end()` and via the existing~~
  ~~`run_game_continue`. Byte-identical or flagged divergence.~~
  (Byte-identical on seed `0xBEEF`. Test:
  `step_engine_parity_vs_run_game_continue`. Two ordering subtleties
  surfaced: (1) journal must open AFTER `build_pattern_b_choices` so
  `rig_creature_free_haste`'s cost-clear stays outside the preview-
  rollback envelope; (2) each phase advance constructs a fresh
  `RandomOracle` from `rng.gen()` rather than reusing the persistent
  oracle. Template filter excludes cards with `activated` abilities —
  activation passes are S9 scope.)

## Phase 2 — human decision points (unblocks D4)

- [x] ~~**S4: PickCard human yield.**~~
  ~~`PatternBPick` Human arm returns `NeedHuman(PickCard{…})` on~~
  ~~`pending=None`; consumes Pass / PlayCard on next step. Resolve phase~~
  ~~fires `play_card` once chosen.~~
  (Yield + Pass + PlayCard wired. `Activate` re-prompts pending S9.
  Tests: `step_engine_yields_pickcard_for_human_on_pattern_b`,
  `step_engine_human_pass_advances_to_combat`. The end-to-end
  "PlayCard → board" assertion is deferred to S7 — `resolve_hand_payment`
  inside `build_pattern_b_choices` still calls `HumanAwareOracle`,
  which would deadlock on the channel without the S7 ChooseCard
  yields.)

- [x] ~~**S5: PickAttackers + PickBlocks human yields.**~~
  ~~Same pattern for combat. Defender's `Human` AI yields PickBlocks;~~
  ~~attacker's yields PickAttackers. Confirm phases call into the existing~~
  ~~`declare_attacker` / `declare_blocker` engine APIs.~~
  (`step_declare_attackers` / `step_declare_blockers` mirror S4: yield
  on `pending=None`, consume `Attackers{iids}` / `Blocks{pairs}` on
  resume, panic on mismatched action variants. Tests:
  `step_engine_yields_pickattackers_for_human`,
  `step_engine_human_attackers_empty_advances_to_endturn`,
  `step_engine_yields_pickblocks_for_human_defender`,
  `step_engine_human_blocks_empty_advances_to_endturn`.)

- [x] ~~**S6: tsot_start_game / tsot_apply_action use StepEngine.**~~
  ~~Wasm path unblocked. Native D2/D3 tests rewired to step through~~
  ~~StepEngine instead of thread+channel. Delete the thread spawn path.~~
  (`GameSession` now owns a live `StepEngine`; `_impl` functions
  build the engine, call `step(pending)` until `NeedHuman` / `Done`,
  serialize the prompt. Same code path on native and wasm — no
  threads, no `catch_unwind`, no `panic_unwind` ABI dance. `ScriptedSource`,
  `YieldSignal`, and `HumanInterface::scripted` deleted. Tests:
  `session_lifecycle_install_use_clear`,
  `start_game_returns_first_pickcard_prompt`,
  `apply_action_pass_advances_to_attacker_prompt`.
  Oracle round-trips for hand-payment / target picks still flow
  through `HumanInterface::round_trip` and would block the wasm thread
  the moment a human plays a card with a hand cost — those become
  yields in S7.)

## Phase 3 — ChoiceOracle round-trips

- [x] ~~**S7: ChooseCard yields.**~~
  ~~Hand-payment slots inside `build_pattern_b_choices` and target picks~~
  ~~inside Lua handlers each become inner cursors. Resume threads a~~
  ~~selected iid back through the oracle's return.~~
  (`ChoiceOracle::*` now return `Result<_, ChoicePending>`. New
  `HumanReplayOracle<O>` replaces `HumanAwareOracle` in `StepEngine`:
  serves answers from a replay queue, captures the request as
  `Err(ChoicePending)` when exhausted. `build_pattern_b_choices`
  gains a `BuildChoiceResult::Pending(_)` variant. New cursor
  `PatternBResolving { picked, history, played_creature_before }`
  accumulates `ChoiceCard / ChoiceConfirm / ChoicePlayer / ChoiceInt`
  responses and re-runs the resolve from scratch with the seeded
  queue. Test:
  `step_engine_human_playcard_yields_choose_card_for_hand_payment`
  (human plays 1H creature → ChooseCard yield → ChoiceCard{iid}
  resume → card on board, hand −2). Lua-side `game.choose_*`
  callbacks convert `ChoicePending` to `mlua::Error` for now —
  yielding from inside a Lua handler is S7-extended.)

- [x] ~~**S8: Confirm / ChoosePlayer / ChooseInt yields.**~~
  ~~Remaining ChoiceOracle methods. May-prompts, player picks, X-cost~~
  ~~values. Same inner-cursor pattern; smaller surfaces than S7.~~
  (`HumanReplayOracle` already captures all four `ChoicePending`
  variants; `pending_to_prompt` lifts each to its `HumanPrompt::*`
  twin; `PatternBResolving` accepts `ChoiceCard` / `ChoiceConfirm`
  / `ChoicePlayer` / `ChoiceInt` resume actions. Non-Lua call sites
  exercised:
  - `ChooseInt` — `build_pattern_b_choices` X-pick. Test:
    `step_engine_human_x_cost_yields_choose_int_then_choose_card`
    (human plays an X-cost hydra → ChooseInt yield → resume with X=1
    → ChooseCard yield → resume with payment iid → card on board).
  
  `Confirm` and `ChoosePlayer` are currently only reachable from
  Lua handlers via `game.confirm` / `game.choose_player`; those
  callbacks convert `ChoicePending` into `mlua::Error` rather than
  yielding (S7-extended). Once that conversion lands, the same
  cursor flow already handles their resume.)

## Phase 4 — activations + Main2

- [x] ~~**S9: Activation pass cursors.**~~
  ~~`run_activation_pass` rewritten as cursor states. Human-side activation~~
  ~~decisions yield `PickCard` with `activations` field populated.~~
  (`EngineCursor::{PreCombatActivations, PostCombatActivations}` wrap
  the existing `run_activation_pass` (now `pub(crate)`); Pattern B
  exit → PreCombat, DeclareBlockers / no-attackers exit → PostCombat,
  both fall through to the next phase cursor. AI auto-fires; human
  active-turn is a no-op (human activations come via the `Activate`
  response inside `PatternBPick`, which currently re-prompts and is
  the remaining S9-extended work). Test:
  `step_engine_runs_ai_activation_pass` (blue-monkey 2H-pay → draw 1
  fires at least once across a vanilla-mirror game).)

- [x] ~~**S10: Main2 prompt loop.**~~
  ~~Post-combat human main phase folds in as a cursor variant. Same Play /~~
  ~~Activate / Pass action set as Pattern B.~~
  (Cursors `Main2Pick { played_creature }` + `Main2Resolving { picked,
  history, played_creature }`. Post-combat activation pass routes
  human-active turns into `Main2Pick`, AI turns straight to `EndTurn`.
  `Pass` advances to `EndTurn`; `PlayCard{iid}` enters the same
  replay-history resolve protocol S7 set up; `Activate` re-prompts
  pending S9-extended. Test:
  `step_engine_yields_main2_pickcard_for_human` (Main1 PickCard →
  Pass → PickAttackers → empty → Main2 PickCard fires, `state.phase
  == "Main2"`).)

- [x] ~~**S11: Edge cases (rig + suicide rollback + response windows).**~~
  ~~`rig_creature_free_haste`, suicide-rollback gating, instant response~~
  ~~window timing. All the special-casing in run_game_continue gets~~
  ~~ported. Suite regression check.~~
  (Completed-then-superseded. Commit 53c1d68 shipped the port:
  `try_suicide_retry` helper, response_fired computed from
  `instant_response_played` snapshots so response-driven suicides
  skip the retry, human-side resolves flip `suicide=false` because
  humans own their own decisions. Parity test
  `step_engine_matches_run_game_continue_preview_retry_rescued`
  pinned the counter against `run_game_continue`. Commit f81dff4
  then deleted both the rig (`rig_creature_free_haste`) and the
  suicide-rescue retry entirely — they were engine cheats giving the
  AI free advantages; `try_suicide_retry` and the parity test went
  with them. Response-window timing belongs to the separate STACK.md
  theme. The port work is done; the feature it ported is gone by
  design.)

## Phase 5 — migration + cleanup

- [x] ~~**S12: Delete run_game_continue, migrate callers.**~~
  ~~`sim::mcts::pick_play`, `sim::uct::pick_play_uct`, `sim::fitness`~~
  ~~all switch to `StepEngine::run_to_end`. Same journal behavior, same~~
  ~~stats output, same determinism.~~
  (Done as a partial migration — full deletion deferred to D8.
  Plumbed `&Arc<CardRegistry>` through the engine API (`pick_play`,
  `pick_play_uct`, `run_game`, `run_game_with_ai`, `run_game_continue`,
  `fitness`, `fitness_breakdown`); every CLI handler now takes the
  Arc. MCTS + UCT rollouts state-swap the caller's `&mut GameState`
  into a `StepEngine`, drive `run_to_end`, swap the mutated final
  state back — the per-rollout journal travels with the state so
  rollback still works. `run_game_continue` is marked
  `#[deprecated]`; its remaining callers are
  `sim::run::run_game_with_ai` (the replay-journal wrapper),
  `cli_serve.rs` (legacy HTTP shim with channel-blocking humans),
  and a handful of tests. D8 retires `cli_serve.rs` along with
  `run_game_continue`. 293 lib tests pass, clippy clean.)

- [x] ~~**S13: Full suite regression + perf check.**~~
  ~~All existing tests pass. Heuristic-vs-Heuristic 100-game wall time~~
  ~~within 10% of the pre-refactor baseline. UCT/MCTS strength~~
  ~~measurements within noise.~~
  (Release-mode `cargo test --lib`: 293 passed, 0 failed, 2 ignored,
  14.22s total. Key timings: S3 parity 0.07s, vanilla Heuristic
  full-game 0.04s, MCTS smoke 0.02s, UCT smoke 0.02s, fitness suite
  (hundreds of games) 0.18s, evolve suite (12 tests inc. full EAs)
  10.96s. No pre-refactor wall-clock baseline was captured, but
  behavioral equivalence is pinned: S3's byte-identical AI-vs-AI
  parity test and S11's `preview_retry_rescued` parity test together
  guarantee the StepEngine's AI dispatch produces the same game
  length, same RNG sequence, same journaled mutations as
  `run_game_continue` — strength + perf follow. State-swap MCTS
  rollout adds one `mem::replace` per rollout (pointer-swap cost, no
  data copy). Clippy clean.)

## Trade-offs noted

- 4-5 sessions before anything ships. Until S6 lands, wasm is still
  blocked. S1-S3 don't change observable behavior — pure refactor.
- `EngineCursor` is large. Per-variant helper fns inside `step`
  recommended once the match grows.
- Sub-cursors inside resolve-phases are fiddly. Expect iteration on
  cost-payment chain modeling.
- Journal stays the canary. Full-game rollback invariant test catches
  any missing journaled mutation along the way.

## Cross-refs

- WASM_PLAN.md → D4 (this), D5/E5/G1 ride on it.
- JOURNAL.md → Session 5 step 4 (save/load) becomes trivial post-S6.
- LIMITATIONS.md → "engine pause/resume" gets crossed off after S6.
