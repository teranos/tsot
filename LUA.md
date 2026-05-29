# tsot — Lua Execution Plan

> Three-phase plan for turning card abilities from strings into executable Lua handlers.
> Resolves the `events` theme in LIMITATIONS.md.

---

## Phase 1 — Foundation

Cards become modules; the engine fires a small set of events; a minimal `game` API lets handlers do basic moves.

**Goal:** prove the architecture. Three to five cards' abilities actually execute. No design questions deferred to later phases gate this.

### Status (2026-05-29)

Phase 1 is in progress. All six event fire sites are wired; the API surface and corpus retrofits are partial.

**Events wired** (fire site → `fire_self_only` or `fire_with_partner` in `lua_api.rs` → log-and-continue on error):
- [x] `on_blocked_by` (per blocker, in `declare_blocker`) — added beyond the original v1 list as the squirrel-overrun canary
- [x] `on_die` (in `resolve_combat` death loop, after Board → Graveyard)
- [x] `on_enter_board` (in `play_card` after board.push + attachment wiring)
- [x] `on_attack` (in `declare_attacker` after attack recorded)
- [x] `on_block` (in `declare_blocker`, blocker-side; per blocker)
- [x] `on_play` (in `play_card` after validation, before mutations — card still in HAND)

**Self table** passed to handlers: `{ instance_id, owner, controller, attached }`. Partner table (`on_blocked_by`, `on_block`) has the same shape.

**`game` API** (exposed via per-call scoped userdata in `src/game/lua_api.rs`, built by `build_game_table!` macro):
- [x] `game.damage(card_id, n)`
- [x] `game.mill(player_id, n, "graveyard"|"exile")`
- [x] `game.draw(player_id, n)` — empty-deck mid-effect assigns L.1 loss
- [x] `game.move(card_id, dest_zone)` — searches zones AND attached lists; clears face_down when unattaching
- [x] `game.opponent(player_id)`
- [x] `game.deck_top(player_id) → iid_or_nil` — read top of deck without drawing
- [ ] `game.tap(card_id)`, `game.untap(card_id)`
- [ ] `game.zones(player_id).{hand, deck, graveyard, exile, board}`
- [ ] `game.card(card_id)` — read-only view
- [ ] `game.add_status(card_id, kind, duration)`
- [ ] `game.discard(player_id, n)` — needs choice in the natural reading
- [ ] `game.print(msg)` — debug

**Cards with active handlers:**
- `tantrum-imp` — `on_blocked_by`: damage blocker 1, mill defender 1 to exile
- `squirrel-overrun` — `on_blocked_by`: draw 1
- `trustworthy-lender` — `on_die`: return attached to controller's hand
- `midnight-raven` — `on_attack`: put top of deck on the bottom
- `goblin-scribe` — `on_enter_board`: draw 1
- `thorn-beetle` — `on_block`: deal 1 damage to attacker

**Cards in corpus awaiting Phase 2** (data + abilities text only, no handler):
- `goblin-berserker`, `goblin-warlord`, `goblin-conspirator` — all need choice API (`discard a card`, `reveal a goblin`); `goblin-warlord` also needs `static`.

**Other Phase 1 spec items:**
- [ ] mlua sandbox mode (strip `os`, `io`, `loadstring`)
- [x] `CardRegistry` owns long-lived Lua VM; handlers stored as `mlua::Function` on `Card`
- [x] Engine metric `event_fires: HashMap<EventName, [u32; 2]>` plumbed to sim output as a per-event breakdown with a `wired` column

**Plumbing pattern:** every event method (`play_card`, `declare_attacker`, `declare_blocker`, `confirm_blocks`) takes `Option<&Lua>`. `None` = tests of pure game logic; `Some(registry.lua())` = sim and integration tests. The trigger to introduce an `Engine` wrapper owning both `GameState` and `&CardRegistry` is when this `Option<&Lua>` becomes noisy across many more methods.

**Scope (in):**
- **Card file shape extended.** Each `.lua` card may add function fields alongside the existing data table:
  ```lua
  return {
    id = "mesopelagic-fish",
    -- ... data ...
    on_die = function(game, self) ... end,
  }
  ```
