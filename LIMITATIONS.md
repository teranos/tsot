# tsot — Known Limitations

> What the engine cannot do today. Code TODOs are tagged so they map back
> to a section here. Last refresh: 2026-06-01 (shift intent-aware targeting).

## events

- **`OnDealtDamageToPlayer`** — no per-attacker post-combat trigger for "this creature successfully damaged the defender's deck." Cinder Wurm currently uses `on_attack` as a workaround (fires whether blocked or not).
- **Phase-entry triggers** — `on_turn_end`, `on_upkeep`, `on_untap_step`. Coupled with the delayed-trigger registry; usually wired together.
- **Delayed-trigger registry** — handlers can't queue future triggers. Required by slow-recall (recurring exile return), attach-shuffler (delayed bounce), bitter-dawn's effect 2 (next-turn sacrifice).

## costs

- **SELF / SelfExile** (P.5) — played card itself → EXILE on resolution. Originally on opponent-draw (currently a HAND substitute).
- **Subtype filter on SACRIFICE** — `CostComponent.kind` filters by CardType today. A `subtypes` filter ("sacrifice a goblin") is the next gap; no current card needs it.
- **Variable X for spells in playability filter** — fixed. `can_pay_instant_cost` accepts is_x cost components (X=1 minimum affordability). `run.rs` X-pick caps X by the tightest binding resource (identity-matching hand size, deck size, GY size, board-creature count for sacrifice). Per RULES P.30 the AI picks `min: 1`, and the engine rejects `x_value = Some(0)` with `XBelowMinimum` unless the card sets `allow_x_zero = true`. Recast and turn-back-time stay filtered (SelfExile cost). Shift now has an `on_play` handler (X mill → move X attached between hosts); recast and turn-back-time remain handler-less.
- **GY → EXILE as HAND-payment substitute** — engine path implemented (`Card.gy_hand_substitute` flag, `PlayChoices.gy_hand_payment_ids`, sim AI integration via `find_gy_hand_substitutes` + `identity_matching_hand_count`, `NoHandPaymentForIdentity` gate). Unit-tested but not yet end-to-end confirmed by an EA round actually drafting and exploiting Clear View. May reveal AI-heuristic gaps (e.g., when to keep Clear Views in GY vs. when to spend them aggressively).

## types

