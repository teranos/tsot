//! Runtime Report — pinned diagnostic block visible at the top of the
//! drawer, populated once at Startup.
//!
//! The evidence packet. Every "won't render" screenshot has to arrive
//! with: which wgpu backend won, which adapter, which limits/features,
//! which downlevel flags the browser gave us, plus the build hash.
//! Without this, RCA on render regressions turns into
//! deploy-and-guess. With it, the screenshot itself is the diagnosis.
//!
//! Reads render resources that Bevy inserts into the main World during
//! `RenderPlugin::finish()` (bevy_render 0.19 `settings.rs:195-198`).
//! By the time Startup fires, those resources are already there — or
//! they aren't coming at all, which is itself diagnostic and the
//! report says so.

use bevy::prelude::*;
use bevy::render::renderer::{RenderAdapter, RenderAdapterInfo, RenderDevice};

use crate::build_info;

#[derive(Resource, Default, Debug, Clone)]
pub struct RuntimeReport {
    pub lines: Vec<String>,
    pub ready: bool,
}

pub fn capture_runtime_report(
    mut report: ResMut<RuntimeReport>,
    adapter_info: Option<Res<RenderAdapterInfo>>,
    adapter: Option<Res<RenderAdapter>>,
    device: Option<Res<RenderDevice>>,
    windows: Query<&Window>,
) {
    if report.ready {
        return;
    }
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!(
        "BUILD:    {} @ {}",
        build_info::COMMIT,
        build_info::BUILT_AT,
    ));

    match adapter_info.as_deref() {
        Some(info) => {
            let info = &**info;
            lines.push(format!("BACKEND:  {:?}", info.backend));
            lines.push(format!(
                "ADAPTER:  {} · driver={} · type={:?} · vendor=0x{:x} · device=0x{:x}",
                info.name, info.driver, info.device_type, info.vendor, info.device,
            ));
        }
        None => {
            lines.push(
                "BACKEND:  (RenderAdapterInfo missing — renderer init failed or headless)"
                    .to_string(),
            );
        }
    }

    if let Some(dev) = device.as_deref() {
        let limits = dev.limits();
        let features = dev.features();
        lines.push(format!(
            "LIMITS:   tex2D={} tex_layers={} bind_groups={} storage_buf/stage={} uniform_buf_max={}",
            limits.max_texture_dimension_2d,
            limits.max_texture_array_layers,
            limits.max_bind_groups,
            limits.max_storage_buffers_per_shader_stage,
            limits.max_uniform_buffer_binding_size,
        ));
        lines.push(format!("FEATURES: {features:?}"));
    }

    if let Some(a) = adapter.as_deref() {
        let cap = a.get_downlevel_capabilities();
        lines.push(format!(
            "DOWNLEVEL: flags={:?} shader_model={:?}",
            cap.flags, cap.shader_model,
        ));
    }

    if let Ok(w) = windows.single() {
        lines.push(format!(
            "CANVAS:   {}x{} (scale_factor={:.2})",
            w.physical_width(),
            w.physical_height(),
            w.scale_factor(),
        ));
    }

    report.lines = lines;
    report.ready = true;
}
