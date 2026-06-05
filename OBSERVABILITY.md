# tsot — engine observability

> Update by crossing through (`~~task line~~`) whenever you finish a task.
> Task descriptions ≤ 3 lines each. Goal: the engine narrates every internal
> decision, the wasm UI surfaces it without devtools, no future debugging
> session requires "I think it's X" hypotheses without measurement. ~7
> sessions of focused work.

## Why this exists

Today the engine is opaque from outside. `engine.log: Vec<String>` collects
free-text summaries at a handful of sites in `step/`. Everything else —
cursor transitions, state mutations, oracle Q&A, Lua handler fires, AI
candidate scoring, combat damage assignment, timing — lives in memory and
disappears the moment a step completes. The wasm UI sees only the summary
log; native CLI runs print to stderr but with the same coverage gap.

Outcome of that opacity: every debugging session devolves into me guessing
("must be UCT") because the data isn't visible. The fix is to make the
engine a *narrating* subsystem — every internal site emits one structured
`TraceEvent`, the bus surfaces them all, the UI renders them with filters
+ timestamps. From then on, "why is X happening" is answered by reading,
not asking.

## API target

```rust
pub enum TraceEvent {
    Step { duration_us: u64, from: EngineCursor, to: EngineCursor, result: StepResultTag },
    Cursor { from: EngineCursor, to: EngineCursor },
    Phase { from: Phase, to: Phase, turn: u32 },
    Mutation(JournalEntrySummary),        // every journal push
    Count { key: String, player: PlayerId, before: u32, after: u32 },
    Oracle { call: OracleCall, asker: Option<PlayerId>, answer: OracleAnswer, duration_us: u64 },
    Play { iid: InstanceId, outcome: PlayOutcome, cost_paid: PaidCostSummary, duration_us: u64 },
    Handler { event: EventName, source: InstanceId, partner: Option<InstanceId>, duration_us: u64, error: Option<String> },
    AiPick { ai: AiKindTag, candidates: Vec<CandidateScore>, chosen: Option<InstanceId>, search: AiSearchBreakdown },
    Combat(CombatEvent),                  // declare, damage, death, mill
    Preview { event: PreviewEvent },      // opened, committed, rolled_back
    Winner { who: PlayerId, cause: WinnerCause },
    Ffi { span: FfiSpan, duration_us: u64 },
}

pub struct StepEngine {
    /* … existing fields … */
    pub trace: Vec<TraceEvent>,           // drained per yield by the FFI
}
```

`trace` is the single source of truth for "what did the engine just do."
The wasm FFI envelope becomes `{ prompt, log, trace }` — `log` stays for
human-friendly summaries, `trace` carries the structured stream. UI renders
both side by side.

Architectural commitments:
- Every new engine site that mutates state or makes a decision emits at
  least one `TraceEvent`. Reviewed for in PRs. No exceptions.
- Trace events are structured, not strings. Strings rot; enums type-check.
- Trace ordering matches execution ordering (push order = chronological).
- Trace events serialize via serde so they travel across the FFI cheaply
  and can be persisted to disk / localStorage.

## Phase 1 — the bus + core engine narration

The infrastructure shipping alone is useless. Phase 1 ships the bus + the
instrumentation that covers ~80% of "what is the engine doing" — cursor
transitions, journal mutations, oracle Q&A, action_counts diffs, play
outcomes, winner-set events, per-step timing.

- [x] ~~**O1: TraceEvent enum + emission bus.**~~
  ~~Define the enum, add `trace: Vec<TraceEvent>` to `StepEngine`,~~
  ~~drain in `drive_to_next_yield` into the envelope, wire JS-side to~~
  ~~receive `env.trace` alongside `env.log`. No event sites instrumented~~
  ~~yet — Phase 1's foundation.~~
  (Bus in `src/trace.rs`: thread-local buffer, `enable/is_enabled/push/drain/now_us` helpers, `TraceEvent` tagged enum with all Phase-1+ variants pre-defined. Stored as thread-local rather than `StepEngine` field — sites without engine access (`Journal::push`, `state.bump_action`, oracle methods) push directly. Native callers default to disabled; wasm enables before each FFI call. 8 bus contract tests. Envelope wiring + JS-side receive deferred to O5 once events are emitted.)

