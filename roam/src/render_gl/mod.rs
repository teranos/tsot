//! WebGL2 renderer for the world canvas.
//!
//! Rust owns the entire render pipeline — context, shaders, buffers,
//! draw calls. JS hands us the canvas once at init and never touches a
//! pixel after. `web_sys`'s typed WebGL bindings still compile to JS
//! calls under the hood, but the bridge's draw surface in JS is zero
//! (one `roam_render(...)` per frame).
//!
//! Per-pass code (programs, shaders, build fns, helpers) lives in
//! sibling submodules; this file is the orchestrator:
//! - `helpers` — `compile_program`, `compile_shader`, `get_uniform`,
//!   `create_buffer_with_data`, `rgb_to_floats`
//! - `tile`    — `TileProgram` + tile shaders + `tile_palette_floats`
//! - `flower`  — `FlowerProgram` + flower shaders + color helpers
//! - `card`    — `CardProgram` + card shaders + `card_color_rgb`
//! - `marker`  — `MarkerProgram` + marker shaders
//! - `line`    — `LineProgram` + line shaders + `push_line` +
//!   `facing_unit_vec` + cliff / arrow constants

#![cfg(target_arch = "wasm32")]

mod card;
mod flower;
mod helpers;
mod line;
mod marker;
mod tile;

use std::cell::RefCell;

use js_sys::Float32Array;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use crate::error::{emit as emit_error, Severity};
use crate::teranos::CoreEdge;
use crate::viewport::{viewport_ptr, VIEWPORT_HEADER_SIZE, VIEWPORT_TILE_SIZE};

use card::{build_card_program, card_color_rgb, CardProgram};
use flower::{
    build_flower_program, core_edge_from_u8, flower_color_rgb, flower_core_rgb, FlowerProgram,
};
use line::{
    build_line_program, facing_unit_vec, push_line, LineProgram, CLIFF_RGBA, CLIFF_THRESHOLD,
    FACING_ARROW_RGBA,
};
use marker::{build_marker_program, MarkerProgram};
use tile::{build_tile_program, tile_palette_floats, TileProgram};

thread_local! {
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
    /// Peer positions + sources, refilled by the JS bridge each frame
    /// via `set_peers`. Storage layout matches the wire format: 3
    /// floats per peer = (world_x, world_y, source). `source` is 0.0
    /// for libp2p, 1.0 for BroadcastChannel; Rust picks the color.
    static PEER_STATE: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

/// Source-of-peer-data tag, matching the `f32` source field passed
/// through `set_peers`. Color picked from this in the marker pass.
const PEER_SOURCE_LIBP2P: f32 = 0.0;
const PEER_SOURCE_BROADCASTCHANNEL: f32 = 1.0;

/// Color triplets — RGB in 0..1. Single source of truth for marker
/// colors lives in Rust (consistent with the tile/flower palette).
const PLAYER_MARKER_RGB: [f32; 3] = [0.4, 0.8, 1.0]; // #6cf
const PEER_LIBP2P_RGB: [f32; 3] = [1.0, 0.4, 0.66]; // #f6a
const PEER_BROADCAST_RGB: [f32; 3] = [1.0, 0.66, 0.4]; // #fa6

/// Called by the JS bridge before each `render_frame` to publish the
/// current peer list. The slice is `[x0, y0, src0, x1, y1, src1, ...]`.
/// Anything not on this list disappears from the next frame's draw.
pub fn set_peers(packed: &[f32]) {
    if !packed.len().is_multiple_of(3) {
        emit_error(
            Severity::Warn,
            "roam::render_gl::set_peers",
            "peer array length not a multiple of 3",
            format!(
                "got {} floats; expected groups of (x, y, source)",
                packed.len()
            ),
        );
        return;
    }
    PEER_STATE.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(packed);
    });
}

