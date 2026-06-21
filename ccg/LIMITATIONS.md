# tsot — Known Limitations

> What the engine cannot do today. Code TODOs are tagged so they map back
> to a section here. Last refresh: 2026-06-18 (audit: all 10 ChoicePending discard sites closed across play / activate / combat / turn — handlers now suspend end-to-end via typed `*Error::ChoicePending` propagation; OnDealtDamageToPlayer shipped; P.8 attached-cascade partial).

New cards:

156:
Artifact, 0
T:Sac this, Search your library for a card called Amsterdam and put it on the board.

## events

- **`OnDealtDamageToPlayer`** — **shipped + corpus migrated.** `EventName::OnDealtDamageToPlayer` at `src/card.rs:516` fires per-attacker post-mill at `src/game/combat.rs:421-432`, including for every card attached to the successful attacker (klotho-style mutations declare the handler and receive `self = the mutation`). `OnAttack` also iterates the attacker's attached list as of 2026-06-20 (combat.rs declare_attacker, same iteration shape — TNF / VEGF wired). Cinder Wurm migrated from the on_attack workaround to `on_dealt_damage_to_player` 2026-06-20.
- **Phase-entry triggers** — `on_turn_end`, `on_upkeep`, `on_untap_step`. Coupled with the delayed-trigger registry; usually wired together.
- **Delayed-trigger registry** — handlers can't queue future triggers. Required by slow-recall (recurring exile return), attach-shuffler (delayed bounce), bitter-dawn's effect 2 (next-turn sacrifice).

## dev-tool

- **`i` screenshot-to-clipboard shortcut wanted but unimplemented.** Goal: press `i` on the page, the current view lands in the clipboard as a PNG so it can be pasted into Claude Code / Slack / bug reports without ⌘-Shift-4 + drag. Same pattern works in the user's other projects. In TSOT, four attempts failed within one session — none of them surfaced a useful error signal, all of them claimed success in JS while leaving the OS clipboard empty. Documented so the next session doesn't re-walk the same dead ends:

  1. **`html2canvas` via CDN script tag + `async` keydown handler that `await`s html2canvas, then `await navigator.clipboard.write(...)`.** LOG logged `[screenshot] copied to clipboard`, Claude Code paste said "No image found in clipboard." Plausible cause (not measured): the `await` on html2canvas drops transient user activation before `clipboard.write` runs, browser silently no-ops the write.
  2. **Native `XMLSerializer` + SVG `<foreignObject>` data-URL `<img>` + canvas + `clipboard.write`.** Same log line, same empty clipboard. No dependency. Possible failure: SVG-as-image sandbox dropped foreignObject content silently (no `img.onerror`), produced a 476KB PNG (logged the size) that was either blank or rejected somewhere downstream.
  3. **Same SVG approach + cloning documentElement, stripping `<script>` tags, filling canvas with body background, charset utf-8 data URL, byte-count logging.** Logged 476474 bytes; Claude Code still saw nothing. Verified the JS-side blob had real content; the failure is downstream of `clipboard.write` resolving, where without a `pbpaste` reading we couldn't tell whether the OS clipboard received the bytes or Claude Code's reader was at fault.
  4. **`html2canvas` global + sync keydown handler + `Promise<Blob>` passed directly to `ClipboardItem` (no `await` between html2canvas and `clipboard.write`).** Matches the exact pattern the user has used successfully in other projects. Even with `e.repeat` guard + `activeElement` check + no log lines per the verbatim snippet — no log appeared on `i` press, no clipboard contents. Either the handler didn't fire (event listener didn't bind for some reason this codebase has and others don't) or it fired silently into a failure with no surfacing.

  Cross-cutting issues that haunted every attempt:
  - No measurement of the OS clipboard from inside the browser. `pbpaste -Prefer public.png > /tmp/x.png; ls -l /tmp/x.png; file /tmp/x.png` was the diagnostic that would have triaged "did JS write reach the OS" — never run.
  - Browser console disallowed in this session, so we couldn't see the actual exception (if any) from `clipboard.write`. The fault-surface diagnostic in `js-bridge.js` catches IIFE-stage errors but doesn't wrap arbitrary keydown handlers, so silent rejections vanished.
  - At least one assistant turn guessed Firefox-foreignObject-restrictions as the cause without evidence. The user wasn't using Firefox. Future sessions: do not guess browser quirks. Measure first.

  What this codebase has that others might not (none confirmed as cause): a Web Worker doing wasm FFI in the background, an Elm app rendering and patching DOM frequently, a large inline `<style>` block. None of these were proven to interact with `clipboard.write`.

  **Future-session checklist before implementing:**
  - Run the `pbpaste` diagnostic FIRST, after a single test write (e.g., a hardcoded 1×1 red PNG), to confirm the page's clipboard.write reaches the OS. If it doesn't, the screenshot-rendering path is irrelevant.
  - Wrap the keydown handler body in `try/catch` and write the actual error message to the LOG. Don't fire-and-forget the `clipboard.write` Promise.
  - If `pbpaste` shows the PNG IS in the clipboard but Claude Code says empty: the problem is Claude Code's clipboard reader, not the page. Stop touching the page.

