//! Memory footprint diagnostic — repeatable, fact-driven.
//!
//! Fires first at t=15s, then every 15s. Emits to `js_rave_error` so
//! every line lands in the sessionStorage-persistent HTML overlay
//! (survives reload cycles) and via the ErrorLog mirror also in the
//! Bevy in-canvas drawer.
//!
//! Computes what wasm+Bevy expose:
//!   - Entity count via `Query<Entity>` (no plugin needed).
//!   - Asset counts via `Assets<T>::iter().count()`.
//!   - Image byte total via `image.data.as_ref().unwrap().len()`
//!     (CPU-side pixel data; GPU allocation is >= this, often more
//!     due to alignment + mipmaps).
//!   - Camera render target byte cost, computed exactly from
//!     `physical_target_size * bytes_per_pixel * msaa_samples` where
//!     `bytes_per_pixel = 8 (Rgba16Float, Hdr) or 4 (Rgba8UnormSrgb,
//!     non-Hdr)`. Both Hdr on/off values printed regardless of current
//!     Hdr state — makes the toggle cost visible without a rebuild.
//!   - FPS via `FrameTimeDiagnosticsPlugin` (already registered).
//!
//! What we don't cover (limitations of the wasm target):
//!   - `performance.memory` — Chrome/Firefox only, not Safari.
//!   - Per-resource GPU byte allocation — wgpu doesn't expose it.
//!   - Total process RAM — no wasm equivalent to `getrusage`.

#![cfg(target_arch = "wasm32")]

use bevy::asset::Assets;
use bevy::camera::Hdr;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::image::Image;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;

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
) {
    *ticks += 1;
    let now = time.elapsed_secs();
    // Force fire on the very first Update — captures pre-OOM state.
    // After that, throttle to every INTERVAL_SECS.
    let first = *ticks == 1;
    if !first && now - *last_secs < INTERVAL_SECS {
        return;
    }
    *last_secs = now;

    let entity_count = entities.iter().count();
    let mesh_count = meshes.iter().count();
    let material_count = materials.iter().count();
    let image_count = images.iter().count();

    let mut image_bytes: usize = 0;
    for (_, img) in images.iter() {
        if let Some(data) = &img.data {
            image_bytes += data.len();
        }
    }

    let mut camera_lines: Vec<String> = Vec::new();
    let mut total_actual: usize = 0;
    let mut total_if_hdr_on: usize = 0;
    let mut total_if_hdr_off: usize = 0;
    for (cam, hdr, msaa) in cameras.iter() {
        let size = cam.physical_target_size().unwrap_or_default();
        let px = size.x as usize * size.y as usize;
        // Bevy default for Msaa (component absent) is Sample4. We put
        // `Msaa::Off` on the rave camera explicitly, so an `Option<&Msaa>`
        // that's `None` on some other camera means default 4.
        let samples: usize = match msaa {
            Some(Msaa::Off) => 1,
            Some(Msaa::Sample2) => 2,
            Some(Msaa::Sample4) => 4,
            Some(Msaa::Sample8) => 8,
            None => 4,
        };
        let is_hdr = hdr.is_some();
        let bpp_actual = if is_hdr { 8 } else { 4 };
        let bytes_actual = px * bpp_actual * samples;
        let bytes_on = px * 8 * samples;
        let bytes_off = px * 4 * samples;
        total_actual += bytes_actual;
        total_if_hdr_on += bytes_on;
        total_if_hdr_off += bytes_off;
        camera_lines.push(format!(
            "{}x{} msaa={} hdr={} → {:.1} MB",
            size.x,
            size.y,
            samples,
            if is_hdr { "on" } else { "off" },
            bytes_actual as f64 / 1_048_576.0,
        ));
    }

    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(-1.0);

    let t = now as u32;

    crate::js_rave_error(&format!(
        "[mem@{t}s] entities={entity_count} meshes={mesh_count} materials={material_count} images={image_count} ({:.1} MB) fps={:.1}",
        image_bytes as f64 / 1_048_576.0,
        fps,
    ));
    for line in &camera_lines {
        crate::js_rave_error(&format!("[mem@{t}s]   camera {line}"));
    }
    crate::js_rave_error(&format!(
        "[mem@{t}s]   render-target totals: actual {:.1} MB · hdr-on would be {:.1} MB · hdr-off would be {:.1} MB (delta {:+.1} MB)",
        total_actual as f64 / 1_048_576.0,
        total_if_hdr_on as f64 / 1_048_576.0,
        total_if_hdr_off as f64 / 1_048_576.0,
        (total_if_hdr_on as f64 - total_if_hdr_off as f64) / 1_048_576.0,
    ));
}
