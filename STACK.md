# tsot — Stack & Response Plan

> Three-phase plan for the response chain, priority, and stack integration.
> Resolves the `stack` theme in LIMITATIONS.md.

## Status (2026-05-30)

Phases 1 and 2 **done end-to-end**.

**Phase 1** delivered the chain, R.1.a window, priority loop, counter mechanic, and counterspell card.

**Phase 2** delivered R.1.b (attack-declaration window), engine introspection (`playable_responses`, `legal_counter_targets`), and explicit-target counter (`game.chain()`, `game.counter(target)`). The UX X.1/X.2/X.3 skip-logic and resolution event metadata moved to Phase 3 — they pair naturally with the Option B refactor (UI as outer driver) and have no live consumer until then.

What's actually running today:
- `StackItem`, `PriorityState`, `GameState.priority`, `SetPriorityState` journal variant.
- Priority primitives: `open_response_window`, `open_response_window_empty`, `pass_priority`, `respond_with`, `counter_top`, `counter_target`.
- `drive_window_to_close` — full priority/pass/resolve loop with response policy hook.
- R.1.a wired in `play_card`; R.1.b wired in `declare_attacker`. Casts during open window route to `respond_with`.
- Engine introspection: `pub playable_responses(player)`, `pub legal_counter_targets()`.
- Lua API: `game.counter_top()`, `game.counter(target)`, `game.chain()`, `game.legal_counter_targets()`.
- counterspell uses `counter_top`; explicit-target counter ready for DTST-creature once activated abilities land.
- `ChoiceOracle::respond_or_pass` policy hook (Option A — flagged for Option B refactor at the trait definition). RandomOracle uses `playable_responses` + threat-aware `would_die_soon` heuristic.
- Sim telemetry: `game.counter_top`, `game.counter`, `game.instant_response_played` counters; "instant responses (R.1)" row reads from action counts.

## Integration points (where stack touches existing code)

Stack and priority cut across more of the engine than `events` did.
Mapping the contact surfaces so future PRs know where to look:

### Engine methods that open a window (✅ done)

Per RULES.md R.1, only two events open a window: card-played and attack-declared. Block declarations resolve atomically (no window) — `on_block` / `on_blocked_by` fire inline. Per R.7, the active player gets priority first in every window.

| Site | Per rule | Status |
|---|---|---|
| `play_card` post-validation, pre-resolution | R.1.a | ✅ wired — opens a window, drives via `drive_window_to_close`, then resolves. |
| `declare_attacker` after attack recorded | R.1.b | ✅ wired — opens an empty-chain window, drives, then `on_attack` fires inline. |

The third site (block declaration) was previously planned as R.1.c but dropped: RULES doesn't open a window there, and the design intent (`on_block` resolves atomically) matches the no-stack-trigger principle.

### Triggered abilities stay inline (design ratified 2026-05-30)

Earlier drafts of this doc proposed moving triggered abilities onto the stack in Phase 2. That's been **dropped**: consequential triggers (`OnEnterBoard`, `OnPlay`, `OnAttack`, `OnBlock`, `OnBlockedBy`, `OnDie`) fire inline as part of resolving the action that caused them. The stack only carries casts and attack declarations, plus instants cast in response.

This kills the MTG "kill-with-priority-on-the-trigger" two-shot but keeps the cleaner "counter the spell / kill the attacker before its effect fires" windows. `fire_self_only` / `fire_with_partner` stay synchronous; no rework needed.

### Sim AI must learn to play instants in response

Today the sim plays one card during Main1 and never touches instants
during combat. Phase 1 only requires the sim to auto-pass every window
(no behavior change). Phase 2 with smart prompting (UX X.1, X.2) the sim
must:
- Know which instants in hand are playable in the current window
- Know which targets are legal (X-E.2)
- Decide: respond or pass

These hooks become the surface for AI lookahead (the recent
discussion about previewing attack decisions extends naturally to
"preview my response options"). Marked `TODO(stack-phase-2-sim)` in
`main.rs`.

### State-based actions (cross-cutting)

In MTG, state-based actions (SBAs) fire between stack items — e.g.,
"if any creature has lethal damage, it dies." tsot's current
`resolve_combat` does the damage tally + death check in one go. With
the stack, deaths should be checked between resolutions of stack items
(so a "regenerate" response card can save a dying creature mid-resolve).

This is partly orthogonal — could come with stack Phase 1 or later.
Marked `TODO(sbas)` in `resolve_combat`.

### Save/load and replay