- **Environment** (P.21) → BOARD with P.22 (one at a time, global) + P.23 (can't replace). Displacement question unresolved.

## targeting

No engine concept of "what is legal to target." Every "target X" card today works because the handler builds the pool itself. Affects every removal/redirect/buff card with explicit targeting.

- **Targetability protection** — Reef Phantom's "tapped → untargetable" can't be enforced. Hexproof / shroud / "can't be targeted by opponents" all need this layer.
- **Multi-target / divided effects** — handlers can call `choose_card` multiple times but no API for "deal 3 damage divided as you choose among any number of targets." Single-choice multi-output patterns aren't expressible.
- **Target-validity recomputation** — if a target becomes illegal between cast and resolution (e.g., it left the board), the engine has no "fizzle if target is gone" check.

STATIC Phase 3 (restriction statics) partially overlaps here; the targeting infrastructure is its own slice.

### Smart targeting heuristics (`TargetIntent`)

`RandomOracle::choose_card` now reads an optional side-channel `TargetIntent` (set by handlers via `game.set_intent("steal"|"donate"|"high_value_attached")`) and dispatches to intent-specific scoring. Intent is consumed on the next `choose_card` (cleared after one use), so handlers re-declare per call site. Wired in shift's `on_play` (source = `steal`, destination = `donate`, per-attached pick = `high_value_attached`). Scripted and Noop oracles ignore the hint. Other targeted cards (beguile, silent-murder, mutation cards) still use the default `target_score` and remain candidates for intent-aware scoring; the framework is in place but each card has to declare its intent.

## stack

- **UX X.1, X.2, X.3 skip-logic** — auto-pass when no playables, auto-pass when no legal target, "opponent considering response" marker.
- **Resolution event metadata** — `unopposed` / `declined` flags on resolved items.
- **Option B refactor** — `play_card` stops driving the priority loop internally; caller drives. Matches the UI shape.

## static

- **Phase 4 — replacement effects.** "Would die → exile instead." No corpus card requires it yet.
- **Static-driven recomputation when attached set changes.** Hydra's ETB stat snapshot persists after falter strips its attached cards.
- **Reef Phantom's tapped-untargetability.** Needs a targeting layer first (see the `targeting` section); once that exists it's a one-line restriction static.

## activated abilities

Phase 1 landed: `T:` activations on BOARD-zone cards (RULES A.5–A.7). Lua schema `activated = {{cost, text, timing, effect}}`, engine `activate_ability` + `can_activate`, sim AI fires pre-combat (non-creatures) and post-combat (everything) passes. Wired into 6 jewels + vigilant-human.

Phase 1.5 landed: multi-component activation costs (RULES A.8). Cost can include any combination of `T:` plus HAND, MILL, or GRAVEYARD components in the play-card cost vocabulary. SACRIFICE and SELF reserved. Wired into the monkey cycle (5 cards: red, blue, pink, purple, white), each with a `2 hand:` activation.

Phase 1.75 landed: X-cost activations. Cost components can carry `is_x = true`; the sim AI picks an X value via a hand-size heuristic and `activate_ability` multiplies amounts accordingly. Handlers read the chosen X via `game.x_value()`; the validate hook can refuse based on X-dependent math. Wired into Dark Salamander's `Y hand: mill opp by 2Y - X` activation.

Phase 3 landed: static-granted activated abilities (RULES A.10). `StaticDef.granted_activated` lets a card's static effect grant a full activated ability to matching candidates. The 6 jewels now grant `T: draw, discard` to the creature they're pitched onto (their host). `GameState::activation_count` / `activation_at` resolve indices across printed + granted activations transparently; the sim AI's activation pass picks up granted abilities the same as printed ones.

Deferred:
- SACRIFICE / SELF cost components in activations — needed by Portable Bolt's "exile this card" rider (SELF).
- Activations from non-BOARD zones — needed by Portable Bolt's portable rider (activate from ATTACHED) and by cycling-style hand activations.

Per RULES A.5 activations resolve immediately and cannot be responded to. This is a deliberate deviation from MTG and is not on a "to fix" list.

## state-based actions (SBAs)

Not started. `combat.rs:321` has a TODO marker. tsot does combat damage + death check in one atomic pass; MTG-style SBAs fire BETWEEN stack-item resolutions so "regenerate" / "prevent damage" responses can save a dying creature.

## sim AI strategic depth

These aren't engine bugs but they limit the validity of sim-based playtest signals. The AI:

- **At most one creature per turn** (Pattern B). Multiple non-creatures per turn are allowed as long as the AI can afford their costs. No "play A, evaluate, then play B" planning beyond the priority-score tier ordering.
- **Attack policy is per-attacker myopic.** `select_attackers` walks attackers big-first and reserves the defender's clean-kill blockers for top threats, skips swings that die for nothing, and mirrors the defender's T2 gate for trade-up gating; reach-aware. What it still doesn't do: hold back to chump-block next turn, anticipate response-window instants (bitter-dawn, counterspell), or plan multi-turn lines. `cats-block-birds` / `rats-can't-block-cats` subtype rules are not modelled here either — the AI is mildly optimistic vs cats blocking flyers and mildly pessimistic vs rats facing cats.
- **No mulligan decision.** Engine deals first 5 cards as the hand, period. Real games have S.2/S.3 redraw. The sim never explores "this opening hand is unplayable."
- **No proactive instant timing in main phase.** Instants only fire from the response policy in R.1.a/R.1.b windows. Pre-emptive "cast surge before combat to enable a vigilance line" never happens.

## smaller items

- **P.8 attached → EXILE on host's death** — attached cards currently get dropped on the floor or stay attached depending on path. RULES says exile.

## EA / evolutionary deck search

Three biggest limitations on what conclusions the EA can support today.
Below these, smaller items.

### Biggest

1. **Population collapse → one run finds one attractor.** Within 15-20 generations a 50-pop run converges (observed: `mean=0.435 → 0.953`, `min=0.086 → 0.829`). The top-5 of a single run share 40+ of 50 cards — they are not independent samples, just five slot-variations on the same evolved deck. *Card-design conclusions need many runs at different `--seed` values, not many ranks from one run.* No diversity-preserving selection (Jaccard penalty, fitness sharing) is wired today.

2. **Fitness noise floor hides weak cards.** At `--n 10`, within-genome stddev is ~0.043. A non-functional 1-of card costs ~2% fitness — below the noise floor. Selection cannot tell "deck with shift" from "deck without shift," so junk 1-ofs persist indefinitely. Above the ceiling: any fitness ≥ ~0.957 is statistically indistinguishable from 1.000, so champion ranking near the top is meaningless. `--n 50` drops the floor to ~0.019 (5× longer runs).

3. **Gauntlet drift.** Champions are strong against the gauntlet they were evolved against (current `baselines/` + accumulated `champions/` extras). As baselines get curated and new champions added, prior champions' saved fitness numbers become non-comparable. `curate-baselines` uses live re-evaluation against the snapshot baselines (apples-to-apples), but a champion saved at "fitness 0.95" against a small early gauntlet may live-win 0.4 against the current strong baselines. *Saved fitness is local to its run; trust live re-evaluation.*

### Smaller

- **Below-noise-floor in champions-report too.** A 5-champion report with `5/5 presence at mean_copies=1.0` is barely above null. Confident "card X is load-bearing" needs 20+ independent champions; the `--save-top K` flag shortcuts collecting these but only within one attractor (see (1)).
- **Champion artifacts age with the card pool.** Saved champions' 50-slot composition is frozen at save time. Adding new cards doesn't invalidate them, but they cannot benefit from new cards either. **Removing** a card breaks every champion whose genome references it — those champion files load but fail to materialize, and the EA's gauntlet skips them with a warning. No auto-pruner today; manually `rm` the stale champion files (or run `make prune-champions` which will live-rank them and likely drop the broken ones since they can't be evaluated).
- **No mid-run hall-of-fame.** Gauntlet is fixed at run start. Champions discovered mid-run don't become opponents until you start a new run with them as `--extra`.
- **`--save PATH` overwrites unconditionally.** A weaker champion silently replaces a stronger file when seeds collide. Backup before risky configs.
- **Parallel speedup caps at ~3.4× on 8 cores.** Each rayon worker pays Lua VM init cost on first touch (~500ms) and the inner game loop has internal serialization that the embarrassingly-parallel fitness step can't fold out. 25-min runs become ~7-8 min — meaningful but not core-count-linear.

---

Code TODOs are tagged in source. Grep `grep -rn 'TODO(' src/` for the full list.