pub struct Renderer {
    gl: Gl,
    canvas: HtmlCanvasElement,
    tile_prog: TileProgram,
    flower_prog: FlowerProgram,
    card_prog: CardProgram,
    marker_prog: MarkerProgram,
    line_prog: LineProgram,
    /// Per-frame scratch buffer for tile instance attributes
    /// (tile_kind, elev_offset). Two floats per visible tile.
    tile_instance_scratch: Vec<f32>,
    /// Per-frame scratch buffer for flower instance attributes
    /// (world_tile.xy, petal_center.rgb, petal_edge.rgb,
    /// core_center.rgb, core_edge.rgb, petal_count). 15 floats per
    /// visible flower; matches `FlowerProgram::INSTANCE_STRIDE_FLOATS`.
    flower_instance_scratch: Vec<f32>,
    /// Per-frame scratch buffer for card instance attributes
    /// (world_tile.xy, color.rgb). 5 floats per visible card; matches
    /// `CardProgram::INSTANCE_STRIDE_FLOATS`. Cards don't share the
    /// flower buffer: separate program, separate shader, separate
    /// geometry (rectangle vs flower).
    card_instance_scratch: Vec<f32>,
    /// Marker positions populated by `roam_set_peers` from JS and
    /// drained each frame in the marker pass. Storage is
    /// (world_x, world_y, r, g, b, size_world_px) per marker, matching
    /// `MarkerProgram::INSTANCE_STRIDE_FLOATS`.
    marker_instance_scratch: Vec<f32>,
    /// Per-frame scratch buffer for line vertices. Layout matches
    /// `LineProgram::VERTEX_STRIDE_FLOATS`: 6 floats per vertex
    /// (world_x, world_y, r, g, b, a). Two vertices per line segment.
    /// Used for the facing arrow and cliff outlines.
    line_vertex_scratch: Vec<f32>,
}

/// Acquire WebGL2 from the canvas, compile shaders, allocate buffers.
/// Caller is the JS bridge at init. If WebGL2 is unavailable, the
/// failure surfaces through the sacred-error log rather than being
/// swallowed.
pub fn init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
    let ctx = canvas.get_context("webgl2")?.ok_or_else(|| {
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::init",
            "WebGL2 unavailable",
            "canvas.getContext('webgl2') returned null on this browser",
        );
        JsValue::from_str(&err.why)
    })?;
    let gl = ctx.dyn_into::<Gl>().map_err(|_| {
        let err = emit_error(
            Severity::Error,
            "roam::render_gl::init",
            "WebGL2 context downcast failed",
            "getContext('webgl2') returned an object that isn't a WebGl2RenderingContext",
        );
        JsValue::from_str(&err.why)
    })?;

    // Alpha blending for soft flower edges.
    gl.enable(Gl::BLEND);
    gl.blend_func(Gl::SRC_ALPHA, Gl::ONE_MINUS_SRC_ALPHA);

    let tile_prog = build_tile_program(&gl)?;
    let flower_prog = build_flower_program(&gl)?;
    let card_prog = build_card_program(&gl)?;
    let marker_prog = build_marker_program(&gl)?;
    let line_prog = build_line_program(&gl)?;

    RENDERER.with(|cell| {
        *cell.borrow_mut() = Some(Renderer {
            gl,
            canvas,
            tile_prog,
            flower_prog,
            card_prog,
            marker_prog,
            line_prog,
            tile_instance_scratch: Vec::new(),
            flower_instance_scratch: Vec::new(),
            card_instance_scratch: Vec::new(),
            marker_instance_scratch: Vec::new(),
            line_vertex_scratch: Vec::new(),
        });
    });
    Ok(())
}

/// Clone the WebGL2 context the renderer is using. Used by
/// `roam::ui::init_for_canvas` so egui's painter shares the same
/// GL state as the world renderer — no second `getContext`, no
/// separate state to keep in sync. Returns None if `init` hasn't run.
pub fn gl_context() -> Option<Gl> {
    RENDERER.with(|cell| cell.borrow().as_ref().map(|r| r.gl.clone()))
}

