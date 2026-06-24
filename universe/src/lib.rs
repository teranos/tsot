use std::sync::Mutex;

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

// Queue from the panic hook into the ECS. The hook runs outside Bevy
// systems, so it can't write the Resource directly. Drain into
// ErrorLog every frame.
static PANIC_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());

// Queue from the tracing Layer (Bevy + our own info!/warn!/error!
// macros) into the ECS. Drain into ErrorLog every frame.
static LOG_QUEUE: Mutex<Vec<(Severity, String)>> = Mutex::new(Vec::new());

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.03, 0.12, 0.22)))
        .insert_resource(ErrorLog::default())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "universe".to_string(),
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
        )
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_systems(Startup, (install_panic_hook, setup, setup_cells))
        .add_systems(
            Update,
            (
                drain_panics,
                drain_logs,
                update_fps,
                update_error_list,
                toggle_log_drawer,
                move_player_cell,
                eat_algae,
                drift_water,
                wobble_player_cell,
            ),
        );

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, update_clock);

    app.run();
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
struct Drift(Vec2);

fn setup_cells(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    // Player cell — center of dish
    commands.spawn((
        PlayerCell,
        CellRadius(20.0),
        Mesh2d(meshes.add(Circle::new(20.0))),
        MeshMaterial2d(materials.add(Color::srgb(0.3, 0.9, 0.5))),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));

    // Algae — scattered around the dish
    for i in 0..20 {
        let angle = (i as f32) * std::f32::consts::TAU / 20.0;
        let radius = 150.0 + (i as f32) * 8.0;
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;
        commands.spawn((
            Algae,
            CellRadius(6.0),
            Mesh2d(meshes.add(Circle::new(6.0))),
            MeshMaterial2d(materials.add(Color::srgb(0.7, 0.9, 0.3))),
            Transform::from_xyz(x, y, 0.0),
        ));
    }

    // Water particles — drift around and push away from the player cell.
    // Deterministic positions so we don't need rand on wasm.
    let water_material = materials.add(Color::srgba(0.85, 0.92, 1.0, 0.18));
    let water_mesh = meshes.add(Circle::new(2.0));
    for i in 0..80 {
        let i_f = i as f32;
        let angle = i_f * 0.37 * std::f32::consts::TAU;
        let radius = 40.0 + (i_f * 13.7) % 480.0;
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;
        let drift_x = (i_f * 0.7).sin() * 10.0;
        let drift_y = (i_f * 1.3).cos() * 10.0;
        commands.spawn((
            WaterParticle,
            Drift(Vec2::new(drift_x, drift_y)),
            Mesh2d(water_mesh.clone()),
            MeshMaterial2d(water_material.clone()),
            Transform::from_xyz(x, y, -1.0),
        ));
    }
}

fn move_player_cell(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut players: Query<&mut Transform, With<PlayerCell>>,
) {
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir.x += 1.0;
    }
    let dir = dir.normalize_or_zero();
    let speed = 200.0;
    let dt = time.delta_secs();
    for mut t in &mut players {
        t.translation.x += dir.x * speed * dt;
        t.translation.y += dir.y * speed * dt;
    }
}

fn drift_water(
    time: Res<Time>,
    players: Query<&Transform, (With<PlayerCell>, Without<WaterParticle>)>,
    mut particles: Query<(&mut Transform, &Drift), With<WaterParticle>>,
) {
    let dt = time.delta_secs();
    let player_pos = players.iter().next().map(|t| t.translation.truncate());
    for (mut t, drift) in &mut particles {
        // Base drift — keeps the field alive even when the player is still.
        t.translation.x += drift.0.x * dt;
        t.translation.y += drift.0.y * dt;

        // Repel from player when close — the cell pushes the water aside.
        if let Some(p) = player_pos {
            let dx = t.translation.x - p.x;
            let dy = t.translation.y - p.y;
            let dist_sq = dx * dx + dy * dy;
            let near = 90.0;
            if dist_sq < near * near && dist_sq > 0.001 {
                let dist = dist_sq.sqrt();
                let strength = (near - dist) / near * 80.0;
                let nx = dx / dist;
                let ny = dy / dist;
                t.translation.x += nx * strength * dt;
                t.translation.y += ny * strength * dt;
            }
        }
    }
}

// Bouncy organic shape — the cell's x and y scale oscillate out of
// phase so it never looks like a perfect rigid circle. Mild amplitude
// so it reads as "alive," not "broken."
fn wobble_player_cell(
    time: Res<Time>,
    mut cells: Query<&mut Transform, With<PlayerCell>>,
) {
    let t = time.elapsed_secs();
    let sx = 1.0 + (t * 3.5).sin() * 0.09;
    let sy = 1.0 + (t * 3.5 + std::f32::consts::FRAC_PI_3).sin() * 0.09;
    for mut tr in &mut cells {
        tr.scale.x = sx;
        tr.scale.y = sy;
    }
}

fn eat_algae(
    mut commands: Commands,
    players: Query<(&Transform, &CellRadius), With<PlayerCell>>,
    algae: Query<(Entity, &Transform, &CellRadius), With<Algae>>,
) {
    for (player_t, player_r) in &players {
        for (algae_e, algae_t, algae_r) in &algae {
            let dx = player_t.translation.x - algae_t.translation.x;
            let dy = player_t.translation.y - algae_t.translation.y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < player_r.0 + algae_r.0 {
                commands.entity(algae_e).despawn();
            }
        }
    }
}

// Runs as a Startup system, AFTER LogPlugin::build has installed its
// own panic hook. Wrapping the previous hook keeps LogPlugin's console
// output AND adds our in-canvas capture. Pre-App install gets
// silently overwritten by LogPlugin on wasm — see
// https://github.com/bevyengine/bevy/issues/12546.
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut q = PANIC_QUEUE.lock().unwrap_or_else(|p| p.into_inner());
        q.push(format!("{info}"));
        prev(info);
    }));
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

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

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
        Text::new("universe · dev"),
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
