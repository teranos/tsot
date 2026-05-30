# tsot — Static Abilities Plan

> Four-phase plan for continuous effects: anthems, keyword grants, restrictions, and replacements.
> Companion to STACK.md (which handles cast-and-response timing — orthogonal to statics).

## Status (2026-05-30)

Nothing wired yet. The existing `Modifier` system (`StatBoost`, `GainsFlying`) is point-in-time applied and persists on the modified card — it's what `add_modifier` and `effective_stats` already use today. That's the substrate Phase 1 builds on, but it isn't itself "static" in the continuous sense.

What this doc plans: a continuous-effect engine where a source card on the BOARD broadcasts effects to qualifying targets, and the effects evaporate the moment the source leaves the board.

## Fundamentals

### What a static ability is

> **A.2** A card may have static abilities. A static ability is a continuous effect that applies while the source card is on the BOARD.

A static ability has:
- A **source** — the card whose presence on the BOARD activates the effect.
- A **predicate** — which other cards (or the source itself) receive the effect.
- An **effect** — what changes about the receivers (stat boost, keyword grant, restriction, replacement).

When the source leaves the BOARD, every effect produced by that source must immediately stop applying. No persistent modifier on the receivers.

### Lazy vs eager re-evaluation

Two architectural choices for **when** statics get recomputed:

- **Lazy** — recompute on every read. `effective_stats(iid)` and `has_keyword(iid, kw)` iterate all on-board static sources, run each predicate against the target, and combine the resulting effects in-place. No cache, no invalidation. Always correct.
- **Eager** — recompute on state changes (board mutations, damage, etc.) and cache results in a per-card `static_modifiers` vec. Effective_stats reads the cache. Cheaper per read but every mutation triggers a pass.

**Decision (2026-05-30): lazy.** Simpler model, no invalidation bugs, and the cost is bounded — 1v1 with ~10 on-board cards and ≤2 statics is ~20 predicate calls per stat read. If sim profiling shows it's the bottleneck, a per-call cache or eager-with-invalidation can land as a Phase 1.5 optimization without changing the Lua surface.

### Effect layering (the "kinds" of static)

Phase 1 handles **stat modifiers** only. Other kinds are separate phases because they hook different code paths:

| Kind | Source-side declaration | Where the engine reads it |
|---|---|---|
| **Stat modifier** | `static.modifier = {x = +1, y = +1}` | `effective_stats` |
| **Keyword grant** | `static.modifier = {keyword = "flying"}` | `has_keyword` |
| **Restriction** | `static.restriction = "cannot_attack"` | `declare_attacker`, `declare_blocker`, etc. |
| **Replacement** | `static.replace = function(event) end` | Event dispatch (intercepts before resolution) |

The first two share the same evaluation site (read-time recomputation). The third and fourth are categorically different — they intercept actions, not modify reads.

## Integration points (where statics touch existing code)

### `effective_stats` becomes static-aware (Phase 1)

Today: reads `printed + card.modifiers` and returns `(x, y)`. Phase 1 adds a third source: every on-board card's static, when the static is a stat-modifier kind and its predicate returns true for this card.

```rust
pub fn effective_stats(&self, iid: &InstanceId) -> (i32, i32) {
    let inst = match self.card_pool.get(iid) { Some(i) => i, None => return (0, 0) };
    let (mut x, mut y) = inst.card.stats.map(|s| (s.x, s.y)).unwrap_or((0, 0));
    // Stored modifiers (today)
    for m in &inst.modifiers {
        if let Modifier::StatBoost { x: dx, y: dy } = m { x += dx; y += dy; }
    }
    // Phase 1 addition: live statics from all on-board sources
    for source_iid in self.all_on_board_iids() {
        if let Some((dx, dy)) = self.evaluate_static_stat_modifier(source_iid, iid) {
            x += dx; y += dy;
        }
    }
    (x, y)
}
```

