mod audio;
mod build_info;
mod chat;
mod drawer;
mod error;
mod floorplan;
mod identity;
mod net;
mod observability;
mod physics;
#[cfg(target_arch = "wasm32")]
mod remote_players;
mod room;
mod trail;
mod trees;

use bevy::asset::AssetMetaCheck;
use bevy::camera::Hdr;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::log::LogPlugin;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::settings::{Backends, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::window::WindowPlugin;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use bevy_observability::{ErrorLog, PANIC_QUEUE};

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
unsafe extern "C" {
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

pub const RELAY_MULTIADDR: &str =
    "/dns4/relaye.sbvh.nl/tcp/443/wss/p2p/12D3KooWC6UBnnmhhv3BAfYKyW1bFBD4GtC5waiEgQWJCb7Hbqaf";

pub const POSITIONS_TOPIC: &str = "rave-positions/v1";

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        js_rave_error(&format!("[pre-Bevy panic] {info}"));
    }));

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(async {
        let identity_bytes = load_or_mint_identity().await;
        build_and_run_app(Some(identity_bytes));
    });

    #[cfg(not(target_arch = "wasm32"))]
    build_and_run_app(None);
}

#[cfg(target_arch = "wasm32")]
async fn load_or_mint_identity() -> Vec<u8> {
    use wasm_bindgen::JsCast;

    let load_promise = identity::js_rave_load_identity();
    match wasm_bindgen_futures::JsFuture::from(load_promise).await {
        Ok(val) if !val.is_null() && !val.is_undefined() => {
            match val.dyn_into::<js_sys::Uint8Array>() {
                Ok(arr) => {
                    let mut bytes = vec![0u8; arr.length() as usize];
                    arr.copy_to(&mut bytes);
                    bytes
                }
                Err(_) => {
                    error::emit_region(
                        error::Severity::Error,
                        "identity-load",
                        "non-Uint8Array from JS bridge",
                        "expected Uint8Array (or null), got something else",
                    );
                    mint_and_save_identity().await
                }
            }
        }
        Ok(_) => mint_and_save_identity().await,
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "identity-load",
                "IndexedDB load rejected",
                format!("{e:?}"),
            );
            mint_and_save_identity().await
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn mint_and_save_identity() -> Vec<u8> {
    let fresh = bevy_libp2p::Keypair::generate_ed25519();
    let bytes = match fresh.to_protobuf_encoding() {
        Ok(b) => b.to_vec(),
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "identity-generate",
                "Ed25519 keypair encode failed",
                format!("{e}"),
            );
            return Vec::new();
        }
    };
    let arr = js_sys::Uint8Array::from(bytes.as_slice());
    let save_promise = identity::js_rave_save_identity(arr);
    if let Err(e) = wasm_bindgen_futures::JsFuture::from(save_promise).await {
        error::emit_region(
            error::Severity::Warn,
            "identity-save",
            "IndexedDB save rejected",
            format!("{e:?}"),
        );
    }
    bytes
}

fn build_and_run_app(_identity_bytes: Option<Vec<u8>>) {
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
                    custom_layer: bevy_observability::install_capture_layer,
                    ..default()
                })
                // Bevy 0.19's default WgpuSettings picks BROWSER_WEBGPU only
                // when the `webgpu` feature is on (bevy_render/settings.rs:78),
                // regardless of `webgl2` also being on. Explicitly union both
                // so wgpu falls through to WebGL2 on browsers without
                // `navigator.gpu`.
                .set(RenderPlugin {
                    render_creation: WgpuSettings {
                        backends: Some(Backends::BROWSER_WEBGPU | Backends::GL),
                        ..default()
                    }
                    .into(),
                    ..default()
                }),
        );

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
                trees::setup_trees,
                trail::setup_trail,
                drawer::setup_drawer,
            ),
        )
        .add_systems(PostStartup, audio::setup_audio)
        .add_systems(
            Update,
            (
                bevy_observability::drain_panics,
                bevy_observability::drain_logs,
                drawer::update_fps,
                drawer::update_error_list,
                drawer::toggle_log_drawer,
                screenshot_on_p,
                room::move_player,
                physics::resolve_collisions,
                room::camera_follow,
                floorplan::pulse_strobes,
                floorplan::pulse_truss_lights,
            ),
        );

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, drawer::update_clock);

    #[cfg(target_arch = "wasm32")]
    {
        app.add_plugins(bevy_libp2p::LibP2PPlugin {
            bootstrap_addrs: vec![RELAY_MULTIADDR.to_string()],
            identity_bytes: _identity_bytes,
            topics: vec![
                bevy_libp2p::Topic(POSITIONS_TOPIC.to_string()),
                bevy_libp2p::Topic(chat::CHAT_TOPIC.to_string()),
            ],
            identify_protocol: "/rave/1.0.0".to_string(),
        });
        app.insert_resource(remote_players::RemotePlayers::default());
        app.add_systems(
            Update,
            (
                observability::flush_typed_errors,
                remote_players::drain_net_events,
                remote_players::publish_self_position,
                chat::publish_pending_chat,
                remote_players::render_remote_players,
                drawer::update_net_stats,
            )
                .chain(),
        );
    }

    app.run();
}

fn setup_scene_lights(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Bloom::default(),
        Transform::from_xyz(0.0, 80.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        PointLight {
            shadow_maps_enabled: false,
            intensity: 600_000.0,
            range: 6000.0,
            color: Color::srgb(0.35, 0.40, 0.55),
            ..default()
        },
        Transform::from_xyz(0.0, 1500.0, 0.0),
    ));
}

fn screenshot_on_p(keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyP) {
        js_rave_screenshot("");
    }
}
