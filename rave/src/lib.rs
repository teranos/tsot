use std::sync::Mutex;

mod build_info;
mod error;
mod identity;
mod net;
mod room;

use bevy::asset::AssetMetaCheck;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::log::{
    tracing::{self, Subscriber},
    tracing_subscriber::Layer,
    BoxedLayer, LogPlugin,
};
use bevy::prelude::*;
use bevy::window::WindowPlugin;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// Out-of-Bevy error path. Calls window.__raveError defined in
// index.html — surfaces panics + ERROR-level tracing to the HTML
// overlay even when Bevy itself is dead. Without this, anything that
// panics before the first Update tick is silently swallowed: the
// in-canvas drawer never renders because the systems that update it
// never run.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = "__raveError")]
    fn js_rave_error(msg: &str);

    #[wasm_bindgen(js_namespace = window, js_name = "__raveErrorTyped")]
    fn js_rave_error_typed(json: &str);

    #[wasm_bindgen(js_namespace = window, js_name = "__raveScreenshot")]
    fn js_rave_screenshot(filename: &str);
}

#[cfg(not(target_arch = "wasm32"))]
fn js_rave_error(_msg: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn js_rave_error_typed(_json: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn js_rave_screenshot(_filename: &str) {}

// Queue from the panic hook into the ECS. The hook runs outside Bevy
// systems, so it can't write the Resource directly. Drain into
// ErrorLog every frame.
static PANIC_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Queue from the tracing Layer (Bevy + our own info!/warn!/error!
// macros) into the ECS. Drain into ErrorLog every frame.
static LOG_QUEUE: Mutex<Vec<(Severity, String)>> = Mutex::new(Vec::new());

// Floor extent — half-size of the playable XZ square at Y=0. The
// player can walk anywhere inside [-FLOOR_HALF, FLOOR_HALF] on X and Z.
// Vertical movement is gone; the rave room is a flat floor, not a
// 3D cube.
const FLOOR_HALF: f32 = 500.0;

// Production relay multiaddr — shared with roam, served by the relayer
// binary at `relay.sbvh.nl`. The 12D3KooW… is the relay's deterministic
// PeerId derived from its persistent identity secret in AWS Secrets
// Manager; same value roam's JS bridge uses.
#[cfg(target_arch = "wasm32")]
const RELAY_MULTIADDR: &str =
    "/dns4/relay.sbvh.nl/tcp/443/wss/p2p/12D3KooWMSVxS7ntMVuvVADgZWMZwsjyYmcZvhnyQAJ53PtSJHpN";

#[cfg(target_arch = "wasm32")]
const POSITIONS_TOPIC: &str = "rave-positions/v1";

// Bridge between the async boot task (spawn_local on the JS microtask
// loop) and Bevy's Update schedule. The async boot writes Net into this
// cell once identity is resolved + Swarm is constructed; the Update
// system `install_pending_net` takes it the next frame and inserts it
// as a NonSend resource. wasm32 is single-threaded so the RefCell never
// races.
#[cfg(target_arch = "wasm32")]
thread_local! {
    static PENDING_NET: std::cell::RefCell<Option<net::Net>> =
        const { std::cell::RefCell::new(None) };
}

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
                    custom_layer: install_capture_layer,
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
        .add_systems(Startup, (setup, room::setup_room))
        .add_systems(
            Update,
            (
                drain_panics,
                drain_logs,
                update_fps,
                update_error_list,
                toggle_log_drawer,
                screenshot_on_p,
                room::move_player,
                room::camera_follow,
            ),
        );

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, update_clock);

    #[cfg(target_arch = "wasm32")]
    {
        // Network resource lives as NonSend because Net contains Rc +
        // RefCell (the swarm task uses Rc clones for the shared event
        // queue). Inserted as None; the async boot fills it.
        app.insert_non_send_resource::<Option<net::Net>>(None);
        app.insert_resource(RemotePlayers::default());
        app.add_systems(
            Update,
            (
                install_pending_net,
                flush_typed_errors,
                drain_net_events,
                publish_self_position,
                render_remote_players,
                update_net_stats,
            )
                .chain(),
        );

        // Kick off the async boot. Awaits the JS identity bridge,
        // constructs Net, subscribes to the positions topic, stashes
        // it in PENDING_NET for the Update system to pick up next
        // frame.
        wasm_bindgen_futures::spawn_local(boot_net());
    }

    app.run();
}

#[cfg(target_arch = "wasm32")]
async fn boot_net() {
    use wasm_bindgen::JsCast;

    // Load identity bytes from IndexedDB, or mint+persist fresh on
    // first visit. Decode failure surfaces through __raveError; the
    // boot continues with fresh bytes only if load returned null —
    // never on decode error (that would silently rotate PeerId).
    let load_promise = identity::js_rave_load_identity();
    let identity_bytes: Vec<u8> = match wasm_bindgen_futures::JsFuture::from(load_promise).await {
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
                    return;
                }
            }
        }
        Ok(_) => {
            // null/undefined — first visit. Generate + persist.
            match identity::generate_identity_protobuf() {
                Ok(fresh) => {
                    let arr = js_sys::Uint8Array::from(fresh.as_slice());
                    let save_promise = identity::js_rave_save_identity(arr);
                    if let Err(e) = wasm_bindgen_futures::JsFuture::from(save_promise).await {
                        error::emit_region(
                            error::Severity::Warn,
                            "identity-save",
                            "IndexedDB save rejected",
                            format!("{e:?}"),
                        );
                    }
                    fresh
                }
                Err(e) => {
                    error::emit_region(
                        error::Severity::Error,
                        "identity-generate",
                        "Ed25519 keypair generation failed",
                        format!("{e:?}"),
                    );
                    return;
                }
            }
        }
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "identity-load",
                "IndexedDB load rejected",
                format!("{e:?}"),
            );
            return;
        }
    };

    let net = match net::Net::new(vec![RELAY_MULTIADDR.to_string()], Some(&identity_bytes)) {
        Ok(n) => n,
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "net-new",
                "Swarm construction failed",
                format!("{e:?}"),
            );
            return;
        }
    };

    if let Err(e) = net.subscribe(&net::Topic(POSITIONS_TOPIC.to_string())) {
        error::emit_region(
            error::Severity::Error,
            "net-subscribe",
            "subscribe to rave-positions/v1 failed",
            format!("{e:?}"),
        );
        return;
    }

    PENDING_NET.with(|cell| *cell.borrow_mut() = Some(net));
}

