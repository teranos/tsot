# tsot — Stack & Response Plan

> Three-phase plan for the response chain, priority, and stack integration.
> Resolves the `stack` theme in LIMITATIONS.md.

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

**Window-openers.** Per a rule extension ratified by this document:

> **R.1** A response window opens when: (a) a card is played, (b) an attack is declared, **(c) blocks are declared**.

Block-declaration as a third opener was discussed in design conversation but never written. This document ratifies it.

**Active player goes first.** Per R.7. Every window starts with `next_to_act = active_player`.

**Resolution rule.** Two consecutive passes → top of chain resolves; reset pass counter. Window closes only when chain is empty AND both pass (R.6).

---

## Phase 1 — Foundation: chain data and manual responses

Smallest piece that's actually a stack. No Lua integration; triggered abilities still resolve immediately.

**Goal:** the data structure and resolution loop exist. A player can play an instant as a response, both players can pass, the top of chain resolves correctly.

**Scope (in):**
- **`ResponseChain` field** on `GameState` (or a new `PriorityState` substruct holding chain + priority bookkeeping).
- **Three window-openers** wired:
  - In `play_card` (post-validation, pre-resolution): open R.1.a window.
  - In `declare_attacker` (after the attack is recorded): open R.1.b window.
  - In `declare_blocker` (after each block is assigned, or once after `confirm_blocks` — pick one): open R.1.c window.
- **Player actions:**
  - `submit_response(card_id)` — adds a `PlayedCard` to the chain top; the card type must be INSTANT or the timing must allow it (per R.3 + C.6). Resets pass counter.
  - `pass_priority()` — increments pass counter; passes priority to the other player.
- **Resolution loop:**
  - On 2 consecutive passes with non-empty chain: pop top, resolve its effect, reset passes, hand priority to active player (R.7).
  - On 2 consecutive passes with empty chain: window closes, return control to the gameplay loop.
- **Headless sim mode:** both players auto-pass. The sim doesn't try to play instants in this phase (would just `pass_priority()` each prompt).
- **Triggered abilities still resolve immediately**, not on stack — kept simple until Phase 2.
- **No counter mechanic yet.**

**Out (deferred):**
- Triggered abilities on the stack.
- Counter effect type.
- Engine introspection (`playable_responses`, `legal_targets`).
- UX X.1–X.7 smart prompting (always prompts both players in this phase, even if they have nothing).
- Pre-declared responses.

**Cards working after Phase 1:**
- Any instant played as a *manual* response. silent-murder mid-attack, falter on a hand-cost play, draw-two during opponent's main phase. Effects resolve correctly even though their handlers run in old-style (engine-handles, no Lua execution).

**Deliverable:**
- `cargo test` includes scenarios:
  - Player A plays a creature → response window opens → B plays silent-murder targeting it → both pass → silent-murder resolves → creature dies.
  - Player A declares attack → window opens → B passes → A passes → window closes → combat resolves.