- **Event taxonomy v1**, 5 events:
  - `on_enter_board` — fires when a card enters the BOARD (after play_card resolves).
  - `on_die` — fires when a creature is moved to GRAVEYARD due to combat damage.
  - `on_attack` — fires when the controller declares an attack with this creature.
  - `on_block` — fires when a creature is declared as blocker.
  - `on_play` — fires when a card is played from HAND (before destination determination).
- **`game` API v1**, sync only, ~10 methods:
  - `game.move(card_id, zone)` — move within current owner's zones.
  - `game.draw(player_id, n)` — draw n cards.
  - `game.zones(player_id).{hand, deck, graveyard, exile, board}` — list of card IDs.
  - `game.card(card_id)` — read-only view of a card (id, type, colors, stats, etc.).
  - `game.tap(card_id)`, `game.untap(card_id)`.
  - `game.damage(card_id, n)` — add n to damage; trigger death check.
  - `game.add_status(card_id, kind, duration)` — apply a status effect.
  - `game.opponent(player_id)`.
  - `game.print(msg)` — debug only.
- **Lua execution via mlua**, sandbox mode enabled (no `os`, `io`, `loadstring`, etc.).
- **Engine fires events** at the existing sites where TODOs already mark them. Each `fire_event(name, ctx)` call iterates relevant cards' handlers and invokes them.

**Out (deferred to later phases):**
- Player choices (no `game.choose_*` yet — handlers run to completion without input).
- Continuous effects (`static`).
- Visibility filtering of the API view.
- Damage to player, complex zone transitions, P.8 attached cleanup.
- Variable X cost.

**Cards working after Phase 1:**
- `mesopelagic-fish` — partial (return without target choice; just picks first non-creature in graveyard).
- `stinging-bee` — partial (apply SkipUntap status on damage, ignoring choice).
- `attach-shuffler`'s "return at end of turn" — once end-of-turn event added.
- Several cards' `on_enter_board` triggers if they have one.

**Deliverable:** `cargo test` includes a test that loads a fixture card with `on_die`, kills it, and verifies the handler ran. `cargo run` simulator shows non-zero counts in the "triggered abilities fired" pending-stats column.

---

## Phase 2 — Player choices and continuous effects

Two big additions: blocking choice prompts and continuous-effect re-evaluation.

**Goal:** real card design space opens up. Handlers can ask players to pick things, and `static` lets cards modify each other.

**Scope (in):**
- **Choice API (blocking via Lua coroutines):**
  - `game.choose_card(pool, opts)` — pool is a list of card IDs; opts include `{filter, optional, prompt}`. Suspends Lua; engine yields to UI/sim; resumes with the chosen ID (or nil if optional).
  - `game.choose_player({opts})`.
  - `game.confirm(prompt)` — yes/no.
  - `game.choose_int(min, max, prompt)` — for variable X.