#[cfg(target_arch = "wasm32")]
fn install_pending_net(mut maybe_net: NonSendMut<Option<net::Net>>) {
    if maybe_net.is_some() {
        return;
    }
    PENDING_NET.with(|cell| {
        if let Some(n) = cell.borrow_mut().take() {
            *maybe_net = Some(n);
        }
    });
}

// 10Hz publish of self position to rave-positions/v1. Accumulates
// delta_secs into a Local until 100ms have passed, then publishes one
// RavePosition. Publish failures surface as NetEvent::Error via the
// drain system; no manual error handling needed here.
#[cfg(target_arch = "wasm32")]
fn publish_self_position(
    time: Res<Time>,
    mut acc: Local<f32>,
    players: Query<&Transform, With<room::PlayerCell>>,
    maybe_net: NonSend<Option<net::Net>>,
) {
    let Some(n) = maybe_net.as_ref() else {
        return;
    };
    *acc += time.delta_secs();
    if *acc < 0.1 {
        return;
    }
    *acc = 0.0;

    let Some(tf) = players.iter().next() else {
        return;
    };
    let pos = net::RavePosition {
        peer: n.identity().0.clone(),
        x: tf.translation.x,
        y: tf.translation.y,
        z: tf.translation.z,
        at_ms: js_sys::Date::now() as u64,
    };
    let bytes = match serde_json::to_vec(&pos) {
        Ok(b) => b,
        Err(e) => {
            error::emit_region(
                error::Severity::Error,
                "publish-serialize",
                "RavePosition serialize failed",
                format!("{e}"),
            );
            return;
        }
    };
    // publish() failing means the cmd channel is closed — the swarm
    // task died. Critical. Surface, don't swallow.
    if let Err(e) = n.publish(&net::Topic(POSITIONS_TOPIC.to_string()), &bytes) {
        error::emit_region(
            error::Severity::Error,
            "publish-send",
            "publish to rave-positions/v1 failed",
            format!("{e:?}"),
        );
    }
}