- The simulator runs with the new state machine but `pending mechanics` row "instant responses" still shows 0.0 (sim doesn't fire any).

---

## Phase 2 — Triggered abilities on the stack + smart prompting

The interesting layer. Where the game becomes interactive.

**Goal:** triggered abilities go on the stack rather than resolving immediately. Players can respond to triggers before they resolve. The UX X.1–X.3 smart-skip patterns work.

**Scope (in):**
- **Triggered abilities go on the stack.** When LUA fires a trigger (per LUA.md Phase 2/3), the engine wraps it as a `StackItem::Trigger` and pushes onto the chain instead of running the handler immediately. A window opens after.
- **Counter effect type.** Define `counter` as an effect that removes the *top* (or *target*) item from the chain without resolving it. DTST-creature's "Tap: counter target card on the stack" becomes meaningful.
- **Engine introspection API** for the UI (X-E.1, X-E.2):
  - `playable_responses(player) → Vec<InstanceId>` — instants in player's hand whose timing and cost can be paid right now.
  - `legal_targets(card, state) → Vec<Target>` — for a card being played or considered, the set of legal targets in the current state.
- **UX X.1, X.2, X.3 wired:**
  - **X.1:** if `playable_responses(player)` is empty, the engine auto-passes for that player. No prompt sent to UI.
  - **X.2:** if a player has playable instants but none have a legal target, the engine auto-passes. (Per card. A card that has a legal target proceeds normally.)
  - **X.3:** when one side is prompted, the active player sees a marker: "Opponent has N playable instants, considering response to {cause}."
- **Resolution event metadata** (partial X.7): the resolved item carries whether it resolved unopposed (no playable responses existed) or after declined (responses were possible but the responder passed).

**Out (deferred to Phase 3):**
- Hold-priority (X.5).
- Pre-declared responses (X.6).
- Tight timer (X.4).
- Full visualization API.
- Counter-counter wars beyond the immediate "counter target" mechanic.

**Cards working after Phase 2:**
- mesopelagic-fish death-trigger goes on stack → opponent can falter it before it resolves → if successful, fish dies but no card returns from graveyard. Real interactive play.
- stinging-bee's damage-trigger goes on stack → opponent can respond with an instant (e.g., a future "prevent next damage" card).
- DTST-creature's counter ability works on any item on the stack.
- Squirrel-overrun's attack-trigger + on_blocked_by trigger both go through proper stack resolution.

**Deliverable:**
- The simulator's headless oracle handles "would you like to respond?" prompts. With X.1 + X.2 skip-logic, ~90% of prompts auto-pass.
- Integration test: full mesopelagic-fish + falter scenario. Verify the falter cancels the death-return.
- `cargo run` pending-mechanics row "instant responses" shows non-zero counts.

---

## Phase 3 — Full UX baseline and polish

Round out X.4, X.5, X.6, X.7. Visualization for a UI to consume.

**Goal:** UX baseline (X.1–X.7) fully met. The engine exposes everything a UI needs to render and drive priority decisions.

**Scope (in):**
- **Hold-priority (X.5):** active player can explicitly retain priority after their own play to chain their own response (e.g., draw-two into silent-murder using a drawn card).
- **Pre-declared responses (X.6):** `register_response_intent(player, condition, action)` — engine consults intent registry before prompting. If a registered condition fires, attempt the action automatically.
- **Timer (X.4):** per-window timeout. On expiry, auto-pass. Configurable.
- **Resolution metadata (X.7) complete:** events carry `cause`, `responder_options` (what they could have done), `responder_action` (what they did or didn't do).
- **Visualization API:**
  - `peek_chain() → Vec<StackItemView>` — read-only view of current chain for rendering.
  - `priority_holder() → Option<PlayerId>`.
  - `window_cause() → Option<WindowCause>`.
- **Test coverage:** combat-trick scenarios from B + R sections. Counter-the-counter wars (recursive R.4). Cards held for response.

**Out:**
- Multiplayer (still 1v1).
- Time-travel / replay (separate concern).

**Cards working after Phase 3:** every card with timing-flexible behavior runs at full interactivity. The "answer-rich, tempo-driven" pitch from the README is realizable.

**Deliverable:**
- Full UX X.1–X.7 compliance.
- A CLI playthrough tool that walks through a scripted game, exercising every response window pattern.

---

## Cross-cutting design questions to resolve

These come up across phases and need decisions early.

1. **Triggered abilities on the stack — yes or no?** Phase 2 assumes yes. MTG-style. Alternative is Hearthstone-style (triggers fire and resolve atomically, no interrupts). Affects card design fundamentally: stinging-bee's damage-trigger being interruptable changes its strategic role.
2. **State-based actions.** When a creature has lethal damage, does it die *between* stack items (SBA-style, immediate, interrupt-free), or as a queued event? MTG has SBAs. Tsot probably should too — but it's a decision.
3. **Priority during cost payment.** When paying `1 hand` + `2 mill`, can opponent respond between picking the hand card and milling? Almost certainly no — costs are atomic. Worth pinning in rules.
4. **Multiple triggers from one event.** If a damage event triggers stinging-bee's lockdown AND a future card's "when damaged" reaction, what's the resolution order? MTG: APNAP (active player's triggers first, non-active player chooses order within each set). Tsot can adopt this or pick something simpler.
5. **Cancelling vs. countering.** Are there distinct effect types: "fizzle" (target missing, resolves to nothing), "counter" (removed from stack), "redirect" (target changes mid-resolve)? Phase 2 introduces counter; others can come later.
6. **Cost to play instants from outside HAND.** Stream-of-thought + future "play this from graveyard" effects. The play_card validation needs to know what zone the card is currently in. Architectural — affects when `submit_response` validates.

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
| LUA Phase 2 + STACK Phase 1 | basic interactivity: cards' handlers run, manual instant responses work |
| LUA Phase 3 + STACK Phase 2 | full interactivity: triggers on stack, smart prompting |
| STACK Phase 3 | UX polish; independent of LUA after Phase 3 |

LUA and STACK Phase 2s should be designed and tested together. They share the trigger-as-stack-item interface.

---

## Open rule extensions to ratify in RULES.md alongside this work

- **R.1.c** (new clause): block-declaration opens a response window.
- **B.x** (new): state-based actions — creatures die between stack items (or queue, depending on the SBA decision).
- **Counter** definition: an effect that removes a stack item without resolving its effect. Targeted or untargeted.
- **APNAP** (if adopted): priority and trigger-ordering rule for simultaneous events.
- **Cost-payment atomicity** statement: payments resolve without intervening priority.

These should be ratified in RULES.md before Phase 1 lands, so the implementation has rules to encode.
