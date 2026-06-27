# roam — UI

See @CLAUDE.md for the JS-is-used-in-spite axiom and the sacred-error
rule. This doc covers the UI layer that lives on top of those.

> **roam stays at v0.4.x; new work moved to [`universe/`](../../universe/) — fresh Bevy/ECS code, not a port.** Everything below is the historical record of the v0.4.x eframe + egui + render_gl UI. The object-identity axiom at the top of this doc carries forward into universe unchanged — it gets *easier* to implement under ECS (entities + Location components + a system that interpolates render position toward target). See `docs/adr/0003-bevy.md` and `universe/CLAUDE.md` for the direction.

## Axiom — object identity is persistent

Every thing in the universe is one object that keeps its identity
through every transformation. A card on the ground and that same
card in your inventory and that same card on the cursor while you
drag it and that same card in your deck — it is one object the
whole time. The card is not destroyed when picked up and a new card
spawned in the inventory. The card moves. The data structure does
not churn, the render entity does not blink out and back in, and
the user sees the actual object physically travel from one place
to another.

This applies to everything: cards, flowers, future fonts-as-drops,
NPCs, anything the player perceives as a discrete thing. Each one
has a stable identity from the moment the player first sees it
until the moment it is destroyed for a real reason (consumed,
expired, decayed) — never as a side-effect of changing where it
lives.

**Implementation implications**

- *One entity, many locations.* The data model holds each object
  once, with a `Location` field — `World { x, y, z }`, `Inventory
  { slot }`, `Cursor`, `Deck { index }`. Pickup is `location =
  Inventory { … }`. There is no `WorldCard` struct and a separate
  `InventoryCard` struct that get translated between.
- *One render entity per object.* The render layer keeps a stable
  handle (an entity id, or just `&Object`) for each thing it has
  drawn. Position is a property of the render entity; container
  membership is a property of the render entity. When the object
  moves between containers, the entity's `target_position`
  changes; its render slot does not get destroyed.
- *Movement is animation, not teleport.* Position changes are
  interpolated. The card on the ground that you click moves
  smoothly toward your cursor, follows the cursor while you drag,
  and animates into its inventory slot when you drop. Same render
  entity throughout. Same screen-space pixels morphing into a new
  shape, not two ghosts handing off.
- *Drag is a first-class affordance.* If an object is on the
  screen, the player can grab it with the cursor and move it
  somewhere valid. The cursor has a `held: Option<EntityId>` slot;
  while held, the object's position tracks the cursor; on release,
  the drop target accepts or rejects (with the object animating
  back to its origin on reject). Nothing about this is special-
  cased per object type — it falls out of the model.

**Tension with procedural worldgen**

Today `flower_at(x, y)` and `card_at(x, y, catalog_len)` are pure
functions of world coordinates: no per-instance identity exists in
the world until pickup. Aligning with this axiom means the moment
of "the player first sees this object" is the moment the engine
materializes an entity id for it. Two reasonable derivations: a
deterministic id from `world_hash(x, y, HashDimension::EntityId)`
(stable across reloads even while the object is on the ground), or
a lazily-assigned monotonic id allocated when the tile enters the
viewport (cheap, but the same tile re-entering the viewport would
yield a different id — bad). The deterministic path wins.

This is an axiomatic shift, not a v0.4.1 implementation gate — the
shift lands when inventory + drag enter scope. v0.4.1 only owes
the axiom: the spawn menu opens on a player marker that is itself
a persistent render entity, not a per-frame "draw a quad at
(x, y)" with no handle.

## Status (v0.4.1)

Action-level checklist — each item is a single edit or a single
verifiable outcome, not a project. Decisions live in the sections
below; this is execution.

