# tsot — Known Limitations

> What the engine cannot do today. Code TODOs are tagged so they map back
> to a section here. Last refresh: 2026-05-30 (post-Pattern-B / Phase 3.5).

## events

- **`OnDealtDamageToPlayer`** — no per-attacker post-combat trigger for "this creature successfully damaged the defender's deck." Cinder Wurm currently uses `on_attack` as a workaround (fires whether blocked or not).
- **Phase-entry triggers** — `on_turn_end`, `on_upkeep`, `on_untap_step`. Coupled with the delayed-trigger registry; usually wired together.
- **Delayed-trigger registry** — handlers can't queue future triggers. Required by slow-recall (recurring exile return), attach-shuffler (delayed bounce), bitter-dawn's effect 2 (next-turn sacrifice).

## costs

- **SELF / SelfExile** (P.5) — played card itself → EXILE on resolution. Originally on opponent-draw (currently a HAND substitute).
- **Subtype filter on SACRIFICE** — `CostComponent.kind` filters by CardType today. A `subtypes` filter ("sacrifice a goblin") is the next gap; no current card needs it.
- **Variable X for spells in playability filter** — `pick_random_playable_in_hand` rejects spells with `is_x` cost. Shift can't be selected by the sim AI as a result. Creatures with X cost bypass this gate (hydra plays normally).

## types

- **Environment** (P.21) → BOARD with P.22 (one at a time, global) + P.23 (can't replace). Displacement question unresolved.

## targeting

No engine concept of "what is legal to target." Every "target X" card today works because the handler builds the pool itself. Affects every removal/redirect/buff card with explicit targeting.

- **Targetability protection** — Reef Phantom's "tapped → untargetable" can't be enforced. Hexproof / shroud / "can't be targeted by opponents" all need this layer.
- **Multi-target / divided effects** — handlers can call `choose_card` multiple times but no API for "deal 3 damage divided as you choose among any number of targets." Single-choice multi-output patterns aren't expressible.
- **Target-validity recomputation** — if a target becomes illegal between cast and resolution (e.g., it left the board), the engine has no "fizzle if target is gone" check.

STATIC Phase 3 (restriction statics) partially overlaps here; the targeting infrastructure is its own slice.

## stack

- **UX X.1, X.2, X.3 skip-logic** — auto-pass when no playables, auto-pass when no legal target, "opponent considering response" marker.
- **Resolution event metadata** — `unopposed` / `declined` flags on resolved items.
- **Option B refactor** — `play_card` stops driving the priority loop internally; caller drives. Matches the UI shape.

## static

- **Phase 4 — replacement effects.** "Would die → exile instead." No corpus card requires it yet.
- **Static-driven recomputation when attached set changes.** Hydra's ETB stat snapshot persists after falter strips its attached cards.
- **Reef Phantom's tapped-untargetability.** Needs a targeting layer first (see the `targeting` section); once that exists it's a one-line restriction static.

## activated abilities

Not started. Player-initiated `T: ...` activations. Needed by DTST-creature (5 Tap-abilities), DTST-creature2, vigilant-human, the jewel cycle's granted `T: draw, discard` rider. Scope: Lua declaration syntax, activation flow that puts the ability on the stack, sim AI decision hook, cost-payment integration.

## state-based actions (SBAs)

Not started. `combat.rs:321` has a TODO marker. tsot does combat damage + death check in one atomic pass; MTG-style SBAs fire BETWEEN stack-item resolutions so "regenerate" / "prevent damage" responses can save a dying creature.

## sim AI strategic depth

These aren't engine bugs but they limit the validity of sim-based playtest signals. The AI:

- **At most one creature per turn** (Pattern B). Multiple non-creatures per turn are allowed as long as the AI can afford their costs. No "play A, evaluate, then play B" planning beyond the priority-score tier ordering.
- **Attacks with everything eligible, always.** No "hold back this 1/1 to chump-block next turn." No "don't attack into the obvious bitter-dawn." Block policy got smart (tiered survival → kill-trade with trade-up → chump → multi-block); attack policy did not.
- **No mulligan decision.** Engine deals first 5 cards as the hand, period. Real games have S.2/S.3 redraw. The sim never explores "this opening hand is unplayable."
- **No proactive instant timing in main phase.** Instants only fire from the response policy in R.1.a/R.1.b windows. Pre-emptive "cast surge before combat to enable a vigilance line" never happens.

## smaller items

- **P.8 attached → EXILE on host's death** — attached cards currently get dropped on the floor or stay attached depending on path. RULES says exile.

---

Code TODOs are tagged in source. Grep `grep -rn 'TODO(' src/` for the full list.
