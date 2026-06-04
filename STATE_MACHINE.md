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

- [ ] **S3: Parity test vs run_game_continue.**
  Heuristic-vs-Heuristic game on a fixed seed runs to the same winner /
  turn count / stats via `StepEngine::run_to_end()` and via the existing
  `run_game_continue`. Byte-identical or flagged divergence.
  (Test green: `step_engine_parity_vs_run_game_continue` byte-identical
  on seed `0xBEEF`. Two ordering subtleties surfaced: (1) journal must
  open AFTER `build_pattern_b_choices` so
  `rig_creature_free_haste`'s cost-clear stays outside the preview-
  rollback envelope; (2) each phase advance constructs a fresh
  `RandomOracle` from `rng.gen()` rather than reusing the persistent
  oracle. Template filter excludes cards with `activated` abilities —
  activation passes are S9 scope. Awaiting confirmation before marking
  complete.)

## Phase 2 — human decision points (unblocks D4)

- [ ] **S4: PickCard human yield.**
  `PatternBPick` Human arm returns `NeedHuman(PickCard{…})` on
  `pending=None`; consumes Pass / PlayCard on next step. Resolve phase
  fires `play_card` once chosen.

- [ ] **S5: PickAttackers + PickBlocks human yields.**
  Same pattern for combat. Defender's `Human` AI yields PickBlocks;
  attacker's yields PickAttackers. Confirm phases call into the existing
  `declare_attacker` / `declare_blocker` engine APIs.

- [ ] **S6: tsot_start_game / tsot_apply_action use StepEngine.**
  Wasm path unblocked. Native D2/D3 tests rewired to step through
  StepEngine instead of thread+channel. Delete the thread spawn path.

## Phase 3 — ChoiceOracle round-trips

- [ ] **S7: ChooseCard yields.**
  Hand-payment slots inside `build_pattern_b_choices` and target picks
  inside Lua handlers each become inner cursors. Resume threads a
  selected iid back through the oracle's return.

- [ ] **S8: Confirm / ChoosePlayer / ChooseInt yields.**
  Remaining ChoiceOracle methods. May-prompts, player picks, X-cost
  values. Same inner-cursor pattern; smaller surfaces than S7.

## Phase 4 — activations + Main2

- [ ] **S9: Activation pass cursors.**
  `run_activation_pass` rewritten as cursor states. Human-side activation
  decisions yield `PickCard` with `activations` field populated.

- [ ] **S10: Main2 prompt loop.**
  Post-combat human main phase folds in as a cursor variant. Same Play /
  Activate / Pass action set as Pattern B.

- [ ] **S11: Edge cases (rig + suicide rollback + response windows).**
  `rig_creature_free_haste`, suicide-rollback gating, instant response
  window timing. All the special-casing in run_game_continue gets
  ported. Suite regression check.

## Phase 5 — migration + cleanup

- [ ] **S12: Delete run_game_continue, migrate callers.**
  `sim::mcts::pick_play`, `sim::uct::pick_play_uct`, `sim::fitness`
  all switch to `StepEngine::run_to_end`. Same journal behavior, same
  stats output, same determinism.

- [ ] **S13: Full suite regression + perf check.**
  All existing tests pass. Heuristic-vs-Heuristic 100-game wall time
  within 10% of the pre-refactor baseline. UCT/MCTS strength
  measurements within noise.

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
