// Verify adaptive polling: the row should now carry push_time + p50 + p90
// alongside repo + branch + sha. Watcher picks adaptive sleep when in_progress.
use std::sync::Mutex;

mod build_info;
mod error;
mod identity;
mod net;

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

    #[wasm_bindgen(js_namespace = window, js_name = "__raveScreenshot")]
    fn js_rave_screenshot(filename: &str);
}

#[cfg(not(target_arch = "wasm32"))]
fn js_rave_error(_msg: &str) {}

#[cfg(not(target_arch = "wasm32"))]
fn js_rave_screenshot(_filename: &str) {}

// Queue from the panic hook into the ECS. The hook runs outside Bevy
// systems, so it can't write the Resource directly. Drain into
// ErrorLog every frame.
static PANIC_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Queue from the tracing Layer (Bevy + our own info!/warn!/error!
// macros) into the ECS. Drain into ErrorLog every frame.
static LOG_QUEUE: Mutex<Vec<(Severity, String)>> = Mutex::new(Vec::new());

// Cube extent — world clamps to [-CUBE_HALF, CUBE_HALF] on each axis.
const CUBE_HALF: f32 = 300.0;

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
        .add_systems(Startup, (setup, setup_cells))
        .add_systems(
            Update,
            (
                drain_panics,
                drain_logs,
                update_fps,
                update_error_list,
                toggle_log_drawer,
                screenshot_on_p,
                move_player_cell,
                camera_follow,
                follow_tether,
                eat_algae,
                die_algae,
                drift_water,
                wobble_player_cell,
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
                drain_net_events,
                publish_self_position,
                render_remote_players,
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
                    js_rave_error("[__raveLoadIdentity] returned non-Uint8Array value");
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
                        js_rave_error(&format!("[__raveSaveIdentity rejected] {e:?}"));
                    }
                    fresh
                }
                Err(e) => {
                    js_rave_error(&format!("[generate_identity_protobuf] {e:?}"));
                    return;
                }
            }
        }
        Err(e) => {
            js_rave_error(&format!("[__raveLoadIdentity rejected] {e:?}"));
            return;
        }
    };

    let net = match net::Net::new(vec![RELAY_MULTIADDR.to_string()], Some(&identity_bytes)) {
        Ok(n) => n,
        Err(e) => {
            js_rave_error(&format!("[Net::new] {e:?}"));
            return;
        }
    };

    if let Err(e) = net.subscribe(&net::Topic(POSITIONS_TOPIC.to_string())) {
        js_rave_error(&format!("[Net::subscribe] {e:?}"));
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
    players: Query<&Transform, With<PlayerCell>>,
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
            js_rave_error(&format!("[publish_self_position serialize] {e}"));
            return;
        }
    };
    // publish() failing means the cmd channel is closed — the swarm
    // task died. Critical. Surface, don't swallow.
    if let Err(e) = n.publish(&net::Topic(POSITIONS_TOPIC.to_string()), &bytes) {
        js_rave_error(&format!("[publish_self_position] {e:?}"));
    }
}

// R10: remote player state. One entry per other peer's last-known
// position. The Bevy Entity is spawned lazily by render_remote_players
// the first time we receive a position for that peer; subsequent
// updates only mutate the Transform.
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
                // Route rave-positions traffic to the RemotePlayers map.
                // Don't push every gossip message into the drawer — at
                // 10Hz per peer it floods the text node and tanks FPS.
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
                            error_log.emit(
                                Severity::Error,
                                format!("[net] decode RavePosition: {e}"),
                            );
                        }
                    }
                }
                // Other topics: do nothing for now. Drawer stays quiet.
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

// R10 render: walks the RemotePlayers map. New peers get a Capsule
// mesh (humanoid placeholder, not a cell). Existing entries get their
// Transform updated. Entries older than 30s get despawned and removed.
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
                let mesh = meshes.add(Capsule3d::new(6.0, 18.0));
                let mat =
                    materials.add(StandardMaterial::from(Color::srgb(0.9, 0.3, 0.85)));
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

#[derive(Component)]
struct PlayerCell;

#[derive(Component)]
struct Algae;

#[derive(Component)]
struct CellRadius(f32);

#[derive(Component)]
struct WaterParticle;

#[derive(Component)]
struct Drift(Vec3);

#[derive(Component)]
struct Velocity(Vec3);

#[derive(Component)]
struct Tethered(Vec3);

#[derive(Component)]
struct Dying {
    progress: f32,
    duration: f32,
}

