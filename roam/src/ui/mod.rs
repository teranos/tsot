//! In-canvas UI built on eframe (=0.34.1) + egui (=0.34.3).
//!
//! Architecture decisions live in `docs/UI.md`:
//! - eframe owns the canvas, drives the rAF loop, routes input.
//! - World render lives inside an `egui::PaintCallback` inside a
//!   fullscreen `CentralPanel`.
//! - Visual style follows ebc-battery-tester: egui defaults +
//!   `global_theme_preference_buttons` for light/dark toggle, three
//!   panels (top menu, left controls, central world).
//! - 16 fonts are embedded via `include_bytes!`; the picker renders
//!   each row in that row's own `FontFamily::Name` so the user
//!   previews the typeface before picking.
//! - Right-click on the world opens an `egui::Popup::context_menu`
//!   with a "Spawn (16, 16)" item — width-pinned via
//!   `set_min_width` + `Layout::top_down_justified` +
//!   `Button::wrap_mode(TextWrapMode::Extend)` so font swaps don't
//!   re-wrap it.

#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::error::{emit as emit_error, Severity};

/// 16 fonts embedded at compile time. Stop-gap until the future
/// fonts-as-gameplay slice (task #49) makes them world drops served
/// from the relay catalog. Each entry's name is the
/// `FontFamily::Name` the picker exposes; the bytes are static so
/// the binary lives in the wasm `data` section once.
const BUNDLED_FONTS: &[(&str, &[u8])] = &[
    ("Alte Haas Grotesk Regular", include_bytes!("../../assets/fonts/alte_haas_grotesk_regular.ttf")),
    ("Alte Haas Grotesk Bold",    include_bytes!("../../assets/fonts/alte_haas_grotesk_bold.ttf")),
    ("Augusta",                   include_bytes!("../../assets/fonts/augusta.ttf")),
    ("Augusta Shadow",            include_bytes!("../../assets/fonts/augusta_shadow.ttf")),
    ("Berry Rotunda",             include_bytes!("../../assets/fonts/berry_rotunda.ttf")),
    ("Cardinal",                  include_bytes!("../../assets/fonts/cardinal.ttf")),
    ("Cardinal Alternate",        include_bytes!("../../assets/fonts/cardinal_alternate.ttf")),
    ("CAT Franken Deutsch",       include_bytes!("../../assets/fonts/cat_franken_deutsch.ttf")),
    ("Fraktur Handschrift",       include_bytes!("../../assets/fonts/fraktur_handschrift.ttf")),
    ("Isabella",                  include_bytes!("../../assets/fonts/isabella.ttf")),
    ("Lyric Poetry",              include_bytes!("../../assets/fonts/lyric_poetry.ttf")),
    ("Rapscallion",               include_bytes!("../../assets/fonts/rapscallion.ttf")),
    ("Renata CAT",                include_bytes!("../../assets/fonts/renata_cat.ttf")),
    ("The Art of Illumina",       include_bytes!("../../assets/fonts/the_art_of_illumina.ttf")),
    ("Euphorigenic",              include_bytes!("../../assets/fonts/euphorigenic.otf")),
    ("X-Scale",                   include_bytes!("../../assets/fonts/x_scale.ttf")),
];

pub struct RoamApp {
    /// Index into `BUNDLED_FONTS` — the active proportional font.
    current_font_idx: usize,
    /// View zoom (screen px per world px). 1.0 default; the spawn
    /// menu doesn't change it for v0.4.1.
    zoom: f32,
}