**Foundation — eframe + egui pinned, WebRunner booting:**
- [x] Read-first audit (ebc-battery-tester source, eframe 0.34.1 WebRunner, egui 0.34.3 `containers::menu`, `examples/custom_3d_glow` PaintCallback).
- [x] `docs/UI.md` — object-identity axiom, eframe decision, API gotchas, battery-tester visual decision, canonical-inventory decision.
- [x] `eframe = "=0.34.1"` + `egui = "=0.34.3"` + `egui_glow = "=0.34.3"` pinned in `roam/Cargo.toml`; `wasm-bindgen-futures` made always-on for wasm32.
- [x] `cargo check --target wasm32-unknown-unknown --lib` clean post-pin.
- [x] `roam/src/ui/mod.rs` with `RoamApp` implementing `eframe::App` (the new 0.34 `fn ui(&mut self, ui: &mut Ui, frame: &mut Frame)` signature).
- [x] Wasm entrypoint `roam_ui_init(canvas)` calls `render_gl::init` then `eframe::WebRunner::new().start(canvas, …, |cc| Ok(Box::new(RoamApp::new(cc))))` inside `spawn_local`.
- [x] **JS-bridge handoff** — `assets/src/js-bridge.js` flipped: `roam_render_init` + JS rAF `roam_render_frame` loop removed; `roam_ui_init` called at startup; rest of the rAF loop (net tick, peer publish, HUD, save throttle) intact. Browser-verified: world renders, UI alive, peer markers + pickups still work.

**Input — single keyboard map:**
- [x] `roam::input::FrameInput::read(ctx)` is the sole reader of `egui::InputState`'s keys. No other file in the codebase imports `egui::Key`. WASD + arrow keys + numpad 1-9 (8-way roguelike, diagonals on one key, Num5 = act-on-self → spawn) all funnel here.
- [x] `WebOptions::should_stop_propagation = false` so surviving JS-side keyboard listeners (zoom +/-, log shortcuts) still fire alongside egui's canvas-target listener.
- [x] Old JS WASD listener + `inputBits()` helper deleted from `assets/src/js-bridge.js`; the rAF `tick(input, dt)` call gone — `RoamApp::logic` calls `roam_tick_impl` from `FrameInput`.

**World render → `PaintCallback`:** (Rust side complete; visible only after the JS handoff above)
- [x] `RoamApp::ui` mounts a fullscreen `egui::CentralPanel` (via `show_inside`, the 0.34 non-deprecated path).
- [x] `ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag())` produces the world rect + response.
- [x] `egui::PaintCallback` built with `egui_glow::CallbackFn::new(move |_info, _painter| { render_gl::render_frame(…) })`.
- [x] Callback body calls the existing `render_gl::render_frame` thread-local path (eframe and `render_gl` share the underlying browser WebGL2 context via the shared canvas).
- [ ] Day brightness Rust-side: v0.4.1 ships a fixed `1.0`; the JS-side longitude → brightness derivation moves to Rust in a follow-up.
- [ ] Viewport buffer write: currently JS calls `roam_viewport_write` in the rAF loop before render. Until the JS handoff lands, this still drives the buffer; after, `RoamApp::ui` calls `roam_viewport_write_impl` itself before pushing the `PaintCallback`.

**UI surface — single right-click menu (battery-tester three-panel layout removed):**
- [x] `egui::CentralPanel::default().show_inside(ui, …)` is the only persistent panel and contains the world `PaintCallback`.
- [x] `egui::Popup::context_menu(&response).show(…)` on the world response holds every UI affordance: Spawn (16, 16) button → Font submenu (16 fonts, each row in its own `FontFamily::Name`) → `egui::widgets::global_theme_preference_buttons` for dark/light.
- [x] No top bar, no left panel — the earlier battery-tester three-panel transplant was confabulation. Pre-egui JS Spawn button in `assets/play.html` deleted in lockstep.
- [x] No custom `Visuals` or `Style` overrides — egui defaults stand.
- [x] Width-pin against font-swap wrap on the Spawn button: `ui.set_min_width(180.0)` + `ui.with_layout(Layout::top_down_justified(Align::LEFT), …)` + `Button::new("Spawn (16, 16)").wrap_mode(TextWrapMode::Extend)`.

**Fonts:**
- [x] 16 font files under `roam/assets/fonts/` (1.3 MB total): alte_haas_grotesk × 2 (regular + bold), augusta × 2 (plain + shadow), berry_rotunda, cardinal × 2 (plain + alternate), cat_franken_deutsch, fraktur_handschrift, isabella, lyric_poetry, rapscallion, renata_cat, the_art_of_illumina, euphorigenic, x_scale.
- [x] `RoamApp::new(cc)` builds `egui::FontDefinitions` with one `FontFamily::Name((*name).into())` per font via `include_bytes!`.
- [x] Font picker is a `ui.collapsing("Font", …)` inside the left controls panel.
- [x] Each row renders as `egui::RichText::new(name).family(FontFamily::Name(name.into())).size(18.0)` inside a `selectable_label` — the user previews each typeface before picking.
- [x] On pick: `self.current_font_idx` updates and `write_fonts(&ctx)` re-runs with the selected font prepended to the `Proportional` family list, so subsequent text uses it.