- **Headless choice mode for the simulator.** The sim provides a `choice_oracle` (random or scripted) so games run without UI. Production runtime asks the user.
- **`static` handler:** a function that returns a list of modifier-add operations. Re-evaluated whenever the engine's mutation counter ticks (every state change). The engine diffs old vs new modifier set and applies the delta.
- **Visibility filtering.** The `game` API view of cards in HAND, ATTACHED, etc. respects V.1–V.7. A handler running on Player A's card cannot see Player B's hand identities through `game.zones(B).hand`; only counts.
- **Death-check integration.** `game.damage` now properly cascades into death events with deferred resolution (don't fire `on_die` mid-damage-tick).
- **End-of-turn events** so attach-shuffler's "return at end of turn" delayed effect works.

**Out (deferred to Phase 3):**
- The response window machinery (R.1, the stack) is still not built. Choice prompts inside handlers are *not* the same as instant-casting response windows.
- Variable X cost integration with play_card (X chosen at cast — different surface from `choose_int` inside a handler).
- Full event taxonomy (still ~5-7 events).

**Cards working after Phase 2:**
- `mesopelagic-fish` fully (choice of which non-creature to return).
- `squirrel-overrun` mostly (static stat boost + on_attack with attach choice + on_blocked_by draw).
- `companion-bird` fully (static gives host flying while attached).
- `modern-lcd-clock` fully (static cost reduction on creatures).
- `flesh-eating-plant` partially (returns insect on death with choice).
- `silent-murder` fully (choose target + kill + conditional draw).

**Deliverable:** simulator's headless oracle picks random-but-legal choices. Card-level integration tests verify each card's intended behavior end-to-end. The "triggered abilities fired" column shows realistic per-game counts.

---

## Phase 3 — Full coverage

Round out the taxonomy, harden the API, integrate with the stack work that lands separately.

**Goal:** every card in the corpus runs. New cards can be authored end-to-end in Lua with no Rust changes.

**Scope (in):**
- **Full event taxonomy** — add the remaining events surfaced by the corpus:
  - `on_damage_to_creature`, `on_damage_to_player`, `on_zone_change` (generic), `on_draw`, `on_discard`, `on_attach`, `on_detach`, `on_turn_begin`, `on_turn_end`, `on_combat_begin`, `on_main_phase_begin`.
- **`game` API completion:**
  - `game.attach(card_id, host_id)`, `game.detach(card_id)`.
  - `game.exile(card_id)`, `game.bounce(card_id)` (BOARD → HAND).
  - `game.discard(player_id, n, opts)`.
  - `game.mill(player_id, n, dest)` — n cards from top of DECK to GRAVEYARD or EXILE.
  - `game.search(zone, filter)` — for tutor effects.
  - `game.modify_card(card_id, prop, value)` — for color/symbol/type mutations.
  - `game.count_by_symbol(zone, symbol)` — Amsterdam-City's `count cards with symbol ⨳`.
  - `game.set_modifier(card_id, key, value)`, `game.remove_modifier(card_id, key)`.
  - `game.reveal(card_id)` — flips face-down to face-up temporarily.
- **Integration with stack work** (the `stack` theme from LIMITATIONS.md, built in parallel):
  - Handlers that emit `game.cast_response_for(card_id)` queue a response on the stack.
  - `on_play` inside a response window respects R.1–R.7.
  - Triggered abilities themselves go on the stack.
- **Variable X cost handler hook.** When a card has `is_x` cost, `play_card` calls a card-level `choose_x(game, self)` Lua function (or uses `game.choose_int` from inside).
- **Pre-declared responses** (UX X.6 / X-E.4) via a Lua-level register API.
- **Performance hardening** — memoize `static` evaluation based on observed dependencies, avoid full-scan recomputation.
- **Card author documentation** — a guide explaining the event taxonomy and `game` API with 5–10 example cards walked through.

**Out:**
- Multiplayer (still 1v1).
- Custom user-uploaded cards (sandboxing is in place but vetting flow isn't).

**Cards working after Phase 3:** every card currently in `cards/`. The "Pending mechanics" section in `cargo run` shows non-zero values for nearly all rows.

**Deliverable:** corpus-wide regression suite — for every card, an integration test asserting its observable behavior in a scripted game state. Card-authoring docs let someone write a new card from scratch using only the guide.

---

## Cross-cutting design questions to resolve

These come up across phases and are worth pinning down early:

1. **Where do handlers live in the Card struct?** Probably as `HashMap<EventName, mlua::Function>` separate from `abilities: Vec<String>`. Strings stay as documentation / pre-implementation, functions are the executable surface.
2. **Is the `game` handle a fresh userdata per call, or a long-lived registry?** Probably long-lived — `game` is `mlua::UserData` whose methods proxy into Rust.
3. **Error handling.** A Lua error inside a handler should not crash the engine. Catch, log, continue — the rest of the game keeps running. Maybe surface as a game event for testing.
4. **Hot reload.** Should card edits trigger a re-load mid-session? Convenient for authoring; complicates state. Probably no for v1; yes for dev mode later.
5. **Coroutine yield semantics.** For Phase 2's choice API, what yields and what blocks? mlua's `Thread` is the mechanism. Spec the exact pattern before writing it.

---

## How this fits with LIMITATIONS.md's four themes

- **events** — this whole document.
- **costs** — Phase 3's variable-X integration; otherwise mostly Rust-side work, separate slice.
- **types** — Rust-side work in `play_card`. Lua doesn't decide where instants go; the engine does.
- **stack** — separate slice. Phase 3 integrates with it but doesn't build it.

Each phase here is one slice of `events`. The four themes' slices interleave by necessity.