// Pulls every Error from the rave::error thread_local buffer, pushes
// each to the in-canvas drawer (formatted with severity/region/title)
// AND to the HTML overlay via __raveErrorTyped (typed JSON, so the
// receiving JS keeps the structured fields). Single source of truth
// for typed errors crossing the wasm→JS boundary.
#[cfg(target_arch = "wasm32")]
fn flush_typed_errors(mut error_log: ResMut<ErrorLog>) {
    for err in error::drain() {
        let region = err.context.region.as_deref().unwrap_or("?");
        let severity_for_drawer = match err.severity {
            sacred_error::Severity::Info => Severity::Note,
            sacred_error::Severity::Warn => Severity::Warn,
            sacred_error::Severity::Error => Severity::Error,
            sacred_error::Severity::Panic => Severity::Error,
        };
        error_log.emit(
            severity_for_drawer,
            format!("[{region}] {} — {}", err.title, err.why),
        );
        match serde_json::to_string(&err) {
            Ok(json) => js_rave_error_typed(&json),
            Err(e) => js_rave_error(&format!("[flush_typed_errors serialize] {e}")),
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn drain_net_events(
    maybe_net: NonSend<Option<net::Net>>,
    mut error_log: ResMut<ErrorLog>,
    mut remotes: ResMut<RemotePlayers>,
) {
    let Some(n) = maybe_net.as_ref() else {
        return;
    };
    let self_peer = n.identity().0.clone();
    let now_ms = js_sys::Date::now() as u64;

    for ev in n.poll_events() {
        match ev {
            net::NetEvent::PeerUp { peer, .. } => {
                error_log.emit(Severity::Note, format!("[net] peer up: {}", peer.0));
            }
            net::NetEvent::PeerDown { peer, reason } => {
                error_log.emit(
                    Severity::Warn,
                    format!("[net] peer down: {} ({reason})", peer.0),
                );
            }
            net::NetEvent::Message { topic, bytes, .. } => {
                // Route rave-positions traffic into RemotePlayers (R10).
                // Don't push every gossip message to the drawer — at 10Hz
                // per peer it floods the text node and tanks FPS.
                if topic.0 == POSITIONS_TOPIC {
                    match serde_json::from_slice::<net::RavePosition>(&bytes) {
                        Ok(pos) => {
                            if pos.peer != self_peer {
                                let entry =
                                    remotes.0.entry(pos.peer.clone()).or_default();
                                entry.pos = Vec3::new(pos.x, pos.y, pos.z);
                                entry.last_seen_ms = now_ms;
                            }
                        }
                        Err(e) => {
                            error::emit_region(
                                error::Severity::Error,
                                "decode-rave-position",
                                "malformed RavePosition wire payload",
                                format!("{e}"),
                            );
                        }
                    }
                }
                // Other topics: silently ignored. Drawer stays quiet.
            }
            net::NetEvent::SubscriptionChange {
                topic,
                peer,
                joined,
            } => {
                error_log.emit(
                    Severity::Note,
                    format!(
                        "[net] {} on {} by {}",
                        if joined { "+sub" } else { "-sub" },
                        topic.0,
                        peer.0
                    ),
                );
            }
            net::NetEvent::Error(err) => {
                error_log.emit(Severity::Error, format!("[net] {err:?}"));
            }
        }
    }
}

// R10 — one entry per other peer's last-known position. The Bevy
// Entity is spawned lazily by render_remote_players when we first
// receive a position; subsequent updates only mutate the Transform.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct RemoteEntry {
    pos: Vec3,
    last_seen_ms: u64,
    entity: Option<Entity>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Resource, Default)]
struct RemotePlayers(std::collections::HashMap<String, RemoteEntry>);

#[cfg(target_arch = "wasm32")]
#[derive(Component)]
struct RemotePlayerCell;

// R10 render. Each remote peer becomes a Sphere at the position they
// last broadcast on rave-positions/v1. Y is taken from the wire so a
// peer that drifts off the floor will still render where they say
// they are.
#[cfg(target_arch = "wasm32")]
fn render_remote_players(
    mut commands: Commands,
    mut remotes: ResMut<RemotePlayers>,
    mut transforms: Query<&mut Transform, With<RemotePlayerCell>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let now_ms = js_sys::Date::now() as u64;
    let stale_cutoff = now_ms.saturating_sub(30_000);

    let stale_peers: Vec<String> = remotes
        .0
        .iter()
        .filter(|(_, e)| e.last_seen_ms < stale_cutoff)
        .map(|(p, _)| p.clone())
        .collect();
    for peer in stale_peers {
        if let Some(entry) = remotes.0.remove(&peer) {
            if let Some(entity) = entry.entity {
                commands.entity(entity).despawn();
            }
        }
    }

    for (_peer, entry) in remotes.0.iter_mut() {
        match entry.entity {
            None => {
                let mesh = meshes.add(Sphere::new(20.0));
                let mat = materials
                    .add(StandardMaterial::from(Color::srgb(0.9, 0.3, 0.85)));
                let id = commands
                    .spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(mat),
                        Transform::from_translation(entry.pos),
                        RemotePlayerCell,
                    ))
                    .id();
                entry.entity = Some(id);
            }
            Some(entity) => {
                if let Ok(mut tf) = transforms.get_mut(entity) {
                    tf.translation = entry.pos;
                }
            }
        }
    }
}