The list is the truth surface for v0.4.1 progress — checkboxes flip
as items land, the decision sections below stay frozen. Once v0.4.1
ships, this becomes a "v0.4.1 — shipped" historical header and v0.5
UI work (inventory grid, deckbuilder, mobile touch) opens a fresh
Status block.

## v0.4.1 decision — eframe owns the canvas

`eframe::WebRunner::start(canvas, …)` boots on the existing
`HtmlCanvasElement` already in the DOM. eframe's internal
`request_animation_frame` loop drives the frame; the previous
JS-side rAF loop driving `roam_render` goes away.

The alternative considered — `egui_glow` guesting on the existing
canvas while the JS bridge keeps the frame loop — loses on input
pipeline split-brain: pointer + key events would need to fan out to
both egui and the world-input path with manual "who handled it"
arbitration. Two input owners means two truth surfaces for "what is
the user doing right now," which the object-identity axiom above
will not tolerate once drag enters scope (the cursor's `held`
state must be readable from a single place).

## World render — PaintCallback inside a fullscreen `CentralPanel`

The world (tiles + flowers + cards + markers + lines) keeps
`roam::render_gl::render_frame` unchanged, but the call site moves:
instead of being invoked by JS each rAF, it lives inside an
`egui::PaintCallback` attached to a fullscreen `egui::CentralPanel`.
Pattern follows `eframe`'s `examples/custom_3d_glow/` — the
callback receives the `egui_glow` painter, calls `painter.gl()` to
get the `glow::Context`, and from there the same WebGL2 ops run.

GL state contract: egui_glow sets the viewport before the callback;
post-callback state is unspecified. The callback restores any bound
VAO / shader program / buffer it touches before returning. This is
already roughly true of `render_frame` (it binds + clears its own
state), but the `gl.bind_vertex_array(None)` tail call becomes
load-bearing.

The `roam::render_gl::gl_context()` accessor (added in the
render_gl split commit `c2622fd`) gives the `egui_glow` painter
its starting handle. Both surfaces share one GL context, one
canvas, one frame loop.

## Input — through egui first

Pointer + keyboard events flow into `egui::Context` via eframe.
World-input handlers (WASD, facing arrow, right-click on player)
read what egui didn't consume. Right-click on the player marker
triggers `egui::Popup::context_menu(response)` with one item
"Spawn (16, 16)". No JS-side `contextmenu` listener exists.

`response.secondary_clicked()` is the trigger; egui handles the
open/close lifecycle. Left-click outside the menu closes it
naturally. Animation time on the popup is forced to `0.0` so the
fade-in doesn't lag behind the right-click.

## Touch movement — virtual joystick on the world

Phones have no keyboard, so the WASD / arrow / numpad map in
`roam::input` can't move the player there. A primary press (a finger
on a touchscreen, a left-drag on desktop) becomes a virtual joystick:
the press position is the origin, the live pointer is the stick, and
the drag vector is mapped onto the **same** 8-way `move_bits` the
keyboard feeds — `input::drag_to_move_bits` picks the octant, a drag
shorter than `TOUCH_DEADZONE_PX` reads as centred. No new movement
model, no second code path through `world::step`; the joystick is
just another producer of the existing bitmask.

`drag_to_move_bits` is pure (no egui, no FFI) and host-unit-tested for
its octant boundaries and deadzone — the reason `roam::input` is no
longer `#[cfg(target_arch = "wasm32")]`-gated.

The base ring + knob are drawn as a non-interactable foreground
`egui::Area` while the press is active (`RoamApp::joystick`, fed from
`FrameInput::{touch_origin, touch_pos}`), so the player can see where
their thumb is steering. Right-drag is untouched — it still belongs to
the context menu.

## Visual style — battery-tester defaults

Adopted directly from ebc-battery-tester (0.34.1, the audit reference):
**no custom `Visuals` or `Style` overrides — egui defaults stand**.
Layout is three panels:

