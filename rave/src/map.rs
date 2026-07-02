//! Map — named world pins the scene will hang from.
//!
//! Scene layout in rave was smeared across `room.rs`, `floorplan.rs`,
//! `trees.rs`, `trail.rs` — every fixture's position was arithmetic on
//! someone else's constant, so adding anything required reconciling
//! four files by hand. `map.rs` is the single source of truth for
//! "where in the world": one `Pin` enum + one `pub const Vec3` per
//! named zone the scene has.
//!
//! Shape at this landing: pins are spawned as anchor entities with
//! world-space transforms, each rendered as an emissive rod topped
//! with a sphere plus a UI text label at its screen-projected
//! position. `M` toggles the whole overlay on/off. Nothing is
//! parented to the anchors yet — the existing modules still
//! `Transform::from_xyz` their fixtures at absolute world coords.
//! Migration is one zone per subsequent commit: reparent that
//! zone's fixtures as children of its pin + rewrite their local
//! transforms.

use bevy::prelude::*;

/// Discriminator on every named zone-anchor entity. Attached to a
/// `Transform`-carrying entity spawned at the pin's world position;
/// the pin's contents live as children and inherit its transform.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Consumers land per-zone in follow-up commits.
pub enum Pin {
    Stage,
    Dancefloor,
    BarZone,
    Trail,
    Campfire,
}

/// Stage — north edge of the clearing. Anchors DJ booth, speakers,
/// back lights, truss + moving spots (currently spawned by
/// `floorplan::setup_floor_plan` with absolute coords).
pub const STAGE: Vec3 = Vec3::new(0.0, 0.0, -500.0);

/// Dancefloor — clearing origin. Anchors the metallic square + 4
/// corner strobes (`floorplan.rs`).
pub const DANCEFLOOR: Vec3 = Vec3::new(0.0, 0.0, 0.0);

/// Bar zone — west side of the clearing. Anchors the bar box +
/// magenta/blue bar lights (`floorplan.rs`).
pub const BAR_ZONE: Vec3 = Vec3::new(-460.0, 0.0, 0.0);

/// Trail — south corridor from the clearing edge toward the player
/// spawn. Anchors the trail plane + any LED strips lining it
/// (`trail.rs`).
pub const TRAIL: Vec3 = Vec3::new(0.0, 0.0, 1470.0);

/// Campfire — just south of the dancefloor edge, inside the
/// tree-exclusion buffer. Not yet reified as scene content; the
/// anchor entity exists so future fixtures can attach under it
/// without another cross-module coordinate hunt.
pub const CAMPFIRE: Vec3 = Vec3::new(-800.0, 0.0, 0.0);

/// Debug overlay state. `M` flips this; the flip cascades
/// `Visibility` to every pin anchor + UI label.
#[derive(Resource)]
pub struct PinOverlayVisible(pub bool);

impl Default for PinOverlayVisible {
    fn default() -> Self {
        // Overlay ships on so the pins are visible the first time
        // the developer loads the scene after this lands. Toggle off
        // with `M` when you want the un-annotated view back.
        Self(true)
    }
}

/// Marker on the UI text label entities, carrying the anchor entity
/// so the per-frame projection system can find that anchor's
/// GlobalTransform to project onto the screen.
#[derive(Component)]
pub struct PinLabel {
    pub anchor: Entity,
}

/// Marker on the debug mesh children of a pin anchor (the yellow
/// rod + head sphere). Toggled independently of the anchor entity
/// so `M` hides the debug overlay without cascading through the
/// transform hierarchy to hide scene content (campfire logs, flame,
/// etc. — see the `2026-07-02` bug where `M` made the campfire
/// disappear alongside the pin).
#[derive(Component)]
pub struct PinMarker;