// LogPlugin's custom_layer hook — called once at plugin build time.
// Pattern from https://github.com/bevyengine/bevy/blob/v0.19.0/examples/app/log_layers.rs
fn install_capture_layer(_app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(CaptureLayer))
}

// Captures every tracing event Bevy or our code emits. Only WARN +
// ERROR levels propagate to the in-canvas drawer; INFO/DEBUG/TRACE
// would flood it. LogPlugin's default fmt layer still emits everything
// to the browser console, so the lower-severity events are not lost —
// they live in the console channel.
struct CaptureLayer;

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: bevy::log::tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        let severity = match level {
            tracing::Level::ERROR => Severity::Error,
            tracing::Level::WARN => Severity::Warn,
            _ => return,
        };

        let target = event.metadata().target();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let formatted = format!("{target}: {}", visitor.message);

        // ERROR-level tracing skips the drawer queue and goes straight
        // to the HTML overlay too — survives the Bevy-never-runs case.
        if matches!(severity, Severity::Error) {
            js_rave_error(&format!("[tracing ERROR] {formatted}"));
        }

        let mut q = LOG_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push((severity, formatted));
    }
}

// tracing events carry their message as a Debug-formatted "message"
// field. Visit collects it.
#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Severity {
    Note,
    Warn,
    Error,
}

struct ErrorEntry {
    severity: Severity,
    message: String,
}

#[derive(Resource, Default)]
struct ErrorLog(Vec<ErrorEntry>);

impl ErrorLog {
    fn emit(&mut self, severity: Severity, message: impl Into<String>) {
        self.0.push(ErrorEntry {
            severity,
            message: message.into(),
        });
    }
}

#[derive(Component)]
struct ClockText;

#[derive(Component)]
struct FpsText;

#[derive(Component)]
struct ErrorListText;

#[derive(Component)]
struct LogDrawer;

#[cfg(target_arch = "wasm32")]
#[derive(Component)]
struct NetStatsText;

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 80.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Single point light high above the cube — surface illumination,
    // simulates sunlight through the water from one direction.
    commands.spawn((
        PointLight {
            shadow_maps_enabled: true,
            intensity: 8_000_000.0,
            range: 1000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 500.0, 0.0),
    ));

    commands.spawn((
        Text::new("HH:MM:SS.mmm  GMT±HHMM"),
        ClockText,
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgb(0.59, 0.59, 0.59)),
        Node {
            position_type: PositionType::Absolute,
            top: px(6),
            right: px(6),
            ..default()
        },
    ));

    commands.spawn((
        Text::new(format!("rave · {} · {}", build_info::COMMIT, build_info::BUILT_AT)),
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgb(0.55, 0.55, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(6),
            right: px(6),
            ..default()
        },
    ));

    commands
        .spawn((
            LogDrawer,
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                width: Val::Percent(100.0),
                height: Val::Percent(40.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(12)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
            Visibility::Hidden,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(
                    "keys: WASD move · Space up · Shift down · `/\\ toggle · P screenshot",
                ),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.55, 0.55, 0.55)),
            ));
            parent.spawn((
                Text::new("net: …"),
                NetStatsText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.85, 0.65)),
            ));
            parent.spawn((
                Text::new("FPS"),
                FpsText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
            ));
            parent.spawn((
                Text::new(""),
                ErrorListText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.4, 0.4)),
            ));
        });

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: px(20),
            left: Val::Percent(50.0),
            margin: UiRect {
                left: px(-180),
                ..default()
            },
            width: px(360),
            height: px(40),
            ..default()
        })
        .with_children(|parent| {
            for _ in 0..9 {
                parent.spawn((
                    Node {
                        width: px(36),
                        height: px(36),
                        margin: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.12, 0.12, 0.12)),
                ));
            }
        });
}

