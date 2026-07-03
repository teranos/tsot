//! Memory footprint diagnostic — repeatable, fact-driven.
//!
//! Memory accountability is the #1 priority: OOM on iPad Pro Sim proves
//! we're spending memory we don't understand. This report is the peek-
//! inside primitive — every load, every 15s, every category we can
//! measure, emitted to the sessionStorage-persistent HTML overlay.
//!
//! Measured buckets (with source):
//!   - Entity count via `Query<Entity>`.
//!   - Asset counts via `Assets<T>::iter().count()`.
//!   - Image bytes (CPU-side pixel data) via `image.data.len()`.
//!   - Mesh vertex+index bytes via `mesh.get_vertex_size() *
//!     count_vertices() + indices().len() * 4`.
//!   - Camera render target bytes = `px * bpp * msaa_samples`.
//!   - Depth buffer bytes (estimated same-res × 4 for Depth32Float).
//!   - wgpu adapter info: name, device_type, backend, vendor. Shows
//!     immediately if we're on software Metal (device_type=Cpu).
//!   - wgpu device limits: max_buffer_size, max_texture_dim_2d,
//!     max_bind_groups, max_uniform_buffer_binding_size. Ceilings that
//!     wgpu enforces — hitting one throws OOM.
//!   - FPS via `FrameTimeDiagnosticsPlugin`.
//!   - Sum + percentage-of-accountable.
//!
//! Not yet covered:
//!   - `performance.memory` — Chrome/Firefox only, not Safari.
//!   - Text glyph atlas — bevy_text internals not directly queryable.
//!   - wgpu pipeline cache byte total — no wgpu API for it.
//!   - Prepass render targets — only allocated if prepass explicitly
//!     enabled on the camera; rave doesn't use it.

#![cfg(target_arch = "wasm32")]

use bevy::asset::Assets;
use bevy::camera::Hdr;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};

/// Sample cadence — after the first tick, fire every `INTERVAL_SECS`.
/// The first-tick fire captures state BEFORE any OOM crashes the
/// render loop, so we see what's loaded even when Bevy dies at ~9s
/// on constrained iPad Pro Simulator.
const INTERVAL_SECS: f32 = 15.0;