- [x] ~~**O2: Step + Cursor + Phase events.**~~
  ~~Wrap each `engine.step()` call with `Instant::now()`; emit~~
  ~~`TraceEvent::Step` with from/to cursor + result + duration. Every~~
  ~~`self.cursor = …` in `step/` becomes a `Cursor` event. Every phase~~
  ~~advance (`state.next_phase`) becomes a `Phase` event.~~
  (Public `StepEngine::step` brackets a private `step_inner` with `Instant::now()` + `cursor_label` snapshots; emits `TraceEvent::Step{from,to,result,duration_us}`. 21 cursor assignments across `step/{mod,main_phases,combat}.rs` routed through new `StepEngine::set_cursor` helper that emits `TraceEvent::Cursor{from,to}` before assigning. `GameState::next_phase` emits `TraceEvent::Phase{turn,from,to}` after `set_phase`. 7 contract tests in `src/sim/step/trace_tests.rs`.)

- [ ] **O3: Mutation + Count events.**
  `Journal::push` also pushes a `Mutation` event into the engine
  trace (needs a `&mut Vec<TraceEvent>` reference threaded in or a
  thread-local). `state.bump_action` emits a `Count` event with
  before/after.

- [ ] **O4: Oracle + Play + Winner events.**
  `HumanReplayOracle`'s `choose_*` methods emit `Oracle` events with
  asker + request + answer + duration. `state.play_card` emits one
  `Play` event summarizing iid + outcome + cost-paid + duration. Every
  `state.winner = Some(…)` emits a `Winner` event with cause
  (deckout, suicide, combat damage to player).

- [ ] **O5: Minimal UI rendering.**
  `play.html`'s LOG panel renders `env.trace` as one structured line
  per event with category prefix + Δms timestamp. No filters yet —
  raw stream. Verify the engine narrates a full turn end-to-end.

## Phase 2 — AI internals

Every AI pick must be auditable: the candidate set, the scoring, the
reason this candidate was chosen, the reasons others were rejected.
Subsumes my existing `UctTrace` ASCII tree — that becomes a structured
`AiPick` payload.

- [ ] **O6: Heuristic AI narration.**
  `pick_random_playable_in_hand` + `play_priority_score` emit one
  `AiPick { ai: Heuristic, candidates: [(iid, score)…], chosen, search: Flat }`.
  Includes affordability rejections (which candidates were filtered
  before scoring + why).

- [ ] **O7: UCT + MCTS narration.**
  `pick_play_uct` emits `AiPick { ai: Uct, … , search: Tree(UctSearchBreakdown) }`
  with per-iteration breakdown: selected path, rollout winner,
  backprop deltas. `pick_play` (MCTS) emits per-candidate rollout
  outcomes. Delete the bolted-on `UctTrace` ASCII-tree code — its data
  belongs in the structured trace.

- [ ] **O8: Attacker / blocker selection narration.**
  `select_attackers` + `pick_blocks` emit `Combat(AttackerPicked { … })`
  / `BlockerPicked { … }` events with eligibility + rejection reasons.
  AI's combat decisions become explicable.

## Phase 3 — Lua handler narration

Every `fire_self_only` / `fire_with_partner` call in `lua_api.rs` emits a
`Handler` event with event name, source iid, partner iid, return value
(or error), and wall-clock duration. Card authors debug handlers by
reading the trace, not by sprinkling `game.print()`.

- [ ] **O9: Handler entry/exit instrumentation.**
  Bracket every `fire_*` call site in `lua_api.rs` with `Instant::now()`
  + trace push. Capture handler errors verbatim (with the existing
  log-and-continue semantics preserved). Lua-side `game.print()` keeps
  working — its lines also become `TraceEvent::Handler` payloads.

- [ ] **O10: Event firing site coverage.**
  Audit every place the engine fires an event (`on_play`, `on_attack`,
  `on_blocked_by`, `on_block`, `on_die`, `on_enter_board`, future
  events). Each becomes a trace event INCLUDING the no-op case
  ("event fired, no handler registered" → still useful for finding
  missing handlers).

## Phase 4 — Combat granularity

Combat is where players lose games. The trace must answer "why did X
die," "why couldn't I block," "where did the damage go."

- [ ] **O11: Eligibility + restriction trace.**
  `eligible_attackers` / `eligible_blockers` emit `Combat(EligibilityCheck)`
  events with per-candidate result + reason (tapped, summoning-sick,
  flying-vs-grounded, cannot-block restriction, etc.).

