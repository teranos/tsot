# roam — UI

See @CLAUDE.md for the JS-is-used-in-spite axiom and the sacred-error
rule. This doc covers the UI layer that lives on top of those.

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

- [x] Read-first audit — ebc-battery-tester source, eframe 0.34.1 WebRunner, egui 0.34.3 changelog + `containers::menu` source, `examples/custom_3d_glow` PaintCallback pattern. Findings folded into this doc.
- [x] `docs/UI.md` — object-identity axiom recorded, eframe decision recorded, API gotchas written down before they bite again.
- [ ] Pin `eframe = "=0.34.1"` + `egui = "=0.34.3"` in `roam/Cargo.toml`; boot `eframe::WebRunner::start` on the existing canvas; JS bridge stops driving rAF.
- [ ] World render moves into a `PaintCallback` inside a fullscreen `egui::CentralPanel`; GL state restore at the callback tail becomes load-bearing.
- [ ] Parchment theme module — `roam::ui::theme::apply(ctx)` runs once at `App::new()` with the colors pinned below.
- [ ] Right-click spawn menu via `egui::containers::menu` + `Popup::context_menu(response)`; one item "Spawn (16, 16)"; width-pinned against font-swap wrap.
- [ ] 16 fonts embedded via `include_bytes!`; picker renders every entry's label in its own `FontFamily::Name`.

This list is the truth surface for v0.4.1 progress — checkboxes flip as
tasks land, the corresponding sections below stay frozen as the
decisions they captured. Once v0.4.1 ships, this section moves to a
"v0.4.1 — shipped" header and v0.5's UI work (inventory grid,
deckbuilder, mobile touch) opens a new Status block.

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

## Theme — parchment

Beige panel background, dark brown text. Source: ebc-battery-tester
visual reference (egui 0.34.1, `widgets::global_theme_preference_buttons`
elided — this game has one theme). `roam::ui::theme::apply(ctx)`
runs once at `App::new()`:

- `Visuals::light()` as base.
- `style.visuals.panel_fill = Color32::from_rgb(228, 210, 175)`.
- `style.visuals.override_text_color = Some(Color32::from_rgb(48, 28, 14))`.
- `style.animation_time = 0.0`.
- Menu `Frame::menu(style)` inner margin = `Margin::same(2.0)`.

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
