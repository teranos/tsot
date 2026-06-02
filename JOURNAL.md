# tsot — Journal & Rollback Plan

> Multi-session architectural plan for journal-based mutation tracking.
> Foundation for preview-for-AI, replay, save/load, undo, multiplayer rollback.

## Status (2026-06-02)

- **Sessions 1–4: complete.** Mutation logging across every subsystem
  (turn, combat, play, lua_api), in-place preview-and-rollback in the sim,
  suicide-play skipping, future-simulation telemetry surfaced.
- **Session 5, steps 1–4: complete.** In-memory replay capture, file dump
  via `TSOT_REPLAY_OUT=<path>`, round-trip JSON serialization with
  forward-apply, and mid-game save/load with Lua-handler rebinding. Tests
  in `tests/replay.rs` and `tests/save_load.rs` cover both round-trips.
- **Session 5, step 5 (undo):** deferred — gated on interactive UI.
- **Session 6 (AI search): one-ply MCTS landed.** Journal-based rollback
  proved out for full-game speculative execution; first AI consumer is
  `sim::mcts::pick_play` (one-ply rollouts per candidate card-pick). See
  the section below for what shipped and what's still deferred.
- **Multiplayer rollback netcode:** still deferred.

---

## Why this exists

tsot's engine today mutates `GameState` directly at scattered sites
(`turn.rs`, `combat.rs`, `play.rs`, `lua_api.rs`, etc.). Each mutation is
final — no way to preview, rewind, or replay. This blocks:

- **Sim AI preview** — "would playing this card kill me?" requires
  speculative execution. Today's options are hardcoded special cases
  (Option 1) or full state clone (Option 2, leaks shared Lua VM state).
- **Replay** — sharing or rewatching a game requires the sequence of
  mutations.
- **Save / load** — restoring a game mid-play requires a serializable
  representation.
- **Undo** — never possible without recorded mutations to reverse.
- **AI search trees** — branching speculative state cheaply requires
  journaled state, not full clones.
- **Multiplayer rollback netcode** — fundamentally requires deterministic
  forward execution + rollback to a prior frame.

The pattern: every state mutation logs an entry into a journal. Each
entry knows its own inverse. Rollback applies inverses in reverse. The
recording is optional — `journal: Option<Journal>` on `GameState` means
zero overhead when not in use.

---

## Key principles

1. **Always-on overhead = zero.** `journal: Option<Journal>` — production
   sim without preview pays nothing.
2. **Single source of truth for mutations.** Once a journaled helper
   exists (e.g. `set_tapped`), direct field assignment to `inst.tapped`
   is forbidden. Lintable via clippy if it becomes an issue.
3. **Inverse correctness is a test invariant.** Every mutation variant
   has a round-trip test: apply → rollback → assert state equal.
   Adding a new variant without its inverse + test is a regression.
4. **Sessions end coherent.** No "half the subsystems are journaled"
   intermediate states landed. Each session leaves the codebase
   internally consistent.

---

## Session 1 — foundations

**Goal:** the abstractions exist, one subsystem demonstrates the pattern,
the round-trip invariant has tests.

**In:**
- `Journal` struct holding `Vec<JournalEntry>`
- `JournalEntry` enum with the core mutations:
  `SetTapped`, `SetDamage`, `SetFaceDown`, `SetSummoningSick`,
  `MoveCard`, `BumpAction`, `BumpEventFire`, `SetWinner`,
  `AddModifier`, `AddStatusEffect`, `AddAttached`
- Each variant carries the data needed to undo it (old values,
  positions, etc.) — *not* deltas
- `GameState.journal: Option<Journal>` field
- Mutation helpers on `GameState`: `set_tapped`, `set_damage`,
  `set_winner`, etc. — auto-log when journal is open
- `Journal::rollback(self, state: &mut GameState)` applies inverses in
  reverse order
- **`movement.rs::move_card`** rewritten to use journaled helpers
  (canonical pattern demonstration)
- Round-trip test: open journal, apply N mutations, rollback, assert
  state equal to pre-apply

