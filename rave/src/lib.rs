//! rave — Bevy + libp2p rave party orchestrator.
//!
//! This file owns ONLY the App scaffold + the JS bridges. Every
//! concern lives in its own module:
//!
//!   - `room`         — floor plane, player, WASD/touch, camera
//!   - `floorplan`    — DJ/speakers/bar/toilets/garderobe/walls + strobes
//!   - `drawer`       — in-canvas diagnostic UI (FPS, errors, net stats)
//!   - `observability`— panic hook + tracing layer + typed-error pipeline
//!   - `net_glue`     — Bevy ↔ libp2p (boot, publish, render peers)
//!   - `net`          — libp2p Swarm wiring + wire types (browser-only)
//!   - `error`        — typed sacred-error helpers
//!   - `identity`     — Ed25519 keypair load/generate + IndexedDB bridge
//!   - `build_info`   — compile-time commit + timestamp

mod build_info;
mod drawer;
mod error;
mod floorplan;
mod identity;
mod net;
mod net_glue;
mod observability;
mod room;

use bevy::asset::AssetMetaCheck;
use bevy::core_pipeline::bloom::Bloom;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::window::WindowPlugin;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use observability::{ErrorLog, PANIC_QUEUE};

// Out-of-Bevy error path. Calls window.__raveError defined in
// rave/web/. Surfaces panics + ERROR-level tracing to the HTML
// overlay even when Bevy itself is dead. Without this, anything that
// panics before the first Update tick is silently swallowed: the
// in-canvas drawer never renders because the systems that update it
// never run.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = "__raveError")]
    pub(crate) fn js_rave_error(msg: &str);

    #[wasm_bindgen(js_namespace = window, js_name = "__raveErrorTyped")]
    pub(crate) fn js_rave_error_typed(json: &str);

    #[wasm_bindgen(js_namespace = window, js_name = "__raveScreenshot")]
    pub(crate) fn js_rave_screenshot(filename: &str);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn js_rave_error(_msg: &str) {}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn js_rave_error_typed(_json: &str) {}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn js_rave_screenshot(_filename: &str) {}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    // Pre-App panic hook — catches panics during App::new() and the
    // first slice of plugin building, before LogPlugin installs its
    // own hook and overwrites ours.
    std::panic::set_hook(Box::new(|info| {
        js_rave_error(&format!("[pre-Bevy panic] {info}"));
    }));

    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.01, 0.05, 0.12)))
        .insert_resource(ErrorLog::default())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "rave".to_string(),
                        canvas: Some("#bevy".to_owned()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(LogPlugin {
                    custom_layer: observability::install_capture_layer,
                    ..default()
                }),
        );

    // Wrap LogPlugin's panic hook NOW (after LogPlugin built, before
    // app.run()). Anything panicking from this point on — Startup
    // systems that can't reach the in-canvas drawer because Update
    // hasn't run yet, Update systems themselves — still surfaces to
    // the HTML overlay via __raveError.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let formatted = format!("{info}");
        js_rave_error(&format!("[panic] {formatted}"));
        let mut q = PANIC_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push(formatted);
        prev(info);
    }));

    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_systems(
            Startup,
            (
                setup_scene_lights,
                room::setup_room,
                floorplan::setup_floor_plan,
                drawer::setup_drawer,
            ),
        )
        .add_systems(
            Update,
            (
                observability::drain_panics,
                observability::drain_logs,
                drawer::update_fps,
                drawer::update_error_list,
                drawer::toggle_log_drawer,
                screenshot_on_p,
                room::move_player,
                room::camera_follow,
                floorplan::pulse_strobes,
                floorplan::pulse_truss_lights,
            ),
        );

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, drawer::update_clock);

    #[cfg(target_arch = "wasm32")]
    {
        // Network resource lives as NonSend because Net contains Rc +
        // RefCell (the swarm task uses Rc clones for the shared event
        // queue). Inserted as None; the async boot fills it.
        app.insert_non_send_resource::<Option<net::Net>>(None);
        app.insert_resource(net_glue::RemotePlayers::default());
        app.add_systems(
            Update,
            (
                net_glue::install_pending_net,
                observability::flush_typed_errors,
                net_glue::drain_net_events,
                net_glue::publish_self_position,
                net_glue::render_remote_players,
                drawer::update_net_stats,
            )
                .chain(),
        );

        // Kick off the async boot. Awaits the JS identity bridge,
        // constructs Net, subscribes to the positions topic, stashes
        // it in PENDING_NET for the Update system to pick up next
        // frame.
        wasm_bindgen_futures::spawn_local(net_glue::boot_net());
    }

    app.run();
}

/// Camera + minimal ambient. Dim on purpose — the floorplan module
/// owns the truss spotlights + strobes that actually light the room,
/// and they only read if the base level is low.
fn setup_scene_lights(mut commands: Commands) {
    // Bloom + AcesFitted tonemapping make bright emissive fixtures + the
    // colored truss spots actually read as nightclub lights instead of
    // washing out to white. Without bloom, even high-intensity
    // PointLights look matte.
    commands.spawn((
        Camera3d::default(),
        Camera {
            hdr: true,
            ..default()
        },
        Tonemapping::AcesFitted,
        Bloom::default(),
        Transform::from_xyz(0.0, 80.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            shadow_maps_enabled: false,
            intensity: 400_000.0,
            range: 1200.0,
            color: Color::srgb(0.35, 0.35, 0.50),
            ..default()
        },
        Transform::from_xyz(0.0, 600.0, 0.0),
    ));
}

/// 'P' copies the canvas as a PNG to the system clipboard via the JS
/// side. Small enough to keep in lib.rs; the screenshot bridge is the
/// only one of the JS externs that ties to a key, not a system tick.
fn screenshot_on_p(keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyP) {
        js_rave_screenshot("");
    }
}
