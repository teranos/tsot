mod audio;
mod build_info;
mod campfire;
mod drawer;
mod error;
mod floorplan;
mod health;
mod identity;
mod map;
#[cfg(target_arch = "wasm32")]
mod memory_report;
mod minimap;
mod net;
mod observability;
mod runtime_report;
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
use bevy::window::WindowPlugin;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use bevy_observability::{ErrorLog, PANIC_QUEUE};

#[cfg(target_arch = "wasm32")]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(target_arch = "wasm32")]
use std::sync::Mutex;
#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(target_arch = "wasm32")]
static RUST_ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_arch = "wasm32")]
static RUST_ALLOC_PEAK: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_arch = "wasm32")]
static RUST_ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

// Per-large-allocation attribution: on every heap alloc >= 1 MB we
// capture the wasm-side JS stack (via `new Error().stack`), which
// resolves to Rust identifiers thanks to the wasm name section preserved
// by wasm-bindgen. Rust's own `std::backtrace::Backtrace::force_capture`
// returns "disabled" on wasm32-unknown-unknown without unwind support, so
// the JS-Error route is the reliable stack source from inside the wasm.
//
// Reentrancy guard: if the capture itself triggers a >= 1 MB allocation,
// the swap-guard makes the inner call skip its own capture, preventing
// unbounded recursion. Small inner allocations (JsValue wrappers, String
// growth) go through the normal aggregate-counter path with no guard.
//
// WASM_BINDGEN_READY: js_sys::Error::new is a JS boundary call. Before
// the wasm-bindgen `start` fn runs, that import table isn't wired.
// Setting the flag first thing inside `run()` gates capture until then.
#[cfg(target_arch = "wasm32")]
static IN_HEAVY_ALLOC: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "wasm32")]
static WASM_BINDGEN_READY: AtomicBool = AtomicBool::new(false);
#[cfg(target_arch = "wasm32")]
static ALLOC_HOTSPOTS: Mutex<Vec<AllocRecord>> = Mutex::new(Vec::new());
#[cfg(target_arch = "wasm32")]
const HOTSPOT_SIZE_THRESHOLD: usize = 65_536;
#[cfg(target_arch = "wasm32")]
const HOTSPOT_CAPACITY: usize = 128;

#[cfg(target_arch = "wasm32")]
struct AllocRecord {
    size: usize,
    align: usize,
    seq: usize,
    stack: String,
}

#[cfg(target_arch = "wasm32")]
struct CountingAlloc;