**Out:**
- Other subsystems (still mutate directly — that's Session 2)
- Lua-side journaling (Session 3)
- Sim preview integration (Session 4)
- Save / load / replay surface (Session 5+)

**Deliverable:** `cargo test` includes the round-trip test. All existing
tests still pass. `movement.rs::move_card` is the journaled-helpers
example any future subsystem refactor can copy.

---

## Session 2 — coverage

**Goal:** every direct state mutation in pure-Rust engine code goes
through journaled helpers.

**In:**
- `turn.rs` — `do_untap_step`, `do_draw_step`, `do_end_step`,
  `clear_all_damage`, `next_phase` (phase/turn/active swap)
- `combat.rs` — `declare_attacker` (tap, combat state push),
  `declare_blocker` (attacks vec push), `resolve_combat` (damage,
  deaths, mill, exile)
- `play.rs` — `play_card` (deck remove, hand remove, board push,
  modifiers, face_down, summoning_sick, attached push)
- Direct field mutations replaced with helper calls
- One round-trip test per subsystem confirming the journal correctly
  reverses every change made by these methods

**Out:**
- Lua-side helpers (`do_damage`, `do_mill`, `do_draw`, `do_move`,
  `do_set_tapped`, `do_add_status`, `do_discard`) — Session 3
- Anything calling into the journal from Lua handlers — Session 3

**Deliverable:** every mutation site in `turn.rs`, `combat.rs`,
`play.rs` is journaled. A round-trip test for each subsystem proves
correctness.

---

## Session 3 — Lua side

**Goal:** handler-driven mutations (via `game.*` methods) are journaled
just like engine-driven mutations.

**In:**
- The scoped `state_cell` in `lua_api.rs::build_game_table!` macro
  becomes a bundle: `(GameState, Journal)`-aware
- Every `do_*` function in `lua_api.rs` (`do_damage`, `do_mill`,
  `do_draw`, `do_move`, `do_set_tapped`, `do_add_status`,
  `do_discard`) logs its mutations as it runs
- Round-trip test: fire a handler that calls multiple `game.*` methods,
  rollback, assert state equal to pre-fire

**Out:**
- `game.choose_card` / `game.confirm` are reads from the oracle, not
  mutations of state — they don't go in the journal. But the side
  effects they enable (handler's subsequent `game.move`, etc.) do.
- Sim preview integration — Session 4

**Deliverable:** every state mutation, engine-driven or
handler-driven, is journaled. Full round-trip works for arbitrary
handler executions.

**Open design note:** if a future card author writes a handler that
mutates Lua's `_G` (or anything in the shared mlua VM), that mutation
is *not* journaled — it lives in the Lua VM, not `GameState`. Convention
"handlers don't touch `_G`" stands; documented in card-authoring guide
when that doc lands. Tests using `_G` as a side-effect channel are
test-only fixtures and don't use preview.

---

## Session 4 — sim preview-for-AI

**Goal:** the sim uses journal-based preview to skip plays that would
end the active player's own game.

**In:**
- Sim picks a candidate play
- Opens a journal on the live `GameState`
- Plays the card (mutations record into the journal)
- Checks: `state.winner == Some(active_player.opponent())` → suicide play
- Rolls back via `journal.rollback(state)`, skips the card
- Otherwise commits (closes the journal, mutations stay)
- New sim row: how many plays were *previewed and skipped* as suicide
- Existing `self_deckout_by_choice` counter drops to zero

**Out:**
- Smarter heuristics beyond suicide-avoidance (combat trade
  prediction, etc.) — separate AI work
- Multi-step lookahead — separate AI work

**Deliverable:** sim AI no longer plays draw-effects that would deck
itself. The "self_deckout_by_choice" row shows 0. A new "previewed
suicide play skips" row shows the actual number of avoided suicides.

**Performance budget:** clone-free preview should be cheap. Acceptable:
≤2× current sim runtime. If it's worse, optimize the journal data
structures (Box<JournalEntry> chains, etc.).

---

## Session 5 — replay, save/load, undo (separate arcs)

These are independent landings on top of the journal infrastructure that
Sessions 1–3 built. Sub-step labels match the implementation order.

### Step 1 — in-memory replay capture (DONE)

- `GameState.replay_journal: Option<Journal>` opened at game start
- Every committed mutation records into it (preview mutations stay
  isolated in the per-action `journal` slot)
- At game end, journal contains the full mutation sequence
- Sim reports avg / min / max entries per game

### Step 2 — file dump (DONE)

- `ReplayFile { seed, deck_a_card_ids, deck_b_card_ids, journal }` in
  `src/replay.rs`
- `JournalEntry`'s `Set*` variants carry both `was` and `now` so
  `apply_forward` and `apply_inverse` are both defined
- `Serialize` / `Deserialize` derives on `PlayerId`, `Phase`, `Zone`,
  `EventName`, `StatusEffect`, `CombatState`, `AttackDecl`,
  `JournalEntry`, `Journal`
- `action_counts: BTreeMap<String, [u32; 2]>` (was `&'static str` keys;
  changed for serializability — any action name now works)
- `TSOT_REPLAY_OUT=<path>` env var dumps the last game's `ReplayFile` to
  pretty JSON

### Step 3 — load + replay (DONE)

- `ReplayFile::from_json` deserializes
- `rebuild_initial_state(&CardRegistry)` rebuilds initial `GameState`
  from the recorded deck ids
- `Journal::replay_forward(&mut state)` applies entries in order
- `tests/replay.rs` round-trip: capture → JSON → deserialize → rebuild
  → forward apply → final state byte-identical to original

### Step 4 — save/load (DONE)

Distinct from replay: save/load resumes mid-game by serializing the
**current state**, not the journal-from-start.

What landed:
- `Serialize` / `Deserialize` on `Card`, `CardInstance`, `PlayerState`,
  `GameState`, `Modifier`, `CostComponent`, `CostSource`, `CardType`,
  `Stats`
- `#[serde(skip, default)]` on `Card.handlers` — `mlua::Function` isn't
  serializable. `rebind_handlers(state, registry)` in `src/replay.rs`
  walks `card_pool` after deserialization and re-attaches handlers from
  a live registry
- `SaveFile { master_seed, state }` with `from_state`/`to_json`/
  `from_json`/`restore`
- `tests/save_load.rs` covers the round-trip (save → JSON →
  deserialize → restore → continue playing matches uninterrupted
  control) and the handler rebind (handlers execute after restore)

### Step 5 — undo (gated)

- Add `Journal::rollback_to(checkpoint: usize)` for partial rollback
- Define "user input boundary" = where to roll back to (per turn / per
  decision)
- Needs interactive UI / CLI to be useful — not reachable until
  someone's actually playing tsot interactively

## Session 6 — AI search (partial)

**One-ply rollout MCTS shipped.** Lives in `src/sim/mcts.rs`. `pick_play`
enumerates playable hand candidates, runs N Heuristic-vs-Heuristic
rollouts per candidate via `run_game_continue`, and picks highest
win-rate. Each rollout opens a fresh per-rollout journal, runs the
game to completion, then rolls back — outer `replay_journal` is saved
and restored so the search is fully transparent to the caller.

What this required:

- `run_game_continue(&mut state, rng, log, lua, ais: &[AiKind; 2])` —
  resumable game runner taking borrowed state and per-player AI. The
  original `run_game` is now a thin owned-state wrapper.
- `build_pattern_b_choices(...) -> BuildChoiceResult` extracted from
  `run_game_continue` so MCTS rollouts and the real game build
  `PlayChoices` through the same code path. Mismatched choice-building
  was the difference between 17% and 80% win rate in mirror matches —
  the rollouts have to play the candidate move under exactly the same
  payment / target-resolution rules as the real game.
- `JournalEntry::RigCreatureFreeHaste { iid, was_cost, was_abilities }`
  — `sim::ai::rig_creature_free_haste` mutates `inst.card.cost` and
  `inst.card.abilities` outside the normal journaled-helper path, so
  full-game rollback diverged from the initial state. Variant captures
  the pre-mutation values; `apply_inverse` restores them. TODO(B):
  eliminate the rig hack by routing free/haste casts through the proper
  play-cost API.
- `PlayerId::index() → usize` (A=0, B=1) and `GameState::active_journal()`
  promoted to `pub` so MCTS can construct `[AiKind; 2]` arrays and
  access the journal directly.

**Full-game rollback invariant test.** `src/sim/run.rs::full_random_game_rollback_restores_initial_state`
runs a full random game with both players on Heuristic, captures the
journal, rolls back, and asserts per-field equality with the pre-game
state. This is the strongest journal test in the codebase — it caught
the rig_creature_free_haste gap by narrowing failures to specific
`Card` fields. (`tests/journal_full_game_rollback.rs` carries two
weaker scripted-game variants since `sim` is binary-only and the
strongest test has to live next to the code.)

**AI-vs-AI head-to-head.** `cli_matchup_mcts` runs Heuristic vs MCTS
with three modes: mirror (`--deck PATH`), explicit asymmetric
(`--deck-a` + `--deck-b`), and auto (picks two random `baselines/*.json`
or a random genome). `--handicap` forces MCTS onto the lower-saved-
fitness deck. Measurements with current defaults (rollouts=5,
max_candidates=10): MCTS ~76% in mirror, ~61% with a 0.025-fitness
handicap.

**EA-vs-MCTS.** `cli_evolve --opponent-ai mcts` threads `AiKind` through
`EvolveConfig → parallel_evaluate_genomes → fitness/fitness_breakdown
→ run_game_with_ai`. Candidate side always plays Heuristic; only the
gauntlet side plays MCTS. ~16× slower per fitness eval; `make
evolve-mcts` wraps the standard EA shape. See `EA.md` "Evolving against
MCTS opponents."

### Still deferred

- **Multi-ply lookahead.** Current MCTS branches one card-pick deep then
  hands off to Heuristic rollouts. The journal supports deeper search;
  the cost-vs-signal tradeoff and tree-policy choice haven't been
  measured.
- **ISMCTS / hidden information.** Opponent hand identities are visible
  to both players in the current rollouts. Determinization (sample the
  opponent's hand from cards-not-yet-seen) is the natural fix.
- **Transposition tables.** Same state reachable via different paths
  isn't deduplicated. State-hash + journal-aware caching is the right
  shape; not built.
- **Multiplayer rollback netcode.**
  - Each client runs the journal forward in lockstep
  - Diverged inputs trigger rollback to last common frame, replay forward
  - Foundation for online play with low-latency input

---

## Cross-cutting interactions

### With LUA Phase 2 `static` (modifier system)

Static effects add and remove modifiers dynamically. The journal needs
to handle `Modifier` additions/removals cleanly. **Order matters:**

- Doing journal first → static system designed with journaling in mind
  from day 1 (cleaner)
- Doing static first → adding journaling for modifiers becomes a
  retrofit across the static recompute machinery

Preference: **journal first.** The `Modifier` identity question
(`(source, target, kind)` tuple) is the same in both, and answering it
for journal pre-answers it for static.

### With determinism

Journal + determinism are complementary. Together they enable
byte-identical replay. Determinism is already in (`clippy.toml` bans
`HashMap`/`HashSet`/`thread_rng`; tests assert two runs match). The
journal carries this further: not just "same seed → same outcome,"
but "same seed → same journal sequence."

### With STACK theme

When response windows / the stack lands (per `STACK.md`), responses are
*also* state mutations that the journal captures. Rolling back a
response window collapses the stack appropriately.

### With types / costs themes

Spell / Artifact / Environment plays and the missing cost sources
(Sacrifice, Self, Variable-X) all mutate state through `play_card`'s
mutation block. Once Session 2 covers `play.rs`, these are
automatically journaled when they land.

---

## How this fits with LIMITATIONS.md's four themes

This is a fifth, cross-cutting concern — not one of the four themes
(`events`, `costs`, `types`, `stack`), but enabling for all of them.
LIMITATIONS.md references this document for the full plan.
