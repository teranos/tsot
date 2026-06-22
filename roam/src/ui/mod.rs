//! In-canvas UI built on eframe + egui. Decisions live in `docs/UI.md`.

#![cfg(target_arch = "wasm32")]

use std::sync::Arc;

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::error::{emit as emit_error, Severity};

/// Embedded font catalog. Name is the `FontFamily::Name` the picker
/// exposes; bytes are static so the wasm `data` section holds one
/// copy.
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
    current_font_idx: usize,
    zoom: f32,
    /// Extended-slots grid above the hotbar. Toggled by Tab.
    inventory_open: bool,
}

const HOTBAR_SLOTS: usize = 9;
const EXTENDED_ROWS: usize = 3;
const TOTAL_SLOTS: usize = HOTBAR_SLOTS + EXTENDED_ROWS * HOTBAR_SLOTS;
const SLOT_PX: f32 = 40.0;
const SLOT_GAP: f32 = 4.0;

const ZOOM_STEP: f32 = 1.25;
const ZOOM_MIN: f32 = 0.4;
const ZOOM_MAX: f32 = 4.0;

impl RoamApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let app = Self {
            current_font_idx: 0,
            zoom: 1.0,
            inventory_open: false,
        };
        app.write_fonts(&cc.egui_ctx);
        app
    }

    fn draw_slot(ui: &mut egui::Ui, item: Option<&crate::teranos::Pickup>) {
        let (rect, _resp) = ui.allocate_exact_size(
            egui::Vec2::splat(SLOT_PX),
            egui::Sense::hover(),
        );
        let painter = ui.painter();
        let bg = match item {
            None => egui::Color32::from_rgba_premultiplied(20, 20, 30, 180),
            Some(crate::teranos::Pickup::Flower(_)) => {
                egui::Color32::from_rgb(60, 90, 60)
            }
            Some(crate::teranos::Pickup::Card(_)) => {
                egui::Color32::from_rgb(80, 70, 110)
            }
        };
        painter.rect_filled(rect, 4.0, bg);
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(120)),
            egui::StrokeKind::Outside,
        );
        if let Some(p) = item {
            let label = match p {
                crate::teranos::Pickup::Flower(f) => format!("F{}", f.petal_count),
                crate::teranos::Pickup::Card(id) => {
                    let s: String = id.0.chars().take(3).collect();
                    s
                }
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::monospace(13.0),
                egui::Color32::WHITE,
            );
        }
    }

    /// Each bundled font is registered as its own `FontFamily::Name`
    /// (so the picker can preview each row in its own typeface), and
    /// the selected font is prepended to `Proportional` so default
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
        let input = crate::input::FrameInput::read(ctx);
        crate::wasm_ffi::roam_tick_impl(input.move_bits, input.dt_ms);
        if input.spawn_pressed {
            let px = 16.0 * crate::world::PIXELS_PER_TILE as f32;
            crate::wasm_ffi::roam_set_position_impl(px, px, 4);
        }
        if input.inventory_toggle_pressed {
            self.inventory_open = !self.inventory_open;
        }
        if input.zoom_in_pressed {
            self.zoom = (self.zoom * ZOOM_STEP).min(ZOOM_MAX);
        }
        if input.zoom_out_pressed {
            self.zoom = (self.zoom / ZOOM_STEP).max(ZOOM_MIN);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        let items = crate::wasm_ffi::roam_inventory_snapshot_impl();
        let slot_for = |idx: usize| -> Option<&crate::teranos::Pickup> { items.get(idx) };

        egui::Area::new(egui::Id::new("roam_hotbar"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::Vec2::new(0.0, -12.0))
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                ui.spacing_mut().item_spacing.x = SLOT_GAP;
                ui.horizontal(|ui| {
                    for i in 0..HOTBAR_SLOTS {
                        Self::draw_slot(ui, slot_for(i));
                    }
                });
            });

        if self.inventory_open {
            egui::Area::new(egui::Id::new("roam_inventory_extended"))
                .anchor(
                    egui::Align2::CENTER_BOTTOM,
                    egui::Vec2::new(
                        0.0,
                        -(SLOT_PX + SLOT_GAP + 12.0 + SLOT_GAP),
                    ),
                )
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::splat(SLOT_GAP);
                    ui.vertical(|ui| {
                        for row in 0..EXTENDED_ROWS {
                            ui.horizontal(|ui| {
                                for col in 0..HOTBAR_SLOTS {
                                    let idx =
                                        HOTBAR_SLOTS + row * HOTBAR_SLOTS + col;
                                    if idx < TOTAL_SLOTS {
                                        Self::draw_slot(ui, slot_for(idx));
                                    }
                                }
                            });
                        }
                    });
                });
        }

        // Wall clock — top-right corner. Same 24-hour format the JS-side
        // strip used ("HH:MM:SS.mmm  GMT±HHMM") so screenshots stay
        // correlatable with the prior surface.
        egui::Area::new(egui::Id::new("roam_clock"))
            .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::new(-6.0, 6.0))
            .interactable(false)
            .show(&ctx, |ui| {
                let d = js_sys::Date::new_0();
                let iso = d.to_iso_string().as_string().unwrap_or_default();
                let time = iso.get(11..23).unwrap_or("");
                let s = d.to_string().as_string().unwrap_or_default();
                let tz = s.get(25..33).unwrap_or("");
                ui.label(
                    egui::RichText::new(format!("{time}  {tz}"))
                        .monospace()
                        .size(11.0)
                        .color(egui::Color32::from_gray(150)),
                );
            });

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

        let mut newly_picked: Option<usize> = None;

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(
                ui.available_size(),
                egui::Sense::click_and_drag(),
            );

            // Width-pin recipe keeps menu items from re-wrapping when
            // the active font changes.
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

            // `content_rect` matches the canvas drawing buffer (eframe
            // sizes it that way); using the inner panel rect instead
            // would trip the canvas-dim Sacred Error every frame.
            let content = ctx.content_rect();
            let canvas_w = content.width().max(1.0) as u32;
            let canvas_h = content.height().max(1.0) as u32;
            let (x_px, y_px, facing) = crate::wasm_ffi::roam_player_snapshot_impl();
            let zoom = self.zoom;
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

        // Force continuous repaint — egui's input-driven heuristic
        // wouldn't animate the world otherwise.
        ctx.request_repaint();
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(&mut *self)
    }
}

/// Wasm entrypoint — boots eframe's `WebRunner` on the canvas.
/// `render_gl::init` runs first so the renderer's thread-local is
/// populated before any frame fires; eframe and `render_gl` both
/// hold handles to the same underlying browser WebGL2 context (the
/// browser returns the same object on repeated `getContext("webgl2")`).
#[wasm_bindgen]
pub fn roam_ui_init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    crate::render_gl::init(canvas.clone())?;

    // egui's default stops propagation on every key, which kills the
    // JS-side keyboard listeners (zoom +/-, log shortcuts) once the
    // canvas has focus. Letting events bubble keeps both pipelines
    // alive — egui handles them at the canvas target phase, JS at
    // the window bubble phase.
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
