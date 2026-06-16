#![allow(dead_code)]

use std::cell::RefCell;

use crate::teranos::{FlowerColor, FlowerCore, TileKind};
use crate::trace::{
    drain_json, emit, pending_count, TraceEvent, STATE_READ_COUNT, TICK_BLOCKED_COUNT, TICK_COUNT,
    VIEWPORT_READ_COUNT,
};
use std::sync::atomic::Ordering;
use crate::viewport::{viewport_ptr, write_viewport};
use crate::world::World;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlCanvasElement;

thread_local! {
    static WORLD: RefCell<Option<World>> = const { RefCell::new(None) };
}

pub(crate) fn roam_init_impl() {
    WORLD.with(|w| *w.borrow_mut() = Some(World::new()));
}

pub(crate) fn roam_tick_impl(input: u32, dt_ms: f32) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.step(input, dt_ms);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_tick",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

pub(crate) fn roam_state_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::state_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_state",
                    msg: "called before roam_init; returning {}".to_string(),
                });
                "{}".to_string()
            })
    })
}

pub(crate) fn roam_set_position_impl(x: f32, y: f32, facing: u8) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.set_position(x, y, facing);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_set_position",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

/// Binary viewport FFI. Writes the typed `[ViewportHeader, TileCell × N]`
/// byte sequence into a thread-local buffer and returns the total length.
/// JS reads the buffer through wasm memory at `roam_viewport_ptr()`.
pub(crate) fn roam_viewport_write_impl(view_w: u32, view_h: u32) -> u32 {
    WORLD.with(|w| match w.borrow().as_ref() {
        Some(world) => write_viewport(world, view_w, view_h),
        None => {
            emit(TraceEvent::Note {
                tag: "roam_viewport_write",
                msg: "called before roam_init; returning 0".to_string(),
            });
            0
        }
    })
}

pub(crate) fn roam_viewport_ptr_impl() -> u32 {
    viewport_ptr()
}

/// Color table FFI. Returns a pointer to a static byte buffer of RGB
/// triplets, laid out in the order documented below. Single source of
/// truth: JS / Elm never re-invent RGB on their side.
///
/// Layout, all triplets are u8[3]:
///
///   [0..15)  TileKind:    Air, Grass, Rock, ShallowWater, DeepWater
///   [15..36) FlowerColor: Red, Yellow, Blue, Purple, Azure, Pink, Glow
///   [36..45) FlowerCore:  White, Yellow, Black
///
/// The discriminant of each enum value matches its position in the
/// table; JS indexes as `palette[3 * (kind_offset + discriminant) + c]`.
pub(crate) const COLOR_TABLE_LEN: u32 = 45;
pub(crate) const COLOR_TABLE_TILE_OFFSET: u32 = 0;
pub(crate) const COLOR_TABLE_PETAL_OFFSET: u32 = 15;
pub(crate) const COLOR_TABLE_CORE_OFFSET: u32 = 36;