- [ ] **O12: Damage assignment + death trace.**
  `confirm_blocks` emits `Combat(DamageAssigned { attacker, blocker_or_player, amount })`
  per attacker. Death checks (C.15 continuous + B.8 combat) emit
  `Combat(Death { iid, cause })`. Mill events emit
  `Combat(Mill { player, iid_from_top })`.

## Phase 5 — UI surface

The LOG panel becomes a structured component. Filter chips per category,
color per category, Δms timestamps, expandable per-event payload (click
an `AiPick` row → see the candidate table; click a `Mutation` row → see
the before/after values).

- [ ] **O13: Filter chips + category coloring.**
  Top of LOG panel: row of toggle chips per `TraceEvent` category.
  Hidden categories filter from view (event keeps its row but greys
  out). Each category gets a stable color so the eye can pattern-match.

- [ ] **O14: Click-to-expand + click-an-iid-to-highlight.**
  Expanding an event row shows its full structured payload. Clicking
  any `iid` in the trace highlights every event involving that
  instance in the stream — full card lifecycle visible.

- [ ] **O15: Persistent local storage.**
  Trace persists across page reloads via `localStorage` (one circular
  buffer per session). "Clear trace" button. "Copy trace to clipboard"
  → JSON dump for sharing in an issue or pasting into a test fixture.

## Phase 6 — Replay infrastructure

Today's "save the seed + script the oracle answers" capability is buried.
Phase 6 promotes it to a first-class feature: any trace can be saved,
loaded, and replayed bit-identically.

- [ ] **O16: Trace serialization + load.**
  Trace + initial game state + seed serialize to one JSON blob. A
  `tsot replay <trace.json>` CLI command reconstructs the engine and
  steps through, asserting each emitted `TraceEvent` matches the
  recorded one. Drift halts with the divergence point.

- [ ] **O17: In-UI replay scrubber.**
  The UI gains a timeline scrubber. Drag to any past event → state
  rewinds to that moment (via journal rollback). Frame-by-frame
  inspection of any past game. Future: branching what-ifs ("what if I
  passed here instead of attacking?").

- [ ] **O18: Two-trace diff.**
  `tools/trace-diff.py` consumes two JSON traces and emits the first
  divergence point + a short side-by-side context. Used by the parity
  + regression test infrastructure. Catches behavior changes the test
  suite doesn't have a specific assertion for.

## Phase 7 — Performance dashboard + budgets

Latency is observability too. Phase 7 aggregates the duration fields
from existing trace events into a live perf surface.

- [ ] **O19: Per-step latency histogram.**
  Aggregate every `Step` event's `duration_us`. Render as a histogram
  in the UI: how many steps under 1ms, 1-10ms, 10-100ms, >100ms.
  Updated live during play. Catches regressions immediately.

- [ ] **O20: Per-handler + per-cursor aggregates.**
  Pivot the trace by `Handler.event` and by `Step.to` cursor: total
  time, mean, p95, n. Surfaces "this Lua handler is slow" or "the
  Main2Pick cursor is doing too much work" without a profiler.

- [ ] **O21: Memory + budget surface.**
  `trace.len()`, `replay_journal.len()`, `card_pool.len()`,
  `engine.log.len()` displayed live. Budget red-lines if a buffer
  outgrows a configurable cap (default 10k events; rotates to keep
  the tail). Catches memory growth before it bites.

---

## Cross-cutting design decisions

1. **Where does the trace live during a step?** Threading `&mut Vec<TraceEvent>`
   through every engine call is invasive. Alternative: thread-local
   `RefCell<Vec<TraceEvent>>` that the engine pushes into, drained at
   step boundaries. Choose one before O1.

2. **What about hot-path overhead?** A trace push allocates. For native
   EA / probe runs we don't want to pay this. Gate via a `trace_enabled:
   bool` flag on `StepEngine`; off by default for native, on for wasm.
   Native CLI gains a `--trace` flag for opt-in.

3. **Trace versioning.** Once O16 ships, recorded traces must replay
   forever. Bump a `trace_format_version: u32` on every breaking enum
   change. Old traces explicitly refuse to load against a newer engine.

4. **String interning.** `Handler.event` and `Count.key` are `String`s
   in the draft API. For tens of thousands of events per game these are
   hot. Use `&'static str` or a small id table once the bus matures.

5. **Test integration.** Once events are structured, tests can assert
   trace contents: "after playing X, trace contains a `Handler{event: OnPlay,
   source: X}` event." Replaces a lot of state-poke-and-check asserts.
   Worth a follow-up doc once Phase 3 lands.