Both already serialize `GameState`. Once `priority: Option<PriorityState>`
is populated, it serializes automatically. Replay's forward-apply needs
new `JournalEntry` variants for chain pushes / passes — `TODO(stack-journal)`.

---

## Fundamentals

These design decisions are baseline for all three phases.

**Stack item shape.** The response chain holds two kinds of items:

1. **Played card** — an instant or spell that's been cast but hasn't resolved. Goes on top of the chain when played as a response; resolves into its effect when popped.
2. **Triggered ability** — an A.1 trigger that's fired but hasn't resolved (e.g., mesopelagic-fish's "when this dies" return-from-graveyard). Has the source card, the trigger name, and the captured context.

```rust
enum StackItem {
    PlayedCard { card: InstanceId, controller: PlayerId },
    Trigger { source: InstanceId, name: TriggerName, ctx: TriggerContext },
}
```

**Priority state.** Tracked on GameState alongside the response chain:

```rust
struct PriorityState {
    chain: Vec<StackItem>,
    next_to_act: PlayerId,    // who has priority right now
    consecutive_passes: u8,   // 0, 1, or 2; 2 = top resolves or window closes
}
```

**Window-openers.** Per RULES.md R.1: a response window opens when (a) a card is played, or (b) an attack is declared. Outside these moments, actions and events resolve atomically. (An earlier draft of this doc proposed an R.1.c for block-declaration windows; that was dropped — blocks are atomic.)

**Active player goes first.** Per R.7. Every window starts with `next_to_act = active_player`.

**Resolution rule.** Two consecutive passes → top of chain resolves; reset pass counter. Window closes only when chain is empty AND both pass (R.6).

---

## Phase 1 — Foundation: chain, R.1.a window, counter mechanic ✅ done (2026-05-30)

Scope ballooned past the original "data structures only" plan because counterspell forced end-to-end wiring. Net result: the chain, R.1.a window, and a working counter mechanic all landed together.

**What shipped:**
- `StackItem::PlayedCard { card, controller, choices }` + `PriorityState { chain, next_to_act, consecutive_passes }` + `GameState.priority`.
- `SetPriorityState` journal variant — chain mutations round-trip through rollback + replay + save/load.
- Three primitives on `GameState`: `open_response_window`, `pass_priority`, `respond_with`. Plus `counter_top` for the counter mechanic.
- `drive_window_to_close(ctx)` — loop that asks `oracle.respond_or_pass(state, next)` at each handoff, routes Respond decisions back into `play_card` (which sees `priority.is_some()` and calls `respond_with` instead of opening a nested window).
- R.1.a wired in `play_card` post-cost, pre-resolution. `resolve_played_card` extracted as the resolution body.
- `ChoiceOracle::respond_or_pass` trait method (default impl: Pass). `RandomOracle` overrides with a threat-aware policy: 95% counter when fast death is imminent (`would_die_soon` heuristic), 25% otherwise.
- `game.counter_top()` Lua API. `cards/counterspell.lua` (free blue instant, symbol ꩜) uses it.
- Sim telemetry: `game.counter_top`, `game.instant_response_played` action counters; "instant responses (R.1)" pending-mechanics row reads live.

**Design decisions ratified in this phase:**
- **Consequential triggers stay inline.** `OnEnterBoard`, `OnPlay`, `OnAttack`, `OnBlock`, `OnBlockedBy`, `OnDie` fire as part of resolving the action that caused them; no `StackItem::Trigger` variant. Kills the MTG kill-with-priority-on-trigger play but keeps the cleaner "counter the spell / kill the attacker before its effect" windows.
- **No R.1.c.** Block declarations are atomic per RULES R.1; `on_block` / `on_blocked_by` fire inline.
- **Active player first in every window** per R.7. The `open_response_window` API takes no `first` parameter — derived from `state.active_player`.
- **HAND payments are refunded on counter.** Mill / Graveyard cost is paid at announce (not refunded). Matches MTG.

**Sim shape after Phase 1:** with the threat-aware policy, ~1.1 instant responses per player per game across 1000 games, with ~33% of responses successfully countering something (rest are caught in counter-battles).

**Still pending from the original Phase 1 plan:**
- R.1.b window in `declare_attacker` (TODO marker at `combat.rs:135`). Same shape as R.1.a; small wiring patch.

---

## Phase 2 (re-scoped 2026-05-30) — Introspection + explicit targeting + resolution metadata

The original Phase 2 was built around "triggered abilities on the stack." That assumption is dead per the inline-triggers ratification. The counter mechanic landed in Phase 1. The UX X.1/X.2/X.3 skip-logic moved to Phase 3 (deferred 2026-05-30) — it needs the per-card target system and is more naturally bundled with the Option B refactor when the UI driver takes over.

**Goal:** the engine exposes the queries a UI needs and the counter mechanic supports arbitrary chain targets.

**Scope:**
- **R.1.b window** in `declare_attacker` — ✅ done 2026-05-30. Defender's response opportunity before `on_attack` fires. Same shape as R.1.a.
- **Engine introspection** (X-E.1, X-E.2) — ✅ done 2026-05-30:
  - `pub fn playable_responses(player) → Vec<InstanceId>` on `GameState`. Filter today: `kind == Spell && timing == Instant && cost.amount == 0`. RandomOracle's policy now calls this instead of inlining the filter.
  - `pub fn legal_counter_targets() → Vec<InstanceId>` on `GameState`. Empty if no window. Card-agnostic for the counter case; a generalized `legal_targets(card, state)` is deferred until more target-shaped effects exist in the corpus.
- **Explicit-target counter** — ✅ done 2026-05-30:
  - `pub fn counter_target(target) → Option<StackItem>` on `GameState`. Same priority/pass semantics as `counter_top`.
  - `game.chain()` Lua API — returns `[{card, controller, kind}, ...]`.
  - `game.counter(target_iid)` Lua API — explicit-target counter.
  - `game.legal_counter_targets()` Lua API — convenience for handler-side target pools.
  - `counter_top` stays as the convenience for "spell directly under me" cards (counterspell).

**Out (deferred to Phase 3):**
- **UX X.1 / X.2 / X.3 skip-logic.** X.1 is de facto in the response policy (returns Pass when no candidates); lifting to engine level pairs naturally with the Option B refactor (when the engine driver becomes the right place for it). X.2 needs a generalized `legal_targets` system. X.3 is UI affordance work — better when there's an actual UI consuming it.
- **Resolution event metadata.** Same deferral logic — the consumer (UI / smarter AI) lands in Phase 3.

**Cards unlocked by Phase 2:**
- **DTST-creature** — "Tap: counter target card on the stack." The counter API is ready; needs the activated-ability (Tap-cost) system before the handler can be written.

**Deliverable:**
- ✅ Integration test `lua_chain_and_counter_target_apis_remove_specific_item` — Lua fixture inspects `game.chain()`, picks a target, calls `game.counter(target)`, asserts target removed.
- ✅ `playable_responses` + `legal_counter_targets` exposed as `pub` methods on `GameState`, used by `RandomOracle::respond_or_pass` and available to future UI drivers.

---

## Phase 3 — UX baseline + the Option B refactor + UI hooks

Round out X.1–X.7 and migrate from Option A (oracle holds the response policy) to Option B (caller drives the priority loop). Visualization API for a UI to consume.

**Goal:** UX baseline (X.1–X.7) fully met. The engine exposes everything a UI needs to render and drive priority decisions, and `play_card` becomes a pure-announce method with the sim/UI as the outer driver. Picks up the X.1/X.2/X.3 skip-logic and resolution metadata items deferred from Phase 2 — they make sense here because the UI is the natural consumer.

**Scope (in):**
- **Option B refactor.** `play_card` no longer drives `drive_window_to_close` internally; it just announces (open window or `respond_with`) and returns. The caller runs the loop, querying `pass_priority` / `resolve_stack_item` and consulting whatever policy module they own. `ChoiceOracle::respond_or_pass` retires; the policy moves into a separate `ResponsePolicy` (sim) or UI driver. Matches how human play actually flows.
- **UX X.1 — auto-pass when nothing playable.** Engine-level skip using `playable_responses`. Today the policy does it; lifting to engine means non-Random oracles get it free.
- **UX X.2 — auto-pass when no legal target.** Needs a generalized `legal_targets(card, state)` system — today only `legal_counter_targets` exists. Card-side: each effect declares its target rule; engine queries it before prompting.
- **UX X.3 — opponent-considering-response marker.** Priority-handoff event carries `cause` field; UI shows "Opponent has N playables, considering response to {cause}."
- **Resolution event metadata (X.7).** Each resolved item carries `unopposed: bool` (responder had nothing playable) and `declined: bool` (could have responded but chose to pass). Powers "they let it through" UI affordance and smarter-AI inference.
- **Hold-priority (X.5):** active player can explicitly retain priority after their own play to chain their own response (e.g., draw-two into silent-murder using a drawn card).
- **Pre-declared responses (X.6):** `register_response_intent(player, condition, action)` — engine consults intent registry before prompting.
- **Timer (X.4):** per-window timeout. On expiry, auto-pass. Configurable.
- **Visualization API:**
  - `peek_chain() → Vec<StackItemView>` — read-only view of current chain for rendering.
  - `priority_holder() → Option<PlayerId>`.
  - `window_cause() → Option<WindowCause>`.
- **Test coverage:** combat-trick scenarios from B + R sections. Cards held for response.

**Out:**
- Multiplayer (still 1v1).
- Time-travel / replay (separate concern).

**Cards working after Phase 3:** every card with timing-flexible behavior runs at full interactivity. The "answer-rich, tempo-driven" pitch from the README is realizable.

**Deliverable:**
- Full UX X.1–X.7 compliance.
- A CLI playthrough tool that walks through a scripted game, exercising every response window pattern.

---

## Cross-cutting design questions

1. **Triggered abilities on the stack — yes or no?** Resolved 2026-05-30: **no.** Consequential triggers (`OnEnterBoard`, `OnPlay`, `OnAttack`, `OnBlock`, `OnBlockedBy`, `OnDie`) fire inline as part of resolving the action that caused them. Hearthstone-side of the spectrum, not MTG. Affects card design: stinging-bee's damage-trigger is interrupt-free; the stack only sees the cast / attack declaration / instant-in-response.
2. **State-based actions.** When a creature has lethal damage, does it die *between* stack items (SBA-style, immediate, interrupt-free), or as a queued event? MTG has SBAs. Tsot probably should too. TODO marker exists at `combat.rs:321`. Still open.
3. **Priority during cost payment.** Costs are atomic; no priority window between sub-payments. Implementation already enforces this (HAND payment selection happens up-front; mill/graveyard payments execute before the window opens). Should be ratified in RULES.
4. **Multiple triggers from one event.** APNAP or simpler? Open. Tsot has no card today that fires multiple triggers off one event, so deferred until one exists.
5. **Cancelling vs. countering.** Phase 1 implements `counter_top` (remove without resolving). "Fizzle" (target missing) is implicit — when `counter` removes a target, the dependent spell would naturally fizzle if it relied on the missing item. "Redirect" hasn't surfaced as a need. Phase 2 adds explicit-target `game.counter(target)`.
6. **Cost to play instants from outside HAND.** Stream-of-thought + future "play from graveyard" effects. The `play_card` validation needs to know the source zone. Architectural — still open.

---

## How this fits with LIMITATIONS.md's four themes

- **stack** — this whole document.
- **events** — LUA.md. STACK Phase 2 depends on LUA Phase 2/3 (triggers fire via Lua handlers).
- **costs** — STACK doesn't change cost machinery; cost-modifier interactions happen separately.
- **types** — STACK Phase 1 needs instant + spell types to play correctly. Depends on `types` slice landing or coexisting.

---

## Interleaving with LUA.md

| Slice | Lands With |
|---|---|
| LUA Phase 1 | independent — events fire but no responses possible |
| LUA Phase 2 + STACK Phase 1 | basic interactivity: cards' handlers run; instants cast as responses work (counterspell live in sim) |
| STACK Phase 2 | introspection, explicit-target counter, X.1–X.3 UX — independent of LUA after this point |
| STACK Phase 3 | UX polish + Option B refactor; fully independent of LUA |

The old "LUA Phase 3 + STACK Phase 2 share the trigger-as-stack-item interface" row is gone — that interface doesn't exist (inline-triggers ratified). STACK Phase 2+ no longer depends on LUA progress.

---

## Open rule extensions to ratify in RULES.md

Still open after Phase 1:

- **B.x** (new): state-based actions — creatures die between stack items (or queue, depending on the SBA decision). TODO marker at `combat.rs:321`.
- **Counter** definition: an effect that removes a stack item without resolving its effect. Phase 1 implements "counter the top" (`game.counter_top`); Phase 2 adds "counter a specific target." Either way the rule definition is the same.
- **APNAP** (if adopted): priority and trigger-ordering rule for simultaneous events. Not yet relevant in the corpus — defer until a card forces the decision.
- **Cost-payment atomicity**: payments resolve without intervening priority. Implementation already enforces this; needs to be ratified in RULES so it's binding.

Closed in Phase 1:
- ~~R.1.c block-declaration clause~~ — dropped; block declarations are atomic per existing R.1.
- ~~"Triggered abilities on the stack" rule~~ — would have required a new clause; inline-triggers ruling means no rule change needed.
