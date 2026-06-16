#![allow(dead_code)]

use std::cell::RefCell;
use std::ops::Deref;

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

// ----- binary player-state FFI -----
//
// JSON-parsing the state every frame was a measurable cost in the
// Firefox profiler. The dirty-flag fingerprint + GL render only need
// (x, y, z, facing); the inventory still ships as JSON but the
// throttled HUD path reads it once every 500ms, not per frame.
//
// Layout (16 bytes, little-endian, repr(C)):
//   offset 0  f32  x
//   offset 4  f32  y
//   offset 8  i32  z
//   offset 12 u8   facing
//   offset 13 u8[3] padding
//
// `roam_player_state_ptr` snapshots into a thread-local static buffer
// and returns its base pointer. Caller reads via DataView; the buffer
// is overwritten on every call.

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct PlayerStateBinary {
    x: f32,
    y: f32,
    z: i32,
    facing: u8,
    _pad: [u8; 3],
}

const _: () = assert!(core::mem::size_of::<PlayerStateBinary>() == 16);

pub(crate) const PLAYER_STATE_LEN: u32 = 16;

thread_local! {
    static PLAYER_STATE_BUFFER: RefCell<PlayerStateBinary> =
        const { RefCell::new(PlayerStateBinary {
            x: 0.0,
            y: 0.0,
            z: 0,
            facing: 4,
            _pad: [0; 3],
        }) };
}

pub(crate) fn roam_player_state_ptr_impl() -> u32 {
    WORLD.with(|w| {
        if let Some(world) = w.borrow().as_ref() {
            PLAYER_STATE_BUFFER.with(|cell| {
                let mut s = cell.borrow_mut();
                s.x = world.player.x;
                s.y = world.player.y;
                s.z = world.player.z;
                s.facing = world.player.facing.as_u8();
            });
        }
        PLAYER_STATE_BUFFER.with(|cell| cell.borrow().deref() as *const _ as u32)
    })
}

pub(crate) fn roam_player_state_len_impl() -> u32 {
    PLAYER_STATE_LEN
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
    pub fn roam_player_state_ptr() -> u32 {
        super::roam_player_state_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_player_state_len() -> u32 {
        super::roam_player_state_len_impl()
    }

    /// Construct the application-layer network state and attach it
    /// to `World`. Called once from the JS bridge after libp2p is up.
    ///
    /// The five JS callbacks form the seam between the trait
    /// (`crate::net::NetworkProvider`) and the existing libp2p
    /// instance. Future provider impls (`RustLibp2pProvider`,
    /// `RemoteServerProvider`) replace this constructor; the rest of
    /// the application never sees a JS function again.
    #[wasm_bindgen]
    pub fn roam_net_init(
        self_peer_id_fn: js_sys::Function,
        publish: js_sys::Function,
        subscribe: js_sys::Function,
        unsubscribe: js_sys::Function,
        drain_events: js_sys::Function,
    ) -> Result<(), JsValue> {
        let provider = crate::net::js_libp2p::JsLibp2pProvider::new(
            self_peer_id_fn,
            publish,
            subscribe,
            unsubscribe,
            drain_events,
        )?;
        let mut net = crate::net::state::Net::new(Box::new(provider));
        if let Err(err) = net.subscribe_positions() {
            crate::error::emit(
                crate::error::Severity::Error,
                "roam::wasm_ffi::roam_net_init",
                "subscribe_positions failed",
                format!("{err:?}"),
            );
        }
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow_mut().as_mut() {
                world.net = Some(net);
            } else {
                crate::error::emit(
                    crate::error::Severity::Error,
                    "roam::wasm_ffi::roam_net_init",
                    "called before roam_init",
                    "World is not constructed; Net not attached",
                );
            }
        });
        Ok(())
    }

    /// Drain provider events, update the peer table, prune stale
    /// peers. Called once per frame from the JS bridge. `now_ms` is
    /// the JS-side `Date.now()` value (f64 to avoid BigInt at the
    /// boundary; truncated to u64 inside).
    #[wasm_bindgen]
    pub fn roam_net_tick(now_ms: f64) {
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow_mut().as_mut() {
                if let Some(net) = world.net.as_mut() {
                    net.tick(now_ms as u64);
                }
            }
        });
    }

    /// Number of remote peers currently in the Rust-owned peer table.
    /// Phase 2c diagnostic — confirms the seam is actually receiving
    /// events. Removed once the renderer reads from `Net.peers`
    /// directly (phase 2d).
    #[wasm_bindgen]
    pub fn roam_net_peer_count() -> u32 {
        super::WORLD.with(|w| {
            w.borrow()
                .as_ref()
                .and_then(|world| world.net.as_ref().map(|n| n.peers().count() as u32))
                .unwrap_or(0)
        })
    }

    /// Publish the local player's current position on the canonical
    /// positions topic. Called from the JS bridge's broadcast timer.
    /// No-op if `Net` hasn't been attached yet (libp2p still booting).
    #[wasm_bindgen]
    pub fn roam_net_publish_position() {
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow_mut().as_mut() {
                let (x, y, z, facing) = (
                    world.player.x,
                    world.player.y,
                    world.player.z,
                    world.player.facing.as_u8(),
                );
                if let Some(net) = world.net.as_mut() {
                    if let Err(err) = net.publish_position(x, y, z, facing) {
                        crate::error::emit(
                            crate::error::Severity::Warn,
                            "roam::wasm_ffi::roam_net_publish_position",
                            "publish_position failed",
                            format!("{err:?}"),
                        );
                    }
                }
            }
        });
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
        facing: u8,
        zoom: f32,
        canvas_w: u32,
        canvas_h: u32,
        day_brightness: f32,
    ) -> Result<(), JsValue> {
        crate::render_gl::render_frame(
            player_x_px,
            player_y_px,
            facing,
            zoom,
            canvas_w,
            canvas_h,
            day_brightness,
        )
    }

    /// Publish the current peer list to the renderer. The packed array
    /// is `[x0, y0, src0, x1, y1, src1, ...]` with `src` as 0.0 for
    /// libp2p and 1.0 for BroadcastChannel. Called once per frame
    /// before `roam_render_frame`.
    #[wasm_bindgen]
    pub fn roam_set_peers(packed: &[f32]) {
        crate::render_gl::set_peers(packed);
    }
}