`evaluate_static_stat_modifier(source_iid, target_iid) -> Option<(i32, i32)>` calls the source card's `static.predicate` with the target as the candidate. If predicate returns true and `static.modifier` is a stat modifier, returns the deltas. Otherwise None.

### `has_keyword` becomes static-aware (Phase 2)

Same pattern as `effective_stats`. Today reads `printed + modifiers`. Phase 2 adds the static source iteration for keyword grants.

### `declare_attacker` / `declare_blocker` consult restrictions (Phase 3)

Before validating an attack or block declaration, check whether any on-board static prevents it. New API: `is_attack_restricted(attacker)`, `is_block_restricted(blocker, attacker)`. Returns true if any static's predicate matches AND the restriction kind applies.

### Event dispatch grows replacement support (Phase 4)

Before `fire_self_only` resolves an event, check if any static wants to replace it. The replacement handler returns a transformed event (or absorbs it). Event resolution proceeds with the replacement.

### Save / load + replay

Statics are derived from card data, not from game state — they live in the `Card.static` field which is read-only after card-registry load. No new journal entries needed; statics auto-apply on the deserialized state because their source cards are on the deserialized board.

### Sim AI implications

The threat-aware policy in `RandomOracle` calls `effective_stats` and would-die-soon. Both already handle the existing modifier system. With Phase 1, the same calls automatically pick up anthem boosts — anthem'd creatures count more toward `incoming_damage`. The AI gets smarter for free.

For Phase 3 (restrictions), the block policy needs to know "can this attacker be blocked at all?" before assigning blockers. `pick_blocks` would add a pre-filter.

---

## Phase 1 — Continuous stat modifiers

The smallest meaningful slice. Anthem-style buffs.

**Goal:** a card on the BOARD broadcasts `+X/+Y` (or `-X/-Y`) to other cards matching its predicate. Effect appears the moment the source enters BOARD, disappears the moment it leaves.

**Scope (in):**
- **Lua surface:** new `static` field on cards, declarative shape:
  ```lua
  static = {
    affects = {
      subtypes = {"human"},     -- (any-of) candidate has at least one
      colors = nil,             -- (any-of) candidate has at least one; nil = any
      controller = "owner",     -- "owner" (= source's controller), "opponent", or nil for any
      exclude_self = true,      -- candidate is not the source itself
    },
    modifier = {x = 1, y = 1},  -- stat boost only in Phase 1
  }
  ```
  Multiple cards can have statics; multiple statics stack on one target.
- **Why declarative and not function-based:** function predicates would need `&Lua` threaded through every `effective_stats` caller (combat math, AI policies, attack heuristics) — invasive cascade. The declarative shape covers every anthem in the corpus (subtype + controller filters) and keeps `effective_stats` Lua-free. Function-based escape hatch can land in Phase 2 if a card needs predicates the declarative shape can't express.
- **Engine evaluation:** `evaluate_static_stat_modifier(source_iid, target_iid) → Option<(i32, i32)>` in `state.rs`. Evaluates `affects` against the candidate using Rust-side field access (no Lua call). Returns the modifier on match.
- **`effective_stats` integration:** loops over all on-board cards, accumulates static stat deltas. Self-evaluation skipped via `exclude_self` (most anthems exclude self).
- **Cards wired:**
  - `battle-captain`: "Other humans you control get +1/+1."
  - `goblin-warlord`: "Other goblins you control get +1/+0."
  - (`hydra` left on its ETB-modifier path — already works.)
- **Tests:**
  - Anthem source enters → matching candidates get the boost via `effective_stats`.
  - Anthem source leaves (graveyard / exile) → boost disappears.
  - Two anthems stacking on one creature → both contribute.
  - Anthem's predicate excludes non-matching subtypes.
  - Anthem doesn't affect the source itself (when predicate excludes self).

**Out (deferred):**
- Keyword grants (Phase 2).
- Restrictions (Phase 3).
- Replacements (Phase 4).
- Conditional re-evaluation triggers beyond board membership (e.g., "while damaged" — needs damage-change recompute).
- Performance optimization (eager + cached evaluation).