#[cfg(target_arch = "wasm32")]
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let n = RUST_ALLOC_BYTES.fetch_add(size, Ordering::Relaxed) + size;
        let seq = RUST_ALLOC_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        let mut peak = RUST_ALLOC_PEAK.load(Ordering::Relaxed);
        while n > peak {
            match RUST_ALLOC_PEAK.compare_exchange_weak(peak, n, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(x) => peak = x,
            }
        }
        if size >= HOTSPOT_SIZE_THRESHOLD
            && WASM_BINDGEN_READY.load(Ordering::Relaxed)
            && !IN_HEAVY_ALLOC.swap(true, Ordering::Relaxed)
        {
            let err = js_sys::Error::new("__rave-alloc-stack__");
            let stack = js_sys::Reflect::get(&err, &wasm_bindgen::JsValue::from_str("stack"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let rec = AllocRecord { size, align: layout.align(), seq, stack };
            if let Ok(mut hs) = ALLOC_HOTSPOTS.lock() {
                if hs.len() >= HOTSPOT_CAPACITY {
                    hs.remove(0);
                }
                hs.push(rec);
            }
            IN_HEAVY_ALLOC.store(false, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        RUST_ALLOC_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn rave_rust_alloc_bytes() -> usize {
    RUST_ALLOC_BYTES.load(Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn rave_rust_peak_alloc_bytes() -> usize {
    RUST_ALLOC_PEAK.load(Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn rave_rust_alloc_count() -> usize {
    RUST_ALLOC_COUNT.load(Ordering::Relaxed)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn rave_crash_with_leak_report(report: &str) {
    panic!("[LEAK] {report}");
}

#[cfg(target_arch = "wasm32")]
fn dump_alloc_hotspots() {
    let hs = match ALLOC_HOTSPOTS.lock() {
        Ok(hs) => hs,
        Err(_) => {
            js_rave_error("[alloc-hotspots] mutex poisoned");
            return;
        }
    };
    if hs.is_empty() {
        js_rave_error("[alloc-hotspots] (none captured — no allocations above 1 MB threshold since last dump)");
        return;
    }
    let mut sorted: Vec<&AllocRecord> = hs.iter().collect();
    sorted.sort_by(|a, b| b.size.cmp(&a.size));
    let top_n = sorted.len().min(20);
    js_rave_error(&format!(
        "[alloc-hotspots] captured={} live_in_ring={} — top {top_n} by size:",
        RUST_ALLOC_COUNT.load(Ordering::Relaxed),
        hs.len(),
    ));
    for r in sorted.iter().take(top_n) {
        let mb = r.size as f64 / 1_048_576.0;
        js_rave_error(&format!(
            "[alloc-hotspot] seq={} size={mb:.2}MB align={} stack:\n{}",
            r.seq, r.align, r.stack
        ));
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn rave_dump_alloc_hotspots() {
    dump_alloc_hotspots();
}

#[cfg(target_arch = "wasm32")]
fn tick_alloc_hotspots(time: Res<Time>, mut last_secs: Local<f32>) {
    let now = time.elapsed_secs();
    if now - *last_secs < 15.0 {
        return;
    }
    *last_secs = now;
    dump_alloc_hotspots();
}

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
pub const CHAT_TOPIC: &str = "rave-chat/v1";

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    WASM_BINDGEN_READY.store(true, Ordering::Relaxed);
    js_rave_error(&format!(
        "[build_info] commit={} built_at={}",
        build_info::COMMIT,
        build_info::BUILT_AT
    ));
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
    js_rave_error("[probe] build_and_run_app entered");
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.01, 0.05, 0.12)))
        .insert_resource(ErrorLog::default())
        .insert_resource(health::Health::default())
        .insert_resource(runtime_report::RuntimeReport::default())
        .insert_resource(map::PinOverlayVisible::default());
    js_rave_error("[probe] resources inserted, pre-DefaultPlugins");
    // Disable unused Bevy render plugins to shrink the wgpu pipeline
    // cache footprint. Rave uses PBR mesh rendering + UI + text; every
    // plugin below adds pipelines to the cache without the game using
    // them.
    app.add_plugins(
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
                // Default is `wgpu=error,naga=warn` — bumping wgpu to
                // `trace` so any wgpu validation error, warning, or
                // notice lands in the tracing bus → CaptureLayer →
                // LOG_QUEUE → drain_logs → ErrorLog → HTML overlay
                // mirror. Backend-agnostic muted throws often have a
                // matching wgpu tracing event that the previous
                // filter dropped.
                filter: "wgpu=trace,naga=warn,rave=info,bevy=info".into(),
                ..default()
            })
            // No plugin disables — with default_app stripped from
            // Cargo.toml, the render/pbr plugins we tried to disable
            // aren't in the group in the first place, and Bevy's
            // `.disable::<T>()` panics when T isn't present. Reintroduce
            // per-plugin after confirming presence in the reduced group.
    );
    js_rave_error("[probe] post-DefaultPlugins");

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let formatted = format!("{info}");
        js_rave_error(&format!("[panic] {formatted}"));
        let mut q = PANIC_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push(formatted);
        prev(info);
    }));

    app.add_plugins((
        bevy_input_capture::InputCapturePlugin,
        bevy_input_capture::DefaultBindingsPlugin,
        bevy_chat::ChatOverlayPlugin::default(),
    ));
    js_rave_error("[probe] post-input+chatoverlay plugins");

    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_systems(
            Startup,
            (
                setup_scene_lights,
                map::setup_map,
                room::setup_room,
                floorplan::setup_floor_plan,
                trees::setup_trees,
                trail::setup_trail,
                drawer::setup_drawer,
                runtime_report::capture_runtime_report,
                campfire::setup_campfire.after(map::setup_map),
                minimap::setup_minimap.after(map::setup_map),
                probe_startup,
            ),
        )
        .add_systems(PostStartup, (audio::setup_audio, probe_post_startup))
        .add_systems(
            Update,
            (
                bevy_observability::drain_panics,
                bevy_observability::drain_logs,
                drawer::update_fps,
                drawer::update_error_list,
                drawer::update_health_text,
                drawer::update_runtime_report_text,
                drawer::toggle_log_drawer,
                map::toggle_pin_overlay,
                map::update_pin_labels,
                campfire::flicker_fire,
                minimap::update_minimap,
                minimap::handle_minimap_toggle_button,
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
    app.add_systems(Update, probe_alive);

    #[cfg(target_arch = "wasm32")]
    app.add_systems(
        Update,
        mirror_errorlog_to_overlay
            .after(bevy_observability::drain_panics)
            .after(bevy_observability::drain_logs),
    );

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, mirror_libp2p_init_error);

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, memory_report::report);

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, tick_alloc_hotspots);

    // Also run memory_report on PostStartup — if the Update schedule
    // hangs (which happened on some iPhone Sim runs) but PostStartup
    // fires, we still get one memory dump. Complements the on-first-
    // Update fire.
    #[cfg(target_arch = "wasm32")]
    app.add_systems(PostStartup, memory_report::report);

    #[cfg(target_arch = "wasm32")]
    {
        js_rave_error("[probe] pre-LibP2PPlugin");
        app.add_plugins(bevy_libp2p::LibP2PPlugin {
            bootstrap_addrs: vec![RELAY_MULTIADDR.to_string()],
            identity_bytes: _identity_bytes,
            topics: vec![
                bevy_libp2p::Topic(POSITIONS_TOPIC.to_string()),
                bevy_libp2p::Topic(CHAT_TOPIC.to_string()),
            ],
            identify_protocol: "/rave/1.0.0".to_string(),
        });
        js_rave_error("[probe] post-LibP2PPlugin");
        app.add_plugins(bevy_chat::ChatPlugin {
            topic: CHAT_TOPIC.to_string(),
            max_body_bytes: 512,
        });
        js_rave_error("[probe] post-ChatPlugin");
        app.insert_resource(remote_players::RemotePlayers::default());
        app.add_systems(
            Update,
            (
                observability::flush_typed_errors,
                remote_players::drain_net_events,
                remote_players::publish_self_position,
                remote_players::render_remote_players,
                drawer::update_net_stats,
            )
                .chain(),
        );
    }

    js_rave_error("[probe] pre-app.run");
    app.run();
}

fn probe_startup() {
    js_rave_error("[probe] Startup phase reached");
}

fn probe_post_startup() {
    js_rave_error("[probe] PostStartup phase reached");
}

fn setup_scene_lights(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // WebGPU on mobile only supports 1 or 4 MSAA samples per the
        // spec, and some iOS Safari WebGPU adapters reject 4x MSAA
        // depending on the texture format Bevy asks for. `Msaa::Off`
        // takes MSAA off the table entirely.
        Msaa::Off,
        // Hdr temporarily off — Out of Memory surfaced via
        // wgpu=trace mirror on iPad Pro Sim (bevy_render::error_handler:
        // Caught DeviceLost error: Unknown: Out of Memory). HDR doubles
        // render-target byte width per pixel. Off-then-on bisect: if
        // OOM clears in the next CI artifact, HDR is (a) contributor.
        // If it doesn't, the OOM is elsewhere (textures, meshes,
        // pipeline count).
        // Hdr,
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

#[cfg(target_arch = "wasm32")]
fn probe_alive(mut ticks: Local<u64>) {
    *ticks += 1;
    if *ticks == 1 {
        js_rave_error("[probe] Bevy Update fired for the first time");
    } else if *ticks % 600 == 0 {
        js_rave_error(&format!("[probe] Bevy tick {}", *ticks));
    }
}

// If LibP2PPlugin::build failed, it inserts LayeNetInitError. Surface
// once to the HTML overlay so the developer sees which stage failed
// (identity vs net) and why. Otherwise silenced — the resource just
// sits in the world unread.
#[cfg(target_arch = "wasm32")]
fn mirror_libp2p_init_error(
    err: Option<Res<bevy_libp2p::LayeNetInitError>>,
    mut fired: Local<bool>,
) {
    if *fired {
        return;
    }
    if let Some(e) = err {
        js_rave_error(&format!("[libp2p_init_error] {}", *e));
        *fired = true;
    }
}

// Every ErrorLog entry mirrors to the HTML overlay via `js_rave_error`.
// The Bevy in-canvas LogDrawer only shows when the render loop is
// alive; if the tab reloads or the canvas fails to render steadily,
// runtime errors silence themselves visually. Mirroring here lands the
// same content on the sessionStorage-persistent HTML overlay so every
// error survives every reload cycle. ERROR.md axiom: never silence.
#[cfg(target_arch = "wasm32")]
fn mirror_errorlog_to_overlay(
    error_log: Res<bevy_observability::ErrorLog>,
    mut mirrored_count: Local<usize>,
) {
    let current = error_log.0.len();
    // Ring-buffer eviction: if len shrinks, reset and re-emit whatever
    // remains so we don't skip forward past evicted-then-refilled slots.
    if current < *mirrored_count {
        *mirrored_count = 0;
    }
    for entry in &error_log.0[*mirrored_count..] {
        let sev = match entry.severity {
            bevy_observability::Severity::Note => "note",
            bevy_observability::Severity::Warn => "warn",
            bevy_observability::Severity::Error => "error",
        };
        js_rave_error(&format!("[error_log:{sev}] {}", entry.message));
    }
    *mirrored_count = current;
}
