#![cfg(target_arch = "wasm32")]

use bevy::asset::Assets;
use bevy::camera::Hdr;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::system::SystemParam;
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::renderer::{RenderAdapterInfo, RenderDevice};
use bevy_observability::ErrorLog;

#[derive(SystemParam)]
pub struct EntityBreakdown<'w, 's> {
    text: Query<'w, 's, Entity, With<bevy::text::TextSpan>>,
    node: Query<'w, 's, Entity, With<bevy::ui::Node>>,
    mesh: Query<'w, 's, Entity, With<Mesh3d>>,
    light: Query<'w, 's, Entity, With<PointLight>>,
}

#[derive(Default)]
pub struct ReportState {
    ticks: u64,
    last_secs: f32,
    last_error_len: usize,
    oom_dumped: bool,
}

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
    mesh_users: Query<&Mesh3d>,
) {
    crate::js_rave_error("[mem-probe] report system entered");
    state.ticks += 1;
    let now = time.elapsed_secs();
    let first = state.ticks == 1;
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

    let rust_used = crate::rave_rust_alloc_bytes();
    let rust_peak = crate::rave_rust_peak_alloc_bytes();
    let rust_count = crate::rave_rust_alloc_count();
    crate::js_rave_error(&format!(
        "[mem@{t}s] rust-alloc: used={:.2}MB peak={:.2}MB count={rust_count}",
        rust_used as f64 / 1_048_576.0,
        rust_peak as f64 / 1_048_576.0,
    ));

    let info = &**adapter_info;
    crate::js_rave_error(&format!(
        "[mem@{t}s] adapter: name={:?} type={:?} backend={:?} vendor=0x{:x} device=0x{:x}",
        info.name, info.device_type, info.backend, info.vendor, info.device,
    ));

    let limits = device.limits();
    crate::js_rave_error(&format!(
        "[mem@{t}s] limits: max_buffer_size={} MB · max_texture_2d={} · max_bind_groups={} · max_uniform_buffer_binding_size={} KB",
        limits.max_buffer_size / 1_048_576,
        limits.max_texture_dimension_2d,
        limits.max_bind_groups,
        limits.max_uniform_buffer_binding_size / 1024,
    ));

    if let Some(pc) = &pipeline_cache {
        let cached = pc.pipelines().count();
        let waiting = pc.waiting_pipelines().count();
        crate::js_rave_error(&format!(
            "[mem@{t}s] pipelines: cached={cached} waiting={waiting}",
        ));
    }

    let entity_count = entities.iter().count();
    let mesh_count = meshes.iter().count();
    let material_count = materials.iter().count();
    let image_count = images.iter().count();

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

    use std::collections::HashMap;
    let mut mesh_use_count: HashMap<bevy::asset::AssetId<Mesh>, usize> = HashMap::new();
    for Mesh3d(handle) in mesh_users.iter() {
        *mesh_use_count.entry(handle.id()).or_insert(0) += 1;
    }
    let mut mesh_vertex_bytes: usize = 0;
    let mut mesh_index_bytes: usize = 0;
    let mut per_mesh_lines: Vec<(usize, String)> = Vec::new();
    for (id, mesh) in meshes.iter() {
        let vbytes = mesh.get_vertex_size() as usize * mesh.count_vertices();
        let ibytes = mesh.indices().map(|i| i.len() * 4).unwrap_or(0);
        mesh_vertex_bytes += vbytes;
        mesh_index_bytes += ibytes;
        let users = mesh_use_count.get(&id).copied().unwrap_or(0);
        let total_kb = (vbytes + ibytes) as f64 / 1024.0;
        per_mesh_lines.push((
            vbytes + ibytes,
            format!(
                "vcount={} users={} bytes={:.2}KB",
                mesh.count_vertices(),
                users,
                total_kb,
            ),
        ));
    }
    per_mesh_lines.sort_by(|a, b| b.0.cmp(&a.0));

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
        rt_depth += px * 4 * samples;
        camera_lines.push(format!(
            "{}x{} msaa={} hdr={} → color {:.1} MB + depth {:.1} MB",
            size.x, size.y, samples,
            if is_hdr { "on" } else { "off" },
            (px * bpp_actual * samples) as f64 / 1_048_576.0,
            (px * 4 * samples) as f64 / 1_048_576.0,
        ));
    }

    let accountable = image_bytes
        + mesh_vertex_bytes
        + mesh_index_bytes
        + rt_color_actual
        + rt_depth;

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(-1.0);

    let text_ents = breakdown.text.iter().count();
    let node_ents = breakdown.node.iter().count();
    let mesh_ents = breakdown.mesh.iter().count();
    let light_ents = breakdown.light.iter().count();
    let other_ents = entity_count
        .saturating_sub(text_ents)
        .saturating_sub(node_ents)
        .saturating_sub(mesh_ents)
        .saturating_sub(light_ents);

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
    for (_, line) in per_mesh_lines.iter().take(10) {
        crate::js_rave_error(&format!("[mem@{t}s]     mesh {line}"));
    }
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
        "[mem@{t}s]   ACCOUNTABLE     = {:.2} MB",
        accountable as f64 / 1_048_576.0,
    ));
}