## lua

ChoicePending propagation is complete across every handler-fire boundary: `PlayError::ChoicePending` (play), `ActivateError::ChoicePending` (activate), `CombatError::ChoicePending` (combat), `TurnError::ChoicePending` (turn-begin triggers). `build_game_table!`'s `choose_card` / `confirm` / `choose_player` / `choose_int` wrappers raise `Err(mlua::Error::external(ChoicePending))` (the `coroutine.yield` path was investigated and rejected — Lua blocks yield across C-call boundaries; `src/game/lua_api.rs:514-517` documents the decision). The `fire_*` family downcasts to `Result<(), ChoicePending>`; each subsystem lifts via `.map_err(*Error::ChoicePending)?`. The StepEngine catches every variant, rolls back the preview journal, surfaces a `HumanPrompt`, and re-fires the operation after the user's answer lands in `HumanReplayOracle.replay`. Open gap downstream: the StepEngine still uses `RandomOracle` for phase-advance triggers, so OnTurnBegin handlers that call `game.choose_*` get random answers rather than human prompts; swapping that to `HumanReplayOracle` is a separate slice.

## costs

- **SELF / SelfExile** (P.5) — **shipped for cast.** `CostSource::SelfExile` in cast paths at `src/game/play.rs:257,1155-1166` routes the played card → EXILE on resolution. SELF cost on *activated* abilities is the open piece — see `## activated abilities` Deferred.
- **Subtype filter on SACRIFICE** — `CostComponent.kind` filters by CardType today. A `subtypes` filter ("sacrifice a goblin") is the next gap; no current card needs it.
- **Variable X for spells in playability filter** — fixed. `can_pay_instant_cost` accepts is_x cost components (X=1 minimum affordability). `run.rs` X-pick caps X by the tightest binding resource (identity-matching hand size, deck size, GY size, board-creature count for sacrifice). Per RULES P.30 the AI picks `min: 1`, and the engine rejects `x_value = Some(0)` with `XBelowMinimum` unless the card sets `allow_x_zero = true`. Recast and turn-back-time stay filtered (SelfExile cost). Shift now has an `on_play` handler (X mill → move X attached between hosts); recast and turn-back-time remain handler-less.
- **GY → EXILE as HAND-payment substitute** — engine path implemented (`Card.gy_hand_substitute` flag, `PlayChoices.gy_hand_payment_ids`, sim AI integration via `find_gy_hand_substitutes` + `identity_matching_hand_count`, `NoHandPaymentForIdentity` gate). Unit-tested but not yet end-to-end confirmed by an EA round actually drafting and exploiting Clear View. May reveal AI-heuristic gaps (e.g., when to keep Clear Views in GY vs. when to spend them aggressively).

## types