fn drain_panics(mut log: ResMut<ErrorLog>) {
    let mut q = PANIC_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    for msg in q.drain(..) {
        log.emit(Severity::Error, format!("PANIC: {msg}"));
    }
}

fn drain_logs(mut log: ResMut<ErrorLog>) {
    let mut q = LOG_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
    for (sev, msg) in q.drain(..) {
        log.emit(sev, msg);
    }
}

fn update_fps(
    diagnostics: Res<DiagnosticsStore>,
    mut texts: Query<&mut Text, With<FpsText>>,
    mut log: ResMut<ErrorLog>,
) {
    let Some(diag) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) else {
        log.emit(
            Severity::Error,
            "FrameTimeDiagnosticsPlugin::FPS not registered",
        );
        return;
    };
    let display = match diag.smoothed() {
        Some(fps) => format!("FPS: {fps:.1}"),
        None => "FPS: warming".to_string(),
    };
    for mut text in &mut texts {
        **text = display.clone();
    }
}

fn update_error_list(
    log: Res<ErrorLog>,
    mut texts: Query<&mut Text, With<ErrorListText>>,
) {
    if !log.is_changed() && !log.is_added() {
        return;
    }
    for mut text in &mut texts {
        **text = log
            .0
            .iter()
            .map(|e| format!("[{:?}] {}", e.severity, e.message))
            .collect::<Vec<_>>()
            .join("\n");
    }
}

// 'P' copies the canvas as a PNG to the system clipboard via the JS side.
fn screenshot_on_p(keys: Res<ButtonInput<KeyCode>>) {
    if keys.just_pressed(KeyCode::KeyP) {
        js_rave_screenshot("");
    }
}

// Condensed network stats line in the drawer: self peer-id (short),
// remote peer count, and the configured topic. Updated every frame.
#[cfg(target_arch = "wasm32")]
fn update_net_stats(
    maybe_net: NonSend<Option<net::Net>>,
    remotes: Res<RemotePlayers>,
    mut texts: Query<&mut Text, With<NetStatsText>>,
) {
    let Some(t) = texts.iter_mut().next() else {
        return;
    };
    let mut t = t;
    match maybe_net.as_ref() {
        Some(n) => {
            let id = &n.identity().0;
            let short = if id.len() > 10 { &id[id.len() - 10..] } else { id };
            t.0 = format!(
                "net: …{short} · peers={} · topic={POSITIONS_TOPIC}",
                remotes.0.len()
            );
        }
        None => {
            t.0 = "net: booting…".into();
        }
    }
}

fn toggle_log_drawer(
    keys: Res<ButtonInput<KeyCode>>,
    mut drawers: Query<&mut Visibility, With<LogDrawer>>,
) {
    if !keys.just_pressed(KeyCode::Backquote) && !keys.just_pressed(KeyCode::Backslash) {
        return;
    }
    for mut vis in &mut drawers {
        *vis = match *vis {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
}

#[cfg(target_arch = "wasm32")]
fn update_clock(
    mut texts: Query<&mut Text, With<ClockText>>,
    mut log: ResMut<ErrorLog>,
) {
    let d = js_sys::Date::new_0();

    let Some(iso) = d.to_iso_string().as_string() else {
        log.emit(Severity::Error, "Date::to_iso_string() returned non-string");
        return;
    };
    let Some(time) = iso.get(11..23) else {
        log.emit(
            Severity::Error,
            format!("Date ISO too short ({} chars): {iso:?}", iso.len()),
        );
        return;
    };

    let Some(s) = d.to_string().as_string() else {
        log.emit(Severity::Error, "Date::to_string() returned non-string");
        return;
    };
    let Some(tz) = s.get(25..33) else {
        log.emit(
            Severity::Error,
            format!("Date string too short ({} chars): {s:?}", s.len()),
        );
        return;
    };

    for mut text in &mut texts {
        **text = format!("{time}  {tz}");
    }
}

// Tests for room::touch_drag_to_plane live in room.rs alongside the
// function under test.