fn setup_cells(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let player_mesh = meshes.add(Sphere::new(20.0));
    let halo_mesh = meshes.add(Sphere::new(50.0));
    let nucleus_mesh = meshes.add(Sphere::new(7.0));
    let algae_mesh = meshes.add(Sphere::new(6.0));
    let water_mesh = meshes.add(Sphere::new(2.0));

    // Player is translucent so the nucleus inside is visible.
    let player_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.35, 0.85, 0.55, 0.6),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let halo_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.5, 0.95, 0.7, 0.12),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let nucleus_mat = materials.add(StandardMaterial::from(Color::srgb(0.15, 0.35, 0.25)));
    let algae_mat = materials.add(StandardMaterial::from(Color::srgb(0.7, 0.9, 0.3)));
    let water_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.85, 0.92, 1.0, 0.18),
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Player cell — center of cube. Velocity-driven; wobbles; eats.
    commands.spawn((
        PlayerCell,
        CellRadius(20.0),
        Velocity(Vec3::ZERO),
        Mesh3d(player_mesh),
        MeshMaterial3d(player_mat),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));

    // Halo — bigger translucent envelope at same center.
    commands.spawn((
        Tethered(Vec3::ZERO),
        Mesh3d(halo_mesh),
        MeshMaterial3d(halo_mat),
        Transform::default(),
    ));

    // Nucleus — small opaque sphere inside the translucent player.
    commands.spawn((
        Tethered(Vec3::ZERO),
        Mesh3d(nucleus_mesh),
        MeshMaterial3d(nucleus_mat),
        Transform::default(),
    ));

    // Algae — deterministically scattered through the cube.
    for i in 0..40 {
        let i_f = i as f32;
        let x = ((i_f * 1.7).sin() * 250.0).clamp(-CUBE_HALF, CUBE_HALF);
        let y = ((i_f * 2.3 + 0.5).sin() * 250.0).clamp(-CUBE_HALF, CUBE_HALF);
        let z = ((i_f * 1.1 + 0.9).cos() * 250.0).clamp(-CUBE_HALF, CUBE_HALF);
        commands.spawn((
            Algae,
            CellRadius(6.0),
            Mesh3d(algae_mesh.clone()),
            MeshMaterial3d(algae_mat.clone()),
            Transform::from_xyz(x, y, z),
        ));
    }

    // Water particles — drift around and push away from the player cell.
    for i in 0..200 {
        let i_f = i as f32;
        let x = ((i_f * 0.37).sin() * 280.0).clamp(-CUBE_HALF, CUBE_HALF);
        let y = ((i_f * 0.71 + 0.3).cos() * 280.0).clamp(-CUBE_HALF, CUBE_HALF);
        let z = ((i_f * 0.53 + 0.7).sin() * 280.0).clamp(-CUBE_HALF, CUBE_HALF);
        let drift_x = (i_f * 0.7).sin() * 10.0;
        let drift_y = (i_f * 1.3).cos() * 10.0;
        let drift_z = (i_f * 0.9 + 0.4).sin() * 10.0;
        commands.spawn((
            WaterParticle,
            Drift(Vec3::new(drift_x, drift_y, drift_z)),
            Mesh3d(water_mesh.clone()),
            MeshMaterial3d(water_mat.clone()),
            Transform::from_xyz(x, y, z),
        ));
    }
}

// WASD horizontal plane, Space/Shift vertical. Drag dampens. Cube
// walls clamp position and zero the perpendicular velocity component —
// no escape, no bounce.
fn move_player_cell(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut players: Query<(&mut Transform, &mut Velocity), With<PlayerCell>>,
) {
    let mut accel = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        accel.z -= 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        accel.z += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        accel.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        accel.x += 1.0;
    }
    if keys.pressed(KeyCode::Space) {
        accel.y += 1.0;
    }
    if keys.pressed(KeyCode::ShiftLeft) {
        accel.y -= 1.0;
    }
    let accel = accel.normalize_or_zero() * 900.0;
    let drag_per_sec = 2.4;
    let dt = time.delta_secs();
    for (mut t, mut v) in &mut players {
        v.0 += accel * dt;
        let drag = (1.0 - drag_per_sec * dt).max(0.0);
        v.0 *= drag;
        t.translation += v.0 * dt;
        if t.translation.x.abs() > CUBE_HALF {
            t.translation.x = t.translation.x.clamp(-CUBE_HALF, CUBE_HALF);
            v.0.x = 0.0;
        }
        if t.translation.y.abs() > CUBE_HALF {
            t.translation.y = t.translation.y.clamp(-CUBE_HALF, CUBE_HALF);
            v.0.y = 0.0;
        }
        if t.translation.z.abs() > CUBE_HALF {
            t.translation.z = t.translation.z.clamp(-CUBE_HALF, CUBE_HALF);
            v.0.z = 0.0;
        }
    }
}

// Third-person follow — camera trails behind + above the player.
fn camera_follow(
    cells: Query<&Transform, (With<PlayerCell>, Without<Camera3d>)>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(cell_t) = cells.iter().next() else {
        return;
    };
    let offset = Vec3::new(0.0, 80.0, 200.0);
    for mut cam_t in &mut cameras {
        cam_t.translation = cell_t.translation + offset;
        cam_t.look_at(cell_t.translation, Vec3::Y);
    }
}

// Halo + nucleus track the cell exactly so they read as part of the
// same body. The cell's own wobble doesn't propagate (these are
// siblings, not children) so they keep their shape.
fn follow_tether(
    cells: Query<&Transform, (With<PlayerCell>, Without<Tethered>)>,
    mut followers: Query<(&mut Transform, &Tethered)>,
) {
    let Some(cell_t) = cells.iter().next() else {
        return;
    };
    for (mut t, tether) in &mut followers {
        t.translation = cell_t.translation + tether.0;
    }
}

