# roam — UI

See @CLAUDE.md for the JS-is-used-in-spite axiom and the sacred-error
rule. This doc covers the UI layer that lives on top of those.

## Axiom — single DOM element

The canvas is the entire game surface. Every menu, every panel, every
inventory grid, every dialog, every HUD widget is painted inside it.
There is no second `<div>`, no React tree, no HTML overlay. JS owns
exactly one DOM element — the canvas — and never touches it again
after handing it to wasm.

"Why" this axiom: each DOM element that holds game state is a
boundary across which truth has to be re-synchronized every frame.
Two sources of truth (Rust state + DOM state) is a class of bug we
opt out of by collapsing to one. Renders, input, focus, z-order,
animation — all of it lives inside the canvas, where Rust is the
only writer.

JS keeps its three legitimate jobs (call browser APIs wasm can't
reach, init/teardown of those, byte-shoveling) and adds nothing.

## v0.4.1 decision — eframe owns the canvas

`eframe::WebRunner::start(canvas, …)` boots on the existing
`HtmlCanvasElement` already in the DOM. eframe's internal
`request_animation_frame` loop drives the frame; the previous
JS-side rAF loop driving `roam_render` goes away.

The alternative considered — `egui_glow` guesting on the existing
canvas while the JS bridge keeps the frame loop — loses on two
counts. (a) Input pipeline split-brain: pointer + key events would
need to fan out to both egui and the world-input path with manual
"who handled it" arbitration. (b) The single-DOM-element axiom
blurs: egui doesn't own the canvas, so the JS bridge has to keep a
`contextmenu` listener for the spawn menu, which violates the rule
that JS holds no game UI.

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
