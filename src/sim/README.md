# sim — AI policy + game runner

The sim drives `run_game` (random-but-legal play, no human input) and is the
fitness signal for the EA. The AI lives in `ai.rs`; main loop in `run.rs`.

## AI heuristics (current state)

- **Pattern B per-turn play loop** — at most one creature per turn, but as many
  non-creatures as the AI can afford. Inner safety cap (`pattern_b_iter`) catches
  picker → continue → re-pick infinite loops.
- **Play-priority scoring** (`play_priority_score` in `ai.rs`) — cost-reducing
  statics and stat-anthem statics land first so they compound across following
  turns. Tiered pick: filter to max-priority tier, choose randomly within tier.
- **Smart-pitch** — when paying HAND cost, score hand candidates by pitch-payoff
  (jewels / zebra / mantis-shrimp matching the host's color) and avoid pitching
  Clear Views unless the cast actually needs the slot.
- **Smart-discard** — U.10 hand-size discards prefer low-value cards (no
  handlers, low cost, low identity match for upcoming casts).
- **Smart-target** — intent-aware via `game.set_intent` side-channel. Handlers
  declare the purpose of the next `choose_card` call; the oracle dispatches to
  intent-specific scoring. See `TargetIntent` catalog below.
- **Trade-up block policy** — tiered: clean-kill (block dies, attacker dies, no
  body loss for blocker); kill-trade (block dies, attacker dies); multi-block
  only when the attacker would otherwise reach a dying threshold.
- **Trade-up attack policy** — big-first blocker reservation, skips clean-kill
  suicide swings (don't attack with a 1/1 into a 3/3), reach-aware (flyers vs.
  ground / reach blockers).
- **Investment-aware sacrifice picker** — `sacrifice_keep_value` (also reused
  by the block policy's trade-up calc) scores `x + y + cost*2 + attached*2`.
  Higher = more valuable to keep = picked last for sacrifice cost.
- **Activation passes** — pre-combat pass for non-creatures (e.g., the jewel
  cycle's `T: draw, discard`), post-combat pass for everything. AI picks X for
  X-cost activations via the tightest-resource cap.
- **Clear View hand-substitution** — when identity-matching hand cards are
  short for a cast's HAND cost, the AI greedily uses Clear View copies from
  graveyard (`gy_hand_substitute = true`) to fill slots, subject to the
  `NoHandPaymentForIdentity` gate.
- **Attached-payment selection** — for P.31 ATTACHED-source cost components,
  the AI picks first-N from `find_attached_payments` sorted ascending by
  `attached_keep_value` (mutation-presence, crystal-color-uniqueness,
  granted-activated, shell-redundancy — weights placeholder pending EA tuning).
- **`rig_creature_free_haste`** — non-setup-cost creatures get their cost
  wiped and haste granted at cast time. Lets the EA explore creature payoff
  without modeling early-game hand economy; setup-cost creatures (Sacrifice /
  Graveyard cost) keep their printed cost.
- **Per-game watchdog** — wall-clock and Pattern-B-inner-loop safety caps in
  `run_game`. On hang the engine dumps state to stderr (turn, active player,
  hand/board/GY card ids, last picked, last activated), scores the active
  player as the loser, and continues. Tunable via `TSOT_GAME_TIMEOUT_SECS`.

## `TargetIntent` catalog

`RandomOracle::choose_card` reads an optional side-channel `TargetIntent` set
by handlers via `game.set_intent(...)` and dispatches to intent-specific
scoring. Intent is consumed on the next `choose_card` (cleared after one use),
so handlers re-declare per call site. Scripted and Noop oracles ignore the
hint.

Intents wired today:

- `steal` (opp-bias + attached-aware) — `shift` source, `falter`
- `donate` (own-bias + body-aware + attached-aware) — `shift` destination
- `high_value_attached` (no controller bias, prefer jewels/statics) — `shift`'s per-attached pick
- `remove_threat` (opp-bias + body-aware) — `silent-murder`, `beguile`, `bring-down`, `condemn`, `jellyfish`, `this-for-that`'s "take"
- `recur` (cost-heavy + handler-density, no controller bias) — `mesopelagic-fish`
- `low_value_own` (own-bias + INVERSE body-aware) — `this-for-that`'s "give"

Targeted cards still on default scoring: `archer`, `cinder-wurm`,
`pyre-spirit`, `portable-bolt`, `sabotage`, `forget`, `glaring-sunlight`,
`resurrect`, `wake-dead`, `philosopher`, `untap`, `flesh-eating-plant`,
`goblin-conspirator`, the monkey cycle's `T:` picks, `ward`, `sparkle`,
`scarecrow`, `bci-megafly`, `blue-scientist`, `signal-goblin`. Each one is
~5 lines of Lua to wire; intents may need extending (e.g., `pump` for buff
targets, `discard_opp` for hand-attack).

## Files

- `ai.rs` — picker, affordability check (`can_pay_instant_cost`), priority
  score, block/attack policies, keep-value functions, helpers consumed by
  `run.rs`.
- `run.rs` — main game loop. Drives turns, calls into `ai.rs` for picks,
  routes through `play_card` / combat.
- `evolve.rs` + `fitness.rs` + `genome.rs` + `ops.rs` + `parallel_eval.rs` —
  EA scaffolding around `run_game`.
- `evolved_deck.rs` + `deck_token.rs` — saved-deck format (JSON files in
  `baselines/` / `champions/`).
