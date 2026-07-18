# tsot — Lua

How card abilities are authored, executed, and triggered.

---

## Shipped

**Events** (`card::EventName`):
`on_enter_board`, `on_die`, `on_would_die`, `on_attack`, `on_block`,
`on_blocked_by`, `on_play`, `on_attached_as_cost`,
`on_dealt_damage_to_player`, `on_turn_begin`, `on_creature_dies`,
`on_tapped`, `on_untapped`, `on_delayed_trigger`.

`on_tapped` / `on_untapped` are mirrors: they fire on a card the moment
it becomes tapped / untapped (a state transition). Both fire on the
synchronous path (attack tap, U.2 untap step) and on external
`game.tap` / `game.untap` from inside another handler, via the
deferred-event queue. Ankle Scorcher's "whenever this becomes untapped,
discard a card" is `on_untapped`.

`on_would_die` opens the death-replacement window (RULES.md P.40).

**Handler signature** — `function(game, self, partner?)`. `self` carries
`{ instance_id, owner, controller, attached }`. `partner` is present
on the two-card events (`on_blocked_by`, `on_block`,
`on_creature_dies`, `on_attached_as_cost`).

**Choice API.** `game.choose_card`, `game.confirm`, `game.choose_player`,
`game.choose_int`. The wrapper raises `mlua::Error::external(ChoicePending)`;
each subsystem (`play_card`, `activate_ability`, `declare_attacker`,
`declare_blocker`, `next_phase`) lifts via `.map_err(_::ChoicePending)?`.
StepEngine catches every variant, rolls back the preview journal,
surfaces a `HumanPrompt`, and re-fires after the user's answer lands
in `HumanReplayOracle.replay`. Headless sim uses `RandomOracle` for
the same surface. Phase-advance triggers go through the same path
(`turn.rs::next_phase_returns_choice_pending_when_on_turn_begin_yields`).

**Static effects.** `static = { ... }` on a card declares a continuous
effect re-evaluated on each engine mutation tick; `card::StaticDef`
carries the affects-predicate + the effect list. 46 cards use it.

**Other `game.*`.** `damage`, `mill`, `draw`, `move`, `move_to`,
`move_to_deck_top`, `move_attached`, `tap`, `untap`, `add_status`,
`add_modifier`, `discard`, `zones`, `card`, `attackers`, `opponent`,
`deck_top`, `deck_bottom`, `print`, plus attach helpers
(`attach`, `attach_from_deck`, `attached_of`, `host_of`), counter helpers
(`counter`, `counter_top`, `chain`, `legal_counter_targets`,
`set_intent`), timing helpers (`schedule_return_at_next_main`,
`grant_extra_turn`, `creature_attacked_this_turn`, `set_summoning_sick`,
`x_value`, `payment_ids`), and death-replacement primitives valid
inside `on_would_die` (see RULES.md P.40):
`prevent_death(self)`, `redirect_death(self, zone)`,
`shed_own_sleeve(self)`.

**Sandbox.** `Lua::new_with(MATH | STRING | TABLE | COROUTINE)`;
`load`/`loadstring`/`loadfile`/`dofile` nil'd. Pinned by
`card::tests::sandbox_denies_dangerous_stdlib`.

**Cards using handlers.** 111 of the corpus. Authoritative count: grep
`cards/*.lua`.

---

## Outstanding

**Events still to wire:** `on_turn_end`, generic `on_zone_change`,
`on_draw`, `on_discard`, `on_attach` (distinct from `on_attached_as_cost`),
`on_detach`, `on_combat_begin`, `on_main_phase_begin`, plus
`on_damage_to_creature` (the creature-target equivalent of
`on_dealt_damage_to_player`).

**`game.*` gaps:** `game.search(zone, filter)` for tutor effects;
`game.modify_card(card_id, prop, value)` for color/symbol/type
mutations; `game.count_by_symbol(zone, symbol)` for Amsterdam-City;
`game.reveal(card_id)` for temporary face-up.

**Variable X cost integration with `play_card`.** When a card has
`is_x` cost, `play_card` should call a card-level `choose_x(game, self)`
or let the card use `game.choose_int` from inside its handler.

**Stack integration** — see STACK.md. Triggered abilities currently
resolve inline; the stack work will let `on_play` inside a response
window respect R.1–R.7 and let handlers queue responses.

**Visibility filtering** of the `game` API view (V.1–V.7). Handlers
can read what they shouldn't through `game.zones(opponent).hand`.
`face_down` and a couple V-rule references exist in `lua_api.rs` but
there's no uniform filter.

**Deferred death cascade.** `game.damage` triggering `on_die`
mid-damage-tick is still the call path; resolving deaths after all
damage is dealt is still pending.

**Card author guide.** Walkthrough of the event taxonomy + `game` API
through 5–10 example cards. None of the docs in this file substitute
for it.

---

## Card variants (balance-probe schema)

A card file may declare alternate versions of itself inline via a
`variants = { [key] = { overrides } }` table. The loader emits the
base card plus one card per variant, with id `{base-id}-{key}` and
`is_variant = true`. Variants are excluded from `main.rs::playable_pool`
so they never enter `make evolve` / champions / gauntlets; `tsot
balance-probe` is the only consumer that picks them up.

```lua
return {
  id = "dark-salamander",
  -- ... base definition ...
  variants = {
    ["1pwr"] = { activated = {...} },
    ["2pwr"] = { activated = {...} },
  },
}
```

Override semantics: each top-level field in a variant table replaces
the base wholesale. No deep merge.

Workflow: `make probe` auto-discovers; `make probe <CARD_ID>` runs
one. The probe pins each variant into every genome of a short EA
and reports ceiling fitness + top co-occurring cards.

When to add variants: comparing alternative versions of the same
conceptual card (cost, stats, effect magnitude). Don't duplicate .lua
files.

---

## Open

- **Hot reload.** Re-load cards mid-session? Convenient for authoring,
  complicates state. Deferred.
- **Deployment target.** Browser via wasm, wrapped in a native WebView
  shell (Capacitor or equivalent) for App Store / Play Store
  distribution. Rust target: `wasm32-unknown-emscripten` (required by
  mlua's vendored Lua C runtime — pure-Rust wasm targets are not
  supported). Cards embed via `include_dir!` so the runtime has no
  filesystem dependency. `rayon` is cfg-gated to native; the EA sim
  doesn't ship in the app.

See LIMITATIONS.md for cross-cutting themes (events, costs, types,
stack).