fn drift_water(
    time: Res<Time>,
    players: Query<&Transform, (With<PlayerCell>, Without<WaterParticle>)>,
    mut particles: Query<(&mut Transform, &Drift), With<WaterParticle>>,
) {
    let dt = time.delta_secs();
    let player_pos = players.iter().next().map(|t| t.translation);
    for (mut t, drift) in &mut particles {
        t.translation += drift.0 * dt;

        if let Some(p) = player_pos {
            let delta = t.translation - p;
            let dist_sq = delta.length_squared();
            let near = 90.0;
            if dist_sq < near * near && dist_sq > 0.001 {
                let dist = dist_sq.sqrt();
                let strength = (near - dist) / near * 80.0;
                let n = delta / dist;
                t.translation += n * strength * dt;
            }
        }

        // Wrap to opposite face so the medium stays evenly populated.
        if t.translation.x.abs() > CUBE_HALF {
            t.translation.x = -t.translation.x.signum() * CUBE_HALF;
        }
        if t.translation.y.abs() > CUBE_HALF {
            t.translation.y = -t.translation.y.signum() * CUBE_HALF;
        }
        if t.translation.z.abs() > CUBE_HALF {
            t.translation.z = -t.translation.z.signum() * CUBE_HALF;
        }
    }
}

// Three-axis lumpy organic wobble — nine sines (three per axis) with
// prime-ish frequencies + phase offsets so no axis returns to a
// perfect sphere at the same instant. CellRadius drives base scale so
// growth (from eating) reads as the body getting bigger, not the
// wobble amplifying.
fn wobble_player_cell(
    time: Res<Time>,
    mut cells: Query<(&mut Transform, &CellRadius), With<PlayerCell>>,
) {
    let t = time.elapsed_secs();
    let wx = (t * 3.5).sin() * 0.06 + (t * 5.7).sin() * 0.035 + (t * 2.1).sin() * 0.02;
    let wy = (t * 3.1 + 1.0).sin() * 0.06
        + (t * 4.9 + 0.4).sin() * 0.035
        + (t * 2.7 + 0.8).sin() * 0.02;
    let wz = (t * 2.9 + 0.6).sin() * 0.06
        + (t * 4.3 + 0.2).sin() * 0.035
        + (t * 5.1 + 1.4).sin() * 0.02;
    for (mut tr, r) in &mut cells {
        let base = r.0 / 20.0;
        tr.scale.x = base * (1.0 + wx);
        tr.scale.y = base * (1.0 + wy);
        tr.scale.z = base * (1.0 + wz);
    }
}

// Eating is a transfer, not a delete. Algae gets a Dying marker
// (animated away over ~0.25s), the cell's CellRadius grows. Mass
// conserved — what you ate goes into you.
fn eat_algae(
    mut commands: Commands,
    mut players: Query<(&Transform, &mut CellRadius), (With<PlayerCell>, Without<Algae>)>,
    algae: Query<(Entity, &Transform, &CellRadius), (With<Algae>, Without<Dying>)>,
) {
    for (player_t, mut player_r) in &mut players {
        for (algae_e, algae_t, algae_r) in &algae {
            let dist = (player_t.translation - algae_t.translation).length();
            if dist < player_r.0 + algae_r.0 {
                commands.entity(algae_e).insert(Dying {
                    progress: 0.0,
                    duration: 0.25,
                });
                player_r.0 += 0.4;
            }
        }
    }
}

// Animates Dying entities — shrinks scale and fades alpha together,
// despawn at progress >= 1.
fn die_algae(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut dying: Query<(
        Entity,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
        &mut Dying,
    )>,
) {
    let dt = time.delta_secs();
    for (e, mut t, mat, mut d) in &mut dying {
        d.progress += dt / d.duration;
        let s: f32 = (1.0 - d.progress).max(0.0);
        t.scale = Vec3::splat(s);
        if let Some(mut m) = materials.get_mut(&mat.0) {
            let c = m.base_color.to_srgba();
            m.base_color = Color::srgba(c.red, c.green, c.blue, s);
            m.alpha_mode = AlphaMode::Blend;
        }
        if d.progress >= 1.0 {
            commands.entity(e).despawn();
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

// Cap to last N entries so a runaway producer (gossip-flood, panic
// loop) can't grow the in-canvas text node unbounded and tank FPS.
const ERROR_LOG_CAP: usize = 50;

impl ErrorLog {
    fn emit(&mut self, severity: Severity, message: impl Into<String>) {
        self.0.push(ErrorEntry {
            severity,
            message: message.into(),
        });
        if self.0.len() > ERROR_LOG_CAP {
            let drop = self.0.len() - ERROR_LOG_CAP;
            self.0.drain(0..drop);
        }
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

fn toggle_log_drawer(
    keys: Res<ButtonInput<KeyCode>>,
    mut drawers: Query<&mut Visibility, With<LogDrawer>>,
) {
    if !keys.just_pressed(KeyCode::Slash) {
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