thread_local! {
    static COLOR_TABLE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn roam_color_table_ptr_impl() -> u32 {
    COLOR_TABLE.with(|t| {
        let mut buf = t.borrow_mut();
        if buf.is_empty() {
            for tk in [
                TileKind::Air,
                TileKind::Grass,
                TileKind::Rock,
                TileKind::ShallowWater,
                TileKind::DeepWater,
            ] {
                buf.extend_from_slice(&tk.rgb());
            }
            for fc in [
                FlowerColor::Red,
                FlowerColor::Yellow,
                FlowerColor::Blue,
                FlowerColor::Purple,
                FlowerColor::Azure,
                FlowerColor::Pink,
                FlowerColor::Glow,
            ] {
                buf.extend_from_slice(&fc.rgb());
            }
            for c in [FlowerCore::White, FlowerCore::Yellow, FlowerCore::Black] {
                buf.extend_from_slice(&c.rgb());
            }
            debug_assert_eq!(buf.len() as u32, COLOR_TABLE_LEN);
        }
        buf.as_ptr() as u32
    })
}

pub(crate) fn roam_color_table_len_impl() -> u32 {
    COLOR_TABLE_LEN
}

pub(crate) fn roam_pixels_per_tile_impl() -> u32 {
    crate::world::PIXELS_PER_TILE
}

// ----- per-frame counters (replaced the per-frame trace events) -----

pub(crate) fn roam_tick_count_impl() -> u64 {
    TICK_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_tick_blocked_count_impl() -> u64 {
    TICK_BLOCKED_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_state_read_count_impl() -> u64 {
    STATE_READ_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_viewport_read_count_impl() -> u64 {
    VIEWPORT_READ_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_drain_trace_impl() -> String {
    drain_json()
}

pub(crate) fn roam_trace_pending_count_impl() -> u32 {
    pending_count() as u32
}

pub(crate) fn roam_drain_errors_impl() -> String {
    let errors = crate::error::drain();
    serde_json::to_string(&errors).unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn roam_session_snapshot_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::session_snapshot_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_session_snapshot",
                    msg: "called before roam_init; returning empty".to_string(),
                });
                r#"{"picked":[],"inv":[]}"#.to_string()
            })
    })
}

pub(crate) fn roam_restore_session_impl(raw: String) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.restore_session_json(&raw);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_restore_session",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::*;

    #[wasm_bindgen]
    pub fn roam_init() {
        super::roam_init_impl();
    }

    #[wasm_bindgen]
    pub fn roam_tick(input: u32, dt_ms: f32) {
        super::roam_tick_impl(input, dt_ms);
    }

    #[wasm_bindgen]
    pub fn roam_state() -> String {
        super::roam_state_impl()
    }

    #[wasm_bindgen]
    pub fn roam_viewport_write(view_w: u32, view_h: u32) -> u32 {
        super::roam_viewport_write_impl(view_w, view_h)
    }

    #[wasm_bindgen]
    pub fn roam_viewport_ptr() -> u32 {
        super::roam_viewport_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_color_table_ptr() -> u32 {
        super::roam_color_table_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_color_table_len() -> u32 {
        super::roam_color_table_len_impl()
    }

    #[wasm_bindgen]
    pub fn roam_pixels_per_tile() -> u32 {
        super::roam_pixels_per_tile_impl()
    }

    #[wasm_bindgen]
    pub fn roam_tick_count() -> u64 {
        super::roam_tick_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_tick_blocked_count() -> u64 {
        super::roam_tick_blocked_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_state_read_count() -> u64 {
        super::roam_state_read_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_viewport_read_count() -> u64 {
        super::roam_viewport_read_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_set_position(x: f32, y: f32, facing: u8) {
        super::roam_set_position_impl(x, y, facing);
    }

    #[wasm_bindgen]
    pub fn roam_drain_trace() -> String {
        super::roam_drain_trace_impl()
    }

    #[wasm_bindgen]
    pub fn roam_trace_pending_count() -> u32 {
        super::roam_trace_pending_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_session_snapshot() -> String {
        super::roam_session_snapshot_impl()
    }

    #[wasm_bindgen]
    pub fn roam_restore_session(raw: String) {
        super::roam_restore_session_impl(raw);
    }

    #[wasm_bindgen]
    pub fn roam_drain_errors() -> String {
        super::roam_drain_errors_impl()
    }

    // ----- S4a: WebGL2 renderer wire -----

    #[wasm_bindgen]
    pub fn roam_render_init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
        crate::render_gl::init(canvas)
    }

    #[wasm_bindgen]
    pub fn roam_render_frame(
        player_x_px: f32,
        player_y_px: f32,
        zoom: f32,
        canvas_w: u32,
        canvas_h: u32,
        day_brightness: f32,
    ) -> Result<(), JsValue> {
        crate::render_gl::render_frame(
            player_x_px,
            player_y_px,
            zoom,
            canvas_w,
            canvas_h,
            day_brightness,
        )
    }
}