**Cards working after Phase 1:**
- battle-captain, goblin-warlord — their anthem text becomes real.
- Any future "+X/+Y to creatures matching predicate" card.

**Deliverable:**
- `cargo test` includes the five anthem scenarios.
- The sim's threat-aware policy automatically picks up anthems (no policy code changes).
- `effective_stats` performance: documented overhead measurement (10× sim run before/after, expect <2× slowdown for current corpus).

---

## Phase 2 — Keyword grants

### Status (2026-05-30)

**Phase 2 complete.** Pieces landed:

1. ✅ `StaticDef.modifier_keyword: Option<String>` field + parser reads `modifier.keyword` from Lua.
2. ✅ `GameState::evaluate_static_keyword_grant(source, target)` + `has_static_keyword(iid, kw)` mirror the Phase 1 stat-modifier iteration via a shared `static_def_if_matches` helper.
3. ✅ `GameState::has_keyword(iid, kw)` combines intrinsic (printed + modifiers) with static-granted; all call sites in main.rs and combat.rs migrated. `CardInstance::has_keyword` stays as the intrinsic-only check for places without state access.
4. ✅ First wired card: goblin-warchief — *"Other goblins you control get +1/+1 and have haste."* Combined stat + keyword on one static.
5. ✅ `StaticAffects.scope = "attached_host"` scope + `static_source_iids` iteration includes cards attached to on-board hosts.
6. ✅ Companion-bird wired with AttachedHost scope.
7. ✅ State-reading predicates: `StaticCondition` enum (declarative — not Lua escape hatch). Variants: `OwnerGraveyardSize { min }` and `OwnerGraveyardNonCreatures { min }`. New variants added as cards need them.
8. ✅ `StaticScope::SourceOnly` — for "this creature has [keyword] when [condition]" style cards.
9. ✅ `StaticAffects.kind: Option<CardType>` — lets cards say "creatures you control" without subtype enumeration.
10. ✅ Ossuary wired: graveyard-threshold + creature-kind filter + stat+keyword on one static. All three Phase 2 capabilities exercised together. Win rate 0.58 in 30-game smoke.
11. ✅ Wandering-wizard wired: SourceOnly + OwnerGraveyardNonCreatures predicate. Win rate 0.53.

Phase 2 unit-test coverage: keyword grant; AttachedHost scope (grants to host, no grants when unattached); condition gate (block then trigger on threshold); non-creature count (correctly excludes creature cards); SourceOnly scope (targets only the source).

### Goal

Statics that grant keywords (flying, vigilance, defender, etc.) to qualifying cards.

### Scope (in)

- Extend Phase 1 `modifier` shape to include `{keyword = "flying"}` variant — ✅ landed (`StaticDef.modifier_keyword`).
- `has_keyword(iid, keyword)` iterates on-board static sources the same way `effective_stats` does — ✅ landed via `GameState::has_keyword`.
- State-reading predicate (e.g., "owner.graveyard.len() >= 5") — ✅ landed as `StaticCondition` enum.
- AttachedHost scope (companion-bird) — ✅ landed.
- SourceOnly scope (wandering-wizard) — ✅ landed.
- Kind filter ("creatures you control" without enumerating subtypes) — ✅ landed.

### Out

- Removing keywords (Phase 3 territory — restriction-flavored).

---

## Phase 3 — Restriction statics

### Status (2026-05-30)

**Phase 3 complete** for the two restriction types the corpus needs.