impl RoamApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let app = Self {
            current_font_idx: 0,
            zoom: 1.0,
        };
        app.write_fonts(&cc.egui_ctx);
        app
    }

    /// Rebuild egui's `FontDefinitions`: every bundled font is
    /// available as its own `FontFamily::Name(name)` (so the picker
    /// can preview each row in its own typeface), and the selected
    /// font is prepended to the `Proportional` family so default
    /// text picks it up.
    fn write_fonts(&self, ctx: &egui::Context) {
        let mut defs = egui::FontDefinitions::default();
        for (name, bytes) in BUNDLED_FONTS {
            let key = (*name).to_string();
            defs.font_data.insert(
                key.clone(),
                Arc::new(egui::FontData::from_static(bytes)),
            );
            defs.families
                .entry(egui::FontFamily::Name((*name).into()))
                .or_default()
                .push(key);
        }
        let (active_name, _) = BUNDLED_FONTS[self.current_font_idx];
        defs.families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, active_name.to_string());
        crate::trace::emit(crate::trace::TraceEvent::Note {
            tag: "ui_write_fonts",
            msg: format!(
                "active={active_name} proportional_head={}",
                defs.families
                    .get(&egui::FontFamily::Proportional)
                    .and_then(|v| v.first())
                    .map(|s| s.as_str())
                    .unwrap_or("<empty>")
            ),
        });
        ctx.set_fonts(defs);
    }
}

impl eframe::App for RoamApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Single keyboard map lives in `roam::input`. eframe captures
        // keydown on the canvas; this reads pressed state out of
        // `egui::InputState` once per frame, builds the world's
        // direction bitmask, and dispatches edge-triggered actions
        // (Num5 → spawn). The previous JS rAF path
        // (`tick(inputBits(), dt)` in js-bridge.js) is gone.
        let input = crate::input::FrameInput::read(ctx);
        crate::wasm_ffi::roam_tick_impl(input.move_bits, input.dt_ms);
        if input.spawn_pressed {
            let px = 16.0 * crate::world::PIXELS_PER_TILE as f32;
            crate::wasm_ffi::roam_set_position_impl(px, px, 4);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Build watermark — bottom-right corner, small monospace gray.
        // Compile-time constants from roam::build_info; the Makefile
        // passes git commit + UTC timestamp + profile via env vars.
        // No JS bridge — the wasm bundle is self-describing.
        egui::Area::new(egui::Id::new("roam_build_watermark"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::Vec2::new(-6.0, -6.0))
            .interactable(false)
            .show(&ctx, |ui| {
                let txt = format!(
                    "roam · {} · {} · {}",
                    crate::build_info::COMMIT,
                    crate::build_info::PROFILE,
                    crate::build_info::BUILT_AT,
                );
                ui.label(
                    egui::RichText::new(txt)
                        .monospace()
                        .size(11.0)
                        .color(egui::Color32::from_gray(140)),
                );
            });

        // No top bar, no left panel — the only UI affordance is the
        // right-click context menu on the world. Theme toggle and font
        // picker both live inside that menu (or its submenus). Battery-
        // tester's three-panel layout was a confabulation when this
        // module was first written; the game asked for "font chooser
        // as a game menu" (i.e. inside the right-click menu) from the
        // start, not a permanent side panel.

        let mut newly_picked: Option<usize> = None;

        // Central panel — world render via PaintCallback, full-canvas.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(
                ui.available_size(),
                egui::Sense::click_and_drag(),
            );

            // Right-click → context menu. Spawn, font picker, theme
            // toggle all live here. Width-pin recipe keeps items from
            // wrapping when fonts swap.
            egui::Popup::context_menu(&response).show(|ui| {
                ui.set_min_width(180.0);
                ui.with_layout(
                    egui::Layout::top_down_justified(egui::Align::LEFT),
                    |ui| {
                        let spawn_btn = egui::Button::new("Spawn (16, 16)")
                            .wrap_mode(egui::TextWrapMode::Extend);
                        if ui.add(spawn_btn).clicked() {
                            let px = 16.0 * crate::world::PIXELS_PER_TILE as f32;
                            crate::wasm_ffi::roam_set_position_impl(px, px, 4);
                        }
                        ui.separator();
                        // Font submenu — each row in its own typeface
                        // so the player previews the font before
                        // committing. Selection survives the menu close.
                        ui.menu_button("Font", |ui| {
                            for (i, (name, _)) in BUNDLED_FONTS.iter().enumerate() {
                                let family = egui::FontFamily::Name((*name).into());
                                let row = egui::RichText::new(*name)
                                    .family(family)
                                    .size(18.0);
                                let selected = i == self.current_font_idx;
                                if ui.selectable_label(selected, row).clicked() {
                                    newly_picked = Some(i);
                                }
                            }
                        });
                        ui.separator();
                        // Theme toggle inside the menu — same widget
                        // egui ships, just relocated.
                        egui::widgets::global_theme_preference_buttons(ui);
                    },
                );
            });

            if let Some(i) = newly_picked {
                let (name, _) = BUNDLED_FONTS[i];
                crate::trace::emit(crate::trace::TraceEvent::Note {
                    tag: "ui_font_picked",
                    msg: format!("idx={i} name={name}"),
                });
                self.current_font_idx = i;
                self.write_fonts(&ctx);
            }

            // Use the FULL content rect, not the central panel rect —
            // render_gl wants the canvas drawing buffer dimensions
            // (eframe sizes them from `ctx.content_rect()`), and that's
            // what the Sacred Error on the canvas boundary demands.
            // Top + left panels paint on top of the world afterwards.
            let content = ctx.content_rect();
            let canvas_w = content.width().max(1.0) as u32;
            let canvas_h = content.height().max(1.0) as u32;
            let (x_px, y_px, facing) = crate::wasm_ffi::roam_player_snapshot_impl();
            let zoom = self.zoom;
            // Day brightness — v0.4.1 ships a fixed 1.0 (full daylight).
            // The pre-eframe path computed it JS-side from longitude;
            // moving that derivation Rust-side is a separate slice.
            let day_brightness = 1.0_f32;

            let callback = egui::PaintCallback {
                rect,
                callback: Arc::new(egui_glow::CallbackFn::new(
                    move |_info, _painter| {
                        if let Err(e) = crate::render_gl::render_frame(
                            x_px,
                            y_px,
                            facing,
                            zoom,
                            canvas_w,
                            canvas_h,
                            day_brightness,
                        ) {
                            // Sacred Error: a render failure is the
                            // user not seeing the world. Surfaces in
                            // red in the event log; no demotion.
                            emit_error(
                                Severity::Error,
                                "roam::ui::paint_callback",
                                "render_frame failed inside PaintCallback",
                                format!("{e:?}"),
                            );
                        }
                    },
                )),
            };
            ui.painter().add(callback);
        });

        // Drive a continuous repaint so the world animates (peer
        // markers, day-brightness eventually, etc.) without relying
        // on egui's input-driven repaint heuristic.
        ctx.request_repaint();
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}