pub fn report(
    time: Res<Time>,
    mut ticks: Local<u64>,
    mut last_secs: Local<f32>,
    diagnostics: Res<DiagnosticsStore>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    images: Res<Assets<Image>>,
    entities: Query<Entity>,
    cameras: Query<(&Camera, Option<&Hdr>, Option<&Msaa>)>,
    adapter_info: Res<RenderAdapterInfo>,
    device: Res<RenderDevice>,
) {
    *ticks += 1;
    let now = time.elapsed_secs();
    let first = *ticks == 1;
    if !first && now - *last_secs < INTERVAL_SECS {
        return;
    }
    *last_secs = now;
    let t = now as u32;

    // ------- adapter identity: name + device_type + backend + vendor.
    // device_type=Cpu means software renderer (~256-512 MB VRAM ceiling
    // typical). device_type=IntegratedGpu / DiscreteGpu = real hardware.
    let info = &**adapter_info;
    crate::js_rave_error(&format!(
        "[mem@{t}s] adapter: name={:?} type={:?} backend={:?} vendor=0x{:x} device=0x{:x}",
        info.name, info.device_type, info.backend, info.vendor, info.device,
    ));

    // ------- device limits. These are the ceilings wgpu enforces. A
    // single allocation exceeding one of these → OOM. Print the ones
    // that matter for the OOM RCA.
    let limits = device.limits();
    crate::js_rave_error(&format!(
        "[mem@{t}s] limits: max_buffer_size={} MB · max_texture_2d={} · max_bind_groups={} · max_uniform_buffer_binding_size={} KB",
        limits.max_buffer_size / 1_048_576,
        limits.max_texture_dimension_2d,
        limits.max_bind_groups,
        limits.max_uniform_buffer_binding_size / 1024,
    ));

    // ------- entities + asset counts.
    let entity_count = entities.iter().count();
    let mesh_count = meshes.iter().count();
    let material_count = materials.iter().count();
    let image_count = images.iter().count();

    // ------- image CPU bytes (real pixel data on Rust side).
    let mut image_bytes: usize = 0;
    for (_, img) in images.iter() {
        if let Some(data) = &img.data {
            image_bytes += data.len();
        }
    }

    // ------- mesh vertex + index bytes. GPU upload matches this
    // exactly at first (no mipmap-style alignment inflation on VBOs).
    let mut mesh_vertex_bytes: usize = 0;
    let mut mesh_index_bytes: usize = 0;
    for (_, mesh) in meshes.iter() {
        mesh_vertex_bytes += mesh.get_vertex_size() as usize * mesh.count_vertices();
        if let Some(indices) = mesh.indices() {
            mesh_index_bytes += indices.len() * 4;
        }
    }

    // ------- per-camera render target + depth buffer bytes.
    let mut camera_lines: Vec<String> = Vec::new();
    let mut rt_color_actual: usize = 0;
    let mut rt_color_hdr_on: usize = 0;
    let mut rt_color_hdr_off: usize = 0;
    let mut rt_depth: usize = 0;
    for (cam, hdr, msaa) in cameras.iter() {
        let size = cam.physical_target_size().unwrap_or_default();
        let px = size.x as usize * size.y as usize;
        let samples: usize = match msaa {
            Some(Msaa::Off) => 1,
            Some(Msaa::Sample2) => 2,
            Some(Msaa::Sample4) => 4,
            Some(Msaa::Sample8) => 8,
            None => 4,
        };
        let is_hdr = hdr.is_some();
        let bpp_actual = if is_hdr { 8 } else { 4 };
        rt_color_actual += px * bpp_actual * samples;
        rt_color_hdr_on += px * 8 * samples;
        rt_color_hdr_off += px * 4 * samples;
        // Depth: Bevy's default Camera3d uses `Depth32Float` = 4 bpp.
        // MSAA-resolved depth is 1 sample; MSAA source is samples.
        rt_depth += px * 4 * samples;
        camera_lines.push(format!(
            "{}x{} msaa={} hdr={} → color {:.1} MB + depth {:.1} MB",
            size.x, size.y, samples,
            if is_hdr { "on" } else { "off" },
            (px * bpp_actual * samples) as f64 / 1_048_576.0,
            (px * 4 * samples) as f64 / 1_048_576.0,
        ));
    }

    // ------- accountable total.
    let accountable = image_bytes
        + mesh_vertex_bytes
        + mesh_index_bytes
        + rt_color_actual
        + rt_depth;

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(-1.0);

    // ------- report.
    crate::js_rave_error(&format!(
        "[mem@{t}s] entities={entity_count} meshes={mesh_count} materials={material_count} images={image_count} fps={:.1}",
        fps,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   images CPU  = {:.2} MB",
        image_bytes as f64 / 1_048_576.0,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   mesh VBOs   = {:.2} MB (vertex {:.2} + index {:.2})",
        (mesh_vertex_bytes + mesh_index_bytes) as f64 / 1_048_576.0,
        mesh_vertex_bytes as f64 / 1_048_576.0,
        mesh_index_bytes as f64 / 1_048_576.0,
    ));
    for line in &camera_lines {
        crate::js_rave_error(&format!("[mem@{t}s]   camera {line}"));
    }
    crate::js_rave_error(&format!(
        "[mem@{t}s]   RT color total  = {:.2} MB (hdr-on would be {:.2} MB, delta {:+.2})",
        rt_color_actual as f64 / 1_048_576.0,
        rt_color_hdr_on as f64 / 1_048_576.0,
        (rt_color_hdr_on as f64 - rt_color_hdr_off as f64) / 1_048_576.0,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   RT depth total  = {:.2} MB",
        rt_depth as f64 / 1_048_576.0,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   ACCOUNTABLE     = {:.2} MB (images + mesh VBOs + RT color + RT depth)",
        accountable as f64 / 1_048_576.0,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   NOT ACCOUNTABLE (invisible): wgpu pipeline cache, text glyph atlas, per-frame uniform/dynamic buffers, wgpu-internal driver allocations."
    ));
}