- **Environment** (P.21) → BOARD with P.22 (one at a time, global) + P.23 (can't replace). Displacement question unresolved.
- **`same_sleeve` semantics not enforced.** RULES.md C.4 defines fused attachments (host + attached share one sleeve, can't be peeled off, leave play with the host). The schema flag `same_sleeve = true` is declared on `cards/APOPTOSIS.lua` and documented in LUA.md, but the loader doesn't read the field and no engine code references it. APOPTOSIS's "strip one attached card per turn → sacrifice host when bare" effect therefore strips itself, firing the sacrifice trigger one turn early. Minimum-viable wire: `Card.same_sleeve: bool` field + loader read + Lua-side card-view exposure so handlers can self-filter. Engine-enforced version (filter `attached_of` by default, couple host-movement, exclude from C.16 counting, decide OnDie behavior for fused attachments) requires the open design questions in the session notes.

## targeting

No engine concept of "what is legal to target." Every "target X" card today works because the handler builds the pool itself. Affects every removal/redirect/buff card with explicit targeting.

- **Targetability protection** — Reef Phantom's "tapped → untargetable" can't be enforced. Hexproof / shroud / "can't be targeted by opponents" all need this layer.
- **Multi-target / divided effects** — handlers can call `choose_card` multiple times but no API for "deal 3 damage divided as you choose among any number of targets." Single-choice multi-output patterns aren't expressible.
- **Target-validity recomputation** — if a target becomes illegal between cast and resolution (e.g., it left the board), the engine has no "fizzle if target is gone" check.

STATIC Phase 3 (restriction statics) partially overlaps here; the targeting infrastructure is its own slice.

AI side: `TargetIntent` (side-channel scoring hint per `game.set_intent`) catalog + wired-cards roster lives in `src/sim/README.md`.

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

- SACRIFICE / SELF cost components in activations — **engine-side partial 2026-06-20, not yet exercised end-to-end.** `activate_ability` accepts SACRIFICE components (validated against `ActivateChoices.sacrifice_ids` for count + kind + on-board + controller, mirrors `play.rs:697-730` cast-side) and SelfExile components (source moves BOARD → EXILE after the effect fires, mirrors P.5 cast-side routing). Unit tests pin both contracts in `src/game/play/activate.rs::tests`. **Downstream gap blocking end-to-end:** sim AI's `run_activation_pass` (`src/sim/run.rs:1608`) and the two human-driven `HumanAction::Activate` sites (`src/sim/run.rs:1027,1407`) both pass `ActivateChoices::default()` — SACRIFICE-cost activations return `WrongSacrificeCount` because no caller supplies sacrifice_ids. `HumanAction::Activate` needs a `sacrifice_ids: Vec<InstanceId>` field + UI plumbing on the human side; sim AI needs a heuristic to pick a sacrifice target (e.g., lowest-value creature on controller's board) on the AI side. Reincubator's activated, card 156's activated, and the ghost cycle's board-attached half are blocked on this downstream wiring. SELF-cost activations need no extra plumbing — the source is the implicit cost — so anything using only SelfExile (no SACRIFICE) is fully wired.
- Activations from non-BOARD zones — **engine-side wired 2026-06-21, corpus partially wired 2026-06-21.** `ActivatedAbility.from_zones: Vec<ActivationZone>` (default `[Board]`) declares the zones from which the ability can fire. `activate_ability` + `can_activate_with_x` gate on `state.iid_in_any_activation_zone(iid, controller, &from_zones)`. `enumerate_human_activations` walks board + hand + graveyard + exile + deck + attached-of-any-host so the UI surfaces non-BOARD activations. Unit tests pin the graveyard and attached cases in `src/game/play/activate.rs::choice_pending_tests`; corpus loader test pins ghost cycle + durian-elemental shape in `src/card/loader.rs::ghost_cycle_and_durian_load_with_zoned_activations`. Lua schema: `from_zones = "graveyard"` or `from_zones = {"board", "graveyard", "attached"}` on the activated entry. **Corpus wired**: blue/green/yellow/purple/red/pink ghost (each declares two activations — attached → tutor matching-color symbol to board, graveyard → tutor matching-color symbol to hand; both cost SELF exile), durian-elemental (graveyard activation: 1H + SELF exile, same rearrange effect as the on_turn_begin trigger). **Not yet wired in corpus**: Portable Bolt's portable rider needs an attach-from-spell path that doesn't exist — spells resolve to graveyard before on_play fires (`play.rs:569`). **Not verified end-to-end at runtime**: the loader-shape test proves the cards parse and declare the right zones; full activation through `activate_ability` from a graveyard / attached card during a live game has not been driven through a test or playthrough yet.
- **AI activation timing is asymmetric vs. human.** The sim AI auto-fires its activation pass at one fixed engine moment (post-combat in `step/combat.rs::step_activation_pass`, body in `run.rs::run_activation_pass`), firing the *first* eligible ability per board card. The human drives activations explicitly via `HumanAction::Activate { iid, ability_index, x }` at any moment during Main1 or Main2. This means UCT/MCTS never see activations as branching choices in their search trees — they fire automatically after the tree's plays resolve, outside the decision graph. Probe results that depend on activation sequencing (mostly the jewel cycle, vigilant-human, dark salamander's X cost) are not reflecting honest AI play. Fix: lift activations into Pattern B's candidate set so they're first-class actions; widen `enumerate_playable_in_hand` to `enumerate_priority_actions → Vec<Action>` where `Action = Play(iid) | Activate(iid, idx, x) | Pass`; change UCT/MCTS pick signatures to return `Action`; delete `run_activation_pass` entirely.

Per RULES A.5 activations resolve immediately and cannot be responded to. This is a deliberate deviation from MTG and is not on a "to fix" list.

## state-based actions (SBAs)

Not started. `combat.rs:463` has a `TODO(sbas)` marker. tsot does combat damage + death check in one atomic pass; MTG-style SBAs fire BETWEEN stack-item resolutions so "regenerate" / "prevent damage" responses can save a dying creature.