pub fn render_frame(
    player_x_px: f32,
    player_y_px: f32,
    facing: u8,
    zoom: f32,
    canvas_w: u32,
    canvas_h: u32,
    day_brightness: f32,
) -> Result<(), JsValue> {
    RENDERER.with(|cell| {
        let mut opt = cell.borrow_mut();
        let Some(renderer) = opt.as_mut() else {
            return Err(JsValue::from_str(
                "roam::render_gl: renderer not initialized — call roam_render_init first",
            ));
        };

        if renderer.canvas.width() != canvas_w {
            renderer.canvas.set_width(canvas_w);
        }
        if renderer.canvas.height() != canvas_h {
            renderer.canvas.set_height(canvas_h);
        }
        renderer.gl.viewport(0, 0, canvas_w as i32, canvas_h as i32);

        // Read the viewport header out of wasm memory.
        let view_w;
        let view_h;
        let center_tx;
        let center_ty;
        let world_px_per_tile;
        unsafe {
            let ptr = viewport_ptr() as *const u8;
            view_w = read_u32_le(ptr, 0);
            view_h = read_u32_le(ptr, 4);
            center_tx = read_i32_le(ptr, 8);
            center_ty = read_i32_le(ptr, 12);
            world_px_per_tile = read_u32_le(ptr, 20) as f32;
        }

        // Build tile instance attributes + collect flower instances +
        // collect cliff line segments in a single pass over the
        // viewport buffer. `elev_offset` is already a signed delta
        // from player.z so neighbor comparisons are direct.
        let tile_count = (view_w * view_h) as usize;
        let half_w = view_w as i32 / 2;
        let half_h = view_h as i32 / 2;
        let tile_scratch = &mut renderer.tile_instance_scratch;
        tile_scratch.clear();
        tile_scratch.reserve(tile_count * 2);
        let flower_scratch = &mut renderer.flower_instance_scratch;
        flower_scratch.clear();
        let card_scratch = &mut renderer.card_instance_scratch;
        card_scratch.clear();
        let line_scratch = &mut renderer.line_vertex_scratch;
        line_scratch.clear();
        unsafe {
            let ptr = viewport_ptr() as *const u8;
            for i in 0..tile_count {
                let off = VIEWPORT_HEADER_SIZE + i * VIEWPORT_TILE_SIZE;
                let tile_kind = *ptr.add(off) as f32;
                let elev_offset = *ptr.add(off + 1) as i8 as f32;
                tile_scratch.push(tile_kind);
                tile_scratch.push(elev_offset);

                let pickup_kind = *ptr.add(off + 2);
                let vx = (i as i32) % (view_w as i32);
                let vy = (i as i32) / (view_w as i32);
                let world_tx = (center_tx + vx - half_w) as f32;
                let world_ty = (center_ty + vy - half_h) as f32;
                if pickup_kind == crate::viewport::PICKUP_KIND_FLOWER {
                    let petal_center = *ptr.add(off + 3);
                    let petal_edge = *ptr.add(off + 4);
                    let core_center = *ptr.add(off + 5);
                    let core_edge_kind = *ptr.add(off + 6);
                    let petal_count = *ptr.add(off + 7);

                    let pc_rgb = flower_color_rgb(petal_center);
                    let pe_rgb = flower_color_rgb(petal_edge);
                    let cc_rgb = flower_core_rgb(core_center);
                    let ce_rgb = match core_edge_from_u8(core_edge_kind) {
                        Some(CoreEdge::White) => [1.0, 1.0, 1.0],
                        Some(CoreEdge::MatchPetalCenter) => pc_rgb,
                        Some(CoreEdge::MatchPetalEdge) => pe_rgb,
                        None => [1.0, 1.0, 1.0],
                    };

                    flower_scratch.push(world_tx);
                    flower_scratch.push(world_ty);
                    flower_scratch.extend_from_slice(&pc_rgb);
                    flower_scratch.extend_from_slice(&pe_rgb);
                    flower_scratch.extend_from_slice(&cc_rgb);
                    flower_scratch.extend_from_slice(&ce_rgb);
                    flower_scratch.push(petal_count as f32);
                } else if pickup_kind == crate::viewport::PICKUP_KIND_CARD {
                    let card_seed = u32::from_le_bytes([
                        *ptr.add(off + 8),
                        *ptr.add(off + 9),
                        *ptr.add(off + 10),
                        *ptr.add(off + 11),
                    ]);
                    let color = card_color_rgb(card_seed);
                    card_scratch.push(world_tx);
                    card_scratch.push(world_ty);
                    card_scratch.extend_from_slice(&color);
                }

                // Cliff outlines: where the local |Δelev| with the
                // right or down neighbor exceeds the walkable step
                // (CLIFF_THRESHOLD), draw a line along that boundary.
                let world_x0 = world_tx * world_px_per_tile;
                let world_y0 = world_ty * world_px_per_tile;
                let world_x1 = world_x0 + world_px_per_tile;
                let world_y1 = world_y0 + world_px_per_tile;

                if vx + 1 < view_w as i32 {
                    let off_r = VIEWPORT_HEADER_SIZE
                        + ((vy as usize) * (view_w as usize) + (vx as usize) + 1)
                            * VIEWPORT_TILE_SIZE;
                    let e_r = *ptr.add(off_r + 1) as i8 as f32;
                    if (e_r - elev_offset).abs() > CLIFF_THRESHOLD {
                        push_line(
                            line_scratch,
                            world_x1,
                            world_y0,
                            world_x1,
                            world_y1,
                            CLIFF_RGBA,
                        );
                    }
                }
                if vy + 1 < view_h as i32 {
                    let off_d = VIEWPORT_HEADER_SIZE
                        + (((vy as usize) + 1) * (view_w as usize) + (vx as usize))
                            * VIEWPORT_TILE_SIZE;
                    let e_d = *ptr.add(off_d + 1) as i8 as f32;
                    if (e_d - elev_offset).abs() > CLIFF_THRESHOLD {
                        push_line(
                            line_scratch,
                            world_x0,
                            world_y1,
                            world_x1,
                            world_y1,
                            CLIFF_RGBA,
                        );
                    }
                }
            }
        }

        // Facing arrow as a line from the player to player + facing
        // direction × marker world size. Same green as the canvas2D
        // stroke (#cfc).
        let (fdx, fdy) = facing_unit_vec(facing);
        let arrow_world_size = (14.0_f32 * zoom).clamp(8.0, 32.0) / zoom;
        let arrow_end_x = player_x_px + fdx * arrow_world_size;
        let arrow_end_y = player_y_px + fdy * arrow_world_size;
        push_line(
            line_scratch,
            player_x_px,
            player_y_px,
            arrow_end_x,
            arrow_end_y,
            FACING_ARROW_RGBA,
        );

        let gl = &renderer.gl;

        // ----- clear + tile pass -----
        gl.clear_color(0.0627, 0.0627, 0.0745, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);

        let tile_prog = &renderer.tile_prog;
        gl.bind_vertex_array(Some(&tile_prog.vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&tile_prog.instance_buffer));
        unsafe {
            let view = Float32Array::view(tile_scratch);
            gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
        }
        gl.use_program(Some(&tile_prog.program));
        gl.uniform2f(Some(&tile_prog.u_camera_px), player_x_px, player_y_px);
        gl.uniform2f(Some(&tile_prog.u_canvas_px), canvas_w as f32, canvas_h as f32);
        gl.uniform1f(Some(&tile_prog.u_world_px_per_tile), world_px_per_tile);
        gl.uniform1f(Some(&tile_prog.u_zoom), zoom);
        gl.uniform4i(
            Some(&tile_prog.u_view_dim),
            view_w as i32,
            view_h as i32,
            center_tx,
            center_ty,
        );
        gl.uniform1f(Some(&tile_prog.u_day_brightness), day_brightness);
        let tile_palette = tile_palette_floats();
        gl.uniform3fv_with_f32_array(Some(&tile_prog.u_tile_palette), &tile_palette);
        gl.draw_elements_instanced_with_i32(
            Gl::TRIANGLES,
            6,
            Gl::UNSIGNED_SHORT,
            0,
            tile_count as i32,
        );

        // ----- flower pass -----
        let flower_count = flower_scratch.len() / FlowerProgram::INSTANCE_STRIDE_FLOATS;
        if flower_count > 0 {
            let flower_prog = &renderer.flower_prog;
            gl.bind_vertex_array(Some(&flower_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&flower_prog.instance_buffer));
            unsafe {
                let view = Float32Array::view(flower_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&flower_prog.program));
            gl.uniform2f(Some(&flower_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&flower_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&flower_prog.u_world_px_per_tile), world_px_per_tile);
            gl.uniform1f(Some(&flower_prog.u_zoom), zoom);
            gl.uniform1f(Some(&flower_prog.u_day_brightness), day_brightness);
            gl.draw_elements_instanced_with_i32(
                Gl::TRIANGLES,
                6,
                Gl::UNSIGNED_SHORT,
                0,
                flower_count as i32,
            );
        }

        // ----- card pass -----
        let card_count = card_scratch.len() / CardProgram::INSTANCE_STRIDE_FLOATS;
        if card_count > 0 {
            let card_prog = &renderer.card_prog;
            gl.bind_vertex_array(Some(&card_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&card_prog.instance_buffer));
            unsafe {
                let view = Float32Array::view(card_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&card_prog.program));
            gl.uniform2f(Some(&card_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&card_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&card_prog.u_world_px_per_tile), world_px_per_tile);
            gl.uniform1f(Some(&card_prog.u_zoom), zoom);
            gl.uniform1f(Some(&card_prog.u_day_brightness), day_brightness);
            gl.draw_elements_instanced_with_i32(
                Gl::TRIANGLES,
                6,
                Gl::UNSIGNED_SHORT,
                0,
                card_count as i32,
            );
        }

        // ----- marker pass -----
        //
        // Player marker is always first; remote peers follow. Marker
        // size matches the original canvas2D logic: 14 screen px at
        // zoom 1, clamped to [8, 32]. Convert to world pixels so
        // we can place the quad in world-space and reuse the zoom
        // uniform path.
        let marker_screen_size = (14.0_f32 * zoom).clamp(8.0, 32.0);
        let marker_world_size = marker_screen_size / zoom;
        let marker_scratch = &mut renderer.marker_instance_scratch;
        marker_scratch.clear();

        // Player.
        marker_scratch.push(player_x_px);
        marker_scratch.push(player_y_px);
        marker_scratch.extend_from_slice(&PLAYER_MARKER_RGB);
        marker_scratch.push(marker_world_size);

        // Peers.
        PEER_STATE.with(|cell| {
            let peers = cell.borrow();
            for chunk in peers.chunks_exact(3) {
                let x = chunk[0];
                let y = chunk[1];
                let source = chunk[2];
                marker_scratch.push(x);
                marker_scratch.push(y);
                let rgb = if (source - PEER_SOURCE_LIBP2P).abs() < f32::EPSILON {
                    PEER_LIBP2P_RGB
                } else if (source - PEER_SOURCE_BROADCASTCHANNEL).abs() < f32::EPSILON {
                    PEER_BROADCAST_RGB
                } else {
                    // Unknown source — surface and fall back to magenta.
                    emit_error(
                        Severity::Warn,
                        "roam::render_gl::render_frame",
                        "peer source tag out of range",
                        format!(
                            "expected 0.0 (libp2p) or 1.0 (BroadcastChannel), got {source}"
                        ),
                    );
                    [1.0, 0.0, 1.0]
                };
                marker_scratch.extend_from_slice(&rgb);
                marker_scratch.push(marker_world_size);
            }
        });

        let marker_count = marker_scratch.len() / MarkerProgram::INSTANCE_STRIDE_FLOATS;
        if marker_count > 0 {
            let marker_prog = &renderer.marker_prog;
            gl.bind_vertex_array(Some(&marker_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&marker_prog.instance_buffer));
            unsafe {
                let view = Float32Array::view(marker_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&marker_prog.program));
            gl.uniform2f(Some(&marker_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&marker_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&marker_prog.u_zoom), zoom);
            gl.draw_elements_instanced_with_i32(
                Gl::TRIANGLES,
                6,
                Gl::UNSIGNED_SHORT,
                0,
                marker_count as i32,
            );
        }

        // ----- line pass (cliffs + facing arrow) -----
        let line_vertex_count = line_scratch.len() / LineProgram::VERTEX_STRIDE_FLOATS;
        if line_vertex_count > 0 {
            let line_prog = &renderer.line_prog;
            gl.bind_vertex_array(Some(&line_prog.vao));
            gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&line_prog.vertex_buffer));
            unsafe {
                let view = Float32Array::view(line_scratch);
                gl.buffer_data_with_array_buffer_view(Gl::ARRAY_BUFFER, &view, Gl::STREAM_DRAW);
            }
            gl.use_program(Some(&line_prog.program));
            gl.uniform2f(Some(&line_prog.u_camera_px), player_x_px, player_y_px);
            gl.uniform2f(
                Some(&line_prog.u_canvas_px),
                canvas_w as f32,
                canvas_h as f32,
            );
            gl.uniform1f(Some(&line_prog.u_zoom), zoom);
            gl.draw_arrays(Gl::LINES, 0, line_vertex_count as i32);
        }

        // Sacred-error compliance: surface GL errors through the
        // event log instead of leaving them silent in `gl.getError()`.
        let err = gl.get_error();
        if err != Gl::NO_ERROR {
            emit_error(
                Severity::Error,
                "roam::render_gl::render_frame",
                format!("gl.getError = 0x{err:x} after draw"),
                format!(
                    "tile_count={tile_count} flower_count={flower_count} marker_count={marker_count} canvas={canvas_w}x{canvas_h}"
                ),
            );
        }

        gl.bind_vertex_array(None);
        Ok(())
    })
}

#[inline]
unsafe fn read_u32_le(ptr: *const u8, offset: usize) -> u32 {
    let p = ptr.add(offset) as *const u32;
    p.read_unaligned().to_le()
}

#[inline]
unsafe fn read_i32_le(ptr: *const u8, offset: usize) -> i32 {
    let p = ptr.add(offset) as *const i32;
    p.read_unaligned().to_le()
}