- `egui::TopBottomPanel::top("menu")` containing
  `egui::MenuBar::new()` for top-level menus + the dark/light toggle
  via `egui::widgets::global_theme_preference_buttons(ui)`.
- `egui::SidePanel::left("controls")` for the font picker and
  whatever the slice after this one adds (inventory, deckbuilder).
- `egui::CentralPanel::default()` holding the world
  `PaintCallback`.

The previously-planned parchment theme (beige panel + dark brown
text + animation_time=0 + Frame::menu margin=2.0) is **dropped**.
It was a confabulation from before the read-first audit; the audit
showed battery-tester uses egui defaults, and the user explicitly
chose the battery-tester look. The `roam::ui::theme` module either
shrinks to a no-op or doesn't exist — whichever falls out of the
implementation cleanly.

The dark/light toggle is the only theme affordance the player gets.
Past that, the visual is whatever egui's default theme renders, and
that's the deliberate choice.

## Canonical inventory

Inventories are canonical world entities. Decision recorded here
and in `docs/CANONICAL.md`.

Implication: when player A picks up a card from a tile, the card is
not "removed from the world and instantiated in A's inventory" — it
*moves* into A's inventory, and that move is a canonical world
transformation visible to every observer in render range. The card's
entity id (deterministic from `world_hash(x, y, …)` while on the
ground; persistent post-pickup) stays the same; only its `Location`
changes from `World{x, y, z}` to `Inventory{owner: did:key, slot}`.

For other players in render range, the same card animates from the
ground tile *into player A's marker*. Same render entity, position
interpolated. No "card disappears for B while it appears for A" —
that would violate the object-identity axiom and would be a worse
user experience besides (B sees the world reshape without
explanation).

Non-canonical (sandbox, per `docs/CANONICAL.md`) players carry
inventories that exist only in their local overlay and don't
replicate. On M7 promotion, the sandbox inventory resets along
with the rest of the personal overlay — consistent with the
existing "promotion is one-way and meaningful loss" axiom.

Wire shape: the existing M6 pickup gossipsub message already
carries the picker's `did:key`. The render layer uses that to
target the animation. Inventory *contents* gossiped as canonical
state is a follow-up question — for v0.4.1 the visible move is
client-side animation off the pickup wire; "what is in A's
inventory right now" as queryable canonical state is a v0.5+
concern when trades, robbery, and vendor inventories enter scope.

## Menu APIs — 0.34, not training-data 0.29

`egui::menu` is deprecated. The current entry points are
`egui::containers::menu::MenuBar` + `MenuButton`, and
`egui::containers::popup::Popup::context_menu(response)`. The
`MenuBar` builder takes a closure; `MenuButton::new("Label").ui(ui, |ui| { … })`
is the submenu pattern.

Width-pinning the spawn button so it doesn't wrap when the active
font changes:

- `ui.set_min_width(140.0)`.
- `ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| { … })`.
- `Button::new("Spawn (16, 16)").wrap_mode(TextWrapMode::Extend)`.

All three are needed; missing any one and the button re-wraps on
the next font swap.

## Fonts — v0.4.1 stopgap, fonts-as-gameplay later

v0.4.1 embeds 16 fonts via `include_bytes!` under
`roam/assets/fonts/` and registers each as its own
`FontFamily::Name`. The picker renders every entry's label in that
entry's `FontFamily::Name`, so the user sees the typeface before
picking. Selected font becomes the default proportional family.

The future shape (task #49, post-v0.4.1, likely v0.6+): fonts
become world drops alongside cards and flowers, the relayer
publishes a font catalog parallel to the card catalog, and each
NPC civ in worldgen renders its own typeface. Curated for
legibility; player-selected typeface is a local preference that
overrides the civ-font for the UI surface but never for diegetic
civ rendering.

The v0.4.1 picker is the first filter pass — every font that ends
up canonical has to pass the "previewable in its own typeface"
sanity check before it can be in the catalog.

## What this doc does not cover

- Inventory grid layout — separate slice once cards + flowers are
  both in inventory and the screen needs more than a `Pickup` count.
- Deckbuilder — v0.5 scope, blocked on autobattle.
- Mobile movement controls — touch input is a v0.5+ question.
- Per-PR dev env / inline error overlays — task #34.