1. ✅ `StaticDef.restrictions: Vec<Restriction>` (one static can carry multiple restrictions; flesh-eating-plant uses two on one static).
2. ✅ `Restriction::CannotAttack` — checked in `declare_attacker`. New error `CombatError::AttackerForbiddenByRestriction`. Sim's `eligible_attackers` filters it out.
3. ✅ `Restriction::CannotBeCostPaid` — `resolve_hand_payment` filters the candidate pool; `play_card` validates explicit payments. New error `PlayError::HandPaymentForbidden`.
4. ✅ `GameState::has_restriction(iid, restriction)` mirrors `has_static_keyword`'s iteration.
5. ✅ Flesh-eating-plant wired (static with both restrictions, controller = opponent, subtype = insect). SACRIFICE cost source (P.16) is now routable through `play_card`, so the plant lands on BOARD via normal play. Sim plays it ~0.17 times per game per side.
6. ✅ 2 unit tests: restriction propagates to opponent insects (own insects unaffected); `declare_attacker` returns `AttackerForbiddenByRestriction` for a restricted attacker.

### Out (deferred)

- "Cannot be played" / "Cannot be cast" — no corpus card needs this. The stripped "you cannot cast this card" lines on jewels/ossuary became permissive when artifacts became routable; there's no current static asking for cast-time restriction.
- "Cannot be blocked" as a restriction — handled today by the `unblockable` keyword (intrinsic). A static-granted unblockable could be expressed as `modifier.keyword = "unblockable"` and the existing combat machinery already consults `has_keyword`.

---

## Phase 4 — Replacement effects

**Goal:** statics that intercept events and transform them. "If this creature would die, exile it instead." "If you would draw a card, draw two instead."

**Scope (in):**
- New `static.replace` Lua field: a function taking the original event and returning either a transformed event or nil (absorb).
- Engine event dispatch consults all on-board replacement statics before the event resolves. First match (with conflict resolution rules — APNAP?) wins.
- Cards:
  - (No current corpus card uses this; landing on speculative demand.)

**Out:**
- Stacking replacements (one replacement replacing another's output). Phase 5 territory.

---

## Cross-cutting design questions

1. **Self-targeting predicates.** Anthems typically exclude self. Is the default "predicate sees self and excludes manually" (current Phase 1 plan) or "engine auto-excludes self from candidates"? Current plan: manual exclusion — explicit > magical.
2. **Order of evaluation when multiple statics affect one card.** Stat modifiers commute (`(+1/+1) + (+2/+0) = (+3/+1)`), so order doesn't matter for Phase 1. For Phase 4 (replacements), APNAP-style ordering is needed.
3. **Static read of own-board state.** Can a static's predicate call `game.zones(self.owner).board` to count things? Yes — predicates run in the same scope as event handlers. Cost: a self-referential query is fine; deeply recursive ones (predicate that triggers state changes) is undefined behavior. RULES should ratify "predicates are read-only."
4. **Static dependency cycles.** Anthem A reads "+1/+1 if there's an anthem on the board" — does A apply to itself? With lazy evaluation, this could recurse. Phase 1: detect and break cycles at depth 2 (predicate called inside another predicate returns false).
5. **Tapped sources.** Are tapped creatures' statics active? **Yes by default** — tapping a creature doesn't suppress its abilities in MTG. If tsot wants different semantics, it should ratify in RULES.

---

## How this fits with LIMITATIONS.md's four themes

- **events** — LUA.md. Statics are a new kind of handler; the parsing extends `read_handlers`.
- **stack** — STACK.md. Orthogonal — statics don't go on the stack; they're continuous. Phase 4 (replacements) interacts with event dispatch but not with priority.
- **costs** — Phase 3 (restrictions) might block "cannot be used as cost paid"; otherwise unrelated.
- **types** — Restrictions might be type-based ("creatures cannot attack"). Otherwise unrelated.

## Open rule extensions to ratify in RULES.md

- **A.2** already defines "static ability" as a continuous effect — minor refinement to specify lazy re-evaluation semantics may be useful but isn't required.
- **Predicate purity** — statics' predicates must be read-only (no state mutation). Should be ratified.
- **Source-on-board condition.** Effect applies only while source is on BOARD. Should be explicit in RULES alongside A.2.