/// Wasm entrypoint — JS hands us the canvas, we boot the eframe
/// `WebRunner` on it. Replaces the JS-side `requestAnimationFrame`
/// loop that drove `roam_render_frame`. The world keeps rendering
/// inside the `PaintCallback`; egui paints menus + controls on top.
///
/// `render_gl::init` runs first so the renderer's thread-local
/// `RENDERER` is populated before any frame fires. Both eframe and
/// `render_gl` end up holding handles to the same underlying browser
/// WebGL2 context (the browser returns the same object on repeated
/// `canvas.getContext("webgl2")` calls).
#[wasm_bindgen]
pub fn roam_ui_init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    crate::render_gl::init(canvas.clone())?;

    // Default `WebOptions::should_stop_propagation` returns `true` for
    // every egui event — which means eframe calls `event.stop_propagation()`
    // on every key that hits the canvas, and the JS WASD listener on
    // `window` never fires after the canvas takes focus. Override to
    // `false` so keyboard events continue to bubble; egui still
    // receives them at the canvas-target phase, the world's JS
    // listener still sees them at the bubble phase.
    let web_options = eframe::WebOptions {
        should_stop_propagation: Box::new(|_| false),
        ..Default::default()
    };
    wasm_bindgen_futures::spawn_local(async move {
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| Ok(Box::new(RoamApp::new(cc)))),
            )
            .await;
        if let Err(e) = result {
            emit_error(
                Severity::Error,
                "roam::ui::roam_ui_init",
                "eframe::WebRunner::start failed",
                format!("{e:?}"),
            );
        }
    });
    Ok(())
}