/// Startup — for each pin: spawn the anchor entity with its
/// Transform + `Visibility::Visible` + a two-child mesh (rod +
/// spherical head, both emissive yellow), and spawn a matching UI
/// text label carrying the anchor's entity id so
/// `update_pin_labels` can project the anchor's world position to
/// screen each frame.
pub fn setup_map(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Shared meshes + material across all pins — five pins, one
    // Cylinder alloc, one Sphere alloc, one StandardMaterial alloc.
    let rod_mesh = meshes.add(Cylinder::new(2.0, 60.0));
    let head_mesh = meshes.add(Sphere::new(8.0));
    let pin_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.2),
        emissive: LinearRgba::new(2.4, 2.0, 0.5, 1.0),
        ..default()
    });

    for (pin, pos, name) in [
        (Pin::Stage, STAGE, "Stage"),
        (Pin::Dancefloor, DANCEFLOOR, "Dancefloor"),
        (Pin::BarZone, BAR_ZONE, "BarZone"),
        (Pin::Trail, TRAIL, "Trail"),
        (Pin::Campfire, CAMPFIRE, "Campfire"),
    ] {
        let anchor = commands
            .spawn((
                pin,
                Name::new(name),
                Transform::from_translation(pos),
                Visibility::Visible,
            ))
            .with_children(|parent| {
                // Rod — Cylinder centred at y=30 puts its base on
                // the ground and its top at y=60.
                parent.spawn((
                    PinMarker,
                    Mesh3d(rod_mesh.clone()),
                    MeshMaterial3d(pin_mat.clone()),
                    Transform::from_xyz(0.0, 30.0, 0.0),
                ));
                // Head — Sphere centred at y=68 sits just above the
                // rod's top so the pin reads as rod-plus-ball.
                parent.spawn((
                    PinMarker,
                    Mesh3d(head_mesh.clone()),
                    MeshMaterial3d(pin_mat.clone()),
                    Transform::from_xyz(0.0, 68.0, 0.0),
                ));
            })
            .id();

        // UI text label — absolute-positioned Node; each frame the
        // label's Node.left/top are set from the anchor's screen
        // projection by `update_pin_labels`.
        commands.spawn((
            Text::new(name),
            PinLabel { anchor },
            TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.9, 0.4)),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Visibility::Visible,
        ));
    }
}

/// Update — while the overlay is on: project each pin anchor's
/// world position to screen coords via `Camera::world_to_viewport`
/// and set the label's absolute UI position. Anchors behind the
/// camera (projection errors) get their label hidden for that
/// frame. While the overlay is off: early-return so
/// `toggle_pin_overlay`'s hide sticks.
pub fn update_pin_labels(
    state: Res<PinOverlayVisible>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    anchors: Query<&GlobalTransform, (With<Pin>, Without<PinLabel>)>,
    mut labels: Query<(&PinLabel, &mut Node, &mut Visibility)>,
) {
    if !state.0 {
        return;
    }
    let Ok((camera, cam_tf)) = cameras.single() else {
        return;
    };
    for (label, mut node, mut vis) in &mut labels {
        let Ok(anchor_tf) = anchors.get(label.anchor) else {
            continue;
        };
        match camera.world_to_viewport(cam_tf, anchor_tf.translation()) {
            Ok(screen) => {
                node.left = Val::Px(screen.x);
                node.top = Val::Px(screen.y);
                *vis = Visibility::Visible;
            }
            Err(_) => {
                *vis = Visibility::Hidden;
            }
        }
    }
}

/// Update — `M` flips the overlay. Cascades the new `Visibility`
/// to the debug marker meshes (`PinMarker`) and UI labels
/// (`PinLabel`) only — never to the `Pin` anchor entity, so its
/// scene-content children (campfire, future zone contents) stay
/// visible regardless of the overlay's state. Ignores `M` while
/// chat is focused so typing doesn't toggle the overlay by accident.
pub fn toggle_pin_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<PinOverlayVisible>,
    mut marker_vis: Query<&mut Visibility, (With<PinMarker>, Without<PinLabel>)>,
    mut label_vis: Query<&mut Visibility, (With<PinLabel>, Without<PinMarker>)>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    if crate::chat::is_chat_focused() {
        return;
    }
    state.0 = !state.0;
    let new_vis = if state.0 {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut v in &mut marker_vis {
        *v = new_vis;
    }
    for mut v in &mut label_vis {
        *v = new_vis;
    }
}
