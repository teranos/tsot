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
use bevy::ecs::system::SystemParam;
use bevy::render::render_resource::PipelineCache;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};
use bevy_observability::ErrorLog;

// Bundled entity-breakdown queries so the outer system stays under
// Bevy's 16-param limit for system functions.
#[derive(SystemParam)]
pub struct EntityBreakdown<'w, 's> {
    text: Query<'w, 's, Entity, With<bevy::text::TextSpan>>,
    node: Query<'w, 's, Entity, With<bevy::ui::Node>>,
    mesh: Query<'w, 's, Entity, With<Mesh3d>>,
    light: Query<'w, 's, Entity, With<PointLight>>,
}

// Bundled Local state so we spend one system-param slot instead of
// four for ticks/last_secs/last_error_len/oom_dumped.
#[derive(Default)]
pub struct ReportState {
    ticks: u64,
    last_secs: f32,
    last_error_len: usize,
    oom_dumped: bool,
}

/// Sample cadence — after the first tick, fire every `INTERVAL_SECS`.
/// The first-tick fire captures state BEFORE any OOM crashes the
/// render loop, so we see what's loaded even when Bevy dies at ~9s
/// on constrained iPad Pro Simulator.
const INTERVAL_SECS: f32 = 15.0;

pub fn report(
    time: Res<Time>,
    mut state: Local<ReportState>,
    diagnostics: Res<DiagnosticsStore>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
    images: Res<Assets<Image>>,
    entities: Query<Entity>,
    cameras: Query<(&Camera, Option<&Hdr>, Option<&Msaa>)>,
    adapter_info: Res<RenderAdapterInfo>,
    device: Res<RenderDevice>,
    pipeline_cache: Option<Res<PipelineCache>>,
    error_log: Res<ErrorLog>,
    breakdown: EntityBreakdown,
) {
    state.ticks += 1;
    let now = time.elapsed_secs();
    let first = state.ticks == 1;
    // OOM trigger: any new ErrorLog entry mentioning "Out of Memory"
    // or "DeviceLost" fires the report immediately, regardless of
    // the 15s throttle. Once we've dumped on OOM once, don't re-dump
    // — the state after DeviceLost is invalid anyway.
    let current_error_len = error_log.0.len();
    let mut oom_now = false;
    if !state.oom_dumped && current_error_len > state.last_error_len {
        for entry in &error_log.0[state.last_error_len..] {
            let m = &entry.message;
            if m.contains("Out of Memory")
                || m.contains("Out of memory")
                || m.contains("DeviceLost")
            {
                oom_now = true;
                state.oom_dumped = true;
                break;
            }
        }
    }
    state.last_error_len = current_error_len;
    if !first && !oom_now && now - state.last_secs < INTERVAL_SECS {
        return;
    }
    state.last_secs = now;
    let t = now as u32;
    if oom_now {
        crate::js_rave_error(&format!(
            "[mem@{t}s] === OOM TRIGGERED MEMORY DUMP ==="
        ));
    }

    // ------- adapter identity: name + device_type + backend + vendor.
    let info = &**adapter_info;
    crate::js_rave_error(&format!(
        "[mem@{t}s] adapter: name={:?} type={:?} backend={:?} vendor=0x{:x} device=0x{:x}",
        info.name, info.device_type, info.backend, info.vendor, info.device,
    ));

    // wgpu Instance::generate_report call reverted — Res<RenderInstance>
    // doesn't have `generate_report()` reachable that way in Bevy 0.19.
    // Correct API requires reading Bevy source first.

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

    // ------- pipeline cache count. Each pipeline is a compiled shader
    // program held in wgpu. Bytes-per-pipeline aren't exposed but the
    // count is, and it's the closest proxy for pipeline-cache footprint.
    if let Some(pc) = &pipeline_cache {
        let cached = pc.pipelines().count();
        let waiting = pc.waiting_pipelines().count();
        crate::js_rave_error(&format!(
            "[mem@{t}s] pipelines: cached={cached} waiting={waiting}",
        ));
    }

    // wgpu Instance::generate_report is not available on wasm — it
    // requires the wgpu_core feature which Bevy doesn't enable in the
    // wasm target. Dead end for per-backend handle counts through that
    // API. Alternative introspection would be a wgpu-hal fork with
    // byte accounting.

    // ------- entities + asset counts.
    let entity_count = entities.iter().count();
    let mesh_count = meshes.iter().count();
    let material_count = materials.iter().count();
    let image_count = images.iter().count();

    // ------- image CPU bytes + per-image size list. If one image is
    // giant (e.g. font atlas), it stands out.
    let mut image_bytes: usize = 0;
    let mut per_image: Vec<usize> = Vec::new();
    for (_, img) in images.iter() {
        let n = img.data.as_ref().map(|d| d.len()).unwrap_or(0);
        image_bytes += n;
        per_image.push(n);
    }
    per_image.sort_by(|a, b| b.cmp(a));
    let biggest_images: String = per_image
        .iter()
        .take(5)
        .map(|b| format!("{:.2}MB", *b as f64 / 1_048_576.0))
        .collect::<Vec<_>>()
        .join(", ");

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

    // ------- entity breakdown.
    let text_ents = breakdown.text.iter().count();
    let node_ents = breakdown.node.iter().count();
    let mesh_ents = breakdown.mesh.iter().count();
    let light_ents = breakdown.light.iter().count();
    let other_ents = entity_count
        .saturating_sub(text_ents)
        .saturating_sub(node_ents)
        .saturating_sub(mesh_ents)
        .saturating_sub(light_ents);

    // ------- report.
    crate::js_rave_error(&format!(
        "[mem@{t}s] entities={entity_count} (text={text_ents} nodes={node_ents} mesh3d={mesh_ents} pointlights={light_ents} other={other_ents}) meshes={mesh_count} materials={material_count} images={image_count} fps={:.1}",
        fps,
    ));
    crate::js_rave_error(&format!(
        "[mem@{t}s]   images CPU  = {:.2} MB (top 5 by size: [{}])",
        image_bytes as f64 / 1_048_576.0,
        biggest_images,
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