## sim AI strategic depth

These describe the heuristic AI's ceiling. UCT searches deeper at autobattle runtime, so heuristic-derived EA signals undersell what cards do in production play. The AI:

- **At most one creature per turn** (Pattern B). Multiple non-creatures per turn are allowed as long as the AI can afford their costs. No "play A, evaluate, then play B" planning beyond the priority-score tier ordering.
- **Attack policy is per-attacker myopic.** `select_attackers` walks attackers big-first and reserves the defender's clean-kill blockers for top threats, skips swings that die for nothing, and mirrors the defender's T2 gate for trade-up gating; reach-aware. What it still doesn't do: hold back to chump-block next turn, anticipate response-window instants (bitter-dawn, counterspell), or plan multi-turn lines. `cats-block-birds` / `rats-can't-block-cats` subtype rules are not modelled here either — the AI is mildly optimistic vs cats blocking flyers and mildly pessimistic vs rats facing cats.
- **No mulligan decision.** Engine deals first 5 cards as the hand, period. Real games have S.2/S.3 redraw. The sim never explores "this opening hand is unplayable."
- **No proactive instant timing in main phase.** Instants only fire from the response policy in R.1.a/R.1.b windows. Pre-emptive "cast surge before combat to enable a vigilance line" never happens.

## smaller items

- **P.8 attached → EXILE on host's death** — **partial.** Sacrifice path cascades correctly (`src/game/play.rs:877`). Combat-death path and source-movement death path still TODO (`src/game/play.rs:1351,1419`). Same RULES.md rule; coverage is partial across the death sites.
- **No set concept on cards.** A "set" in the MTG-Standard sense is a named release — a batch of cards that came out together (Innistrad, Theros, etc.). Cards currently have no `set` field declaring which release they belong to. Without it: no way to group cards by release, no way to compute "what's in Standard" (which is just a derived list of recent sets), no clean way to mark personal test cards / experimental drafts as not part of any released set. The `test` subtype filter and `is_variant` flag handle narrow exclusion cases but they're not a release-set primitive. Minimal shape: a `set` field on each card (`set = "core-1"`, etc.) and a derived legality predicate (`legal_sets = ["core-1", "core-2"]`) that `playable_pool` honors. Larger consequence: limited formats (draft, sealed) are structurally foreclosed — boosters are set-anchored pack compositions (rarity slots filled from the set's card list), and without sets there is no booster, and without boosters there is no draft pod or sealed pool.

## flagged for removal

- **C.14 (transparent-frame attachment restriction)** — flagged for full removal in both RULES.md and engine code. Rule currently states a `transparent`-frame card can only be attached or same-sleeved onto another transparent-frame host; removal would lift that restriction entirely. Engine enforcement sites (cast-time refusal in `src/game/play.rs`, the C.14 mention in cost-payment validation) plus the rule text in RULES.md C.14 and its cross-references in P.8 / P.26 / P.31 would all be touched. Not removed in this pass; tracked for a dedicated slice.

## design intents not yet encoded by a card

- **"Y - X" / two-variable cost-effect interplay** — the original Dark Salamander used `2Y - X` (Y = activation X-cost, X = source's effective X stat) so paying more on activation outscaled the source's own stats. Pattern got dropped when dark-salamander simplified to `2Y`. A future card should re-encode the two-variable-arithmetic idea: handler reads `game.x_value()` AND `game.card(self).x` (or other stat / state value) and combines them. Self-tension built into the math.

## EA / evolutionary deck search

Three biggest limitations on what conclusions the EA can support today.
Below these, smaller items.

### Biggest

1. **Population collapse → one run finds one attractor.** Within 15-20 generations a 50-pop run converges (observed: `mean=0.435 → 0.953`, `min=0.086 → 0.829`). The top-5 of a single run share 40+ of 50 cards — they are not independent samples, just five slot-variations on the same evolved deck. *Card-design conclusions need many runs at different `--seed` values, not many ranks from one run.* **Mitigation wired (2026-06-01):** Jaccard fitness penalty on tournament selection via `--diversity-alpha` (default 0.0 on bare CLI, 0.3 in `make evolve` / `evolve-shallow` / `evolve-deep` — override with `ALPHA=…`). `sim::diversity::selection_scores` computes `fitness - α · mean_jaccard_to_others` per generation; elitism still carries by raw fitness. Fitness sharing (Goldberg) is not wired — Jaccard penalty is the simpler relative; revisit if α-tuning plateaus. Bounded gauntlet growth uses `--promote-unmatched K` on `curate-baselines` (Makefile var `PROMOTE`, default 1) to promote inner-clustered unmatched champions as new baselines.

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
