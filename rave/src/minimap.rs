//! Minimap — top-right UI panel showing the player's position relative
//! to every named `map::Pin`. Player stays dead-centre; pins slide
//! around them as the player walks.
//!
//! Zoom: half the world in view — the minimap's ±100 px span covers
//! ±`room::FLOOR_HALF / 2` (1500) world units on each axis. Close
//! enough that nearby pins land with useful spatial resolution;
//! anything past 1500 units away falls off the map for the frame
//! and returns as the player approaches. Runtime zoom toggle is a
//! small follow-up — `SCALE` is the only thing that has to change.
//!
//! Coordinate handedness: rave's world uses +Z south (see
//! `room::move_player`), so on this north-up minimap, positive
//! world Z maps to positive minimap Y (down).
//!
//! Independent of the 3D pin overlay: pressing `M` hides the world
//! rods + labels but the minimap stays. Different affordance,
//! different toggle (which isn't wired yet — MVP is always-visible).

use bevy::prelude::*;

use crate::map::Pin;
use crate::room::{FLOOR_HALF, PlayerCell};

/// Minimap edge length in screen pixels.
const MINIMAP_SIZE_PX: f32 = 200.0;

/// Marker dot size for the player + pins, in screen pixels.
const MARKER_SIZE_PX: f32 = 8.0;

/// world → minimap scale. `MINIMAP_SIZE_PX / FLOOR_HALF` puts half
/// the walkable world in view — ±1500 world units span the ±100 px
/// minimap radius.
const SCALE: f32 = MINIMAP_SIZE_PX / FLOOR_HALF;

/// Root of the minimap UI subtree — used only for scene structure.
#[derive(Component)]
pub struct MinimapRoot;

/// Marker for the player dot at the minimap centre.
#[derive(Component)]
pub struct MinimapPlayer;

/// Marker for each pin dot, carrying the world-space pin entity so
/// `update_minimap` can look up its `GlobalTransform` per frame.
#[derive(Component)]
pub struct MinimapPin {
    pub world_pin: Entity,
}

/// Marker on the ≡ drawer-toggle button inside the minimap header.
/// The minimap is the container for HUD interaction affordances; the
/// hamburger sits in its top-right so mobile clients (no keyboard)
/// can open the diagnostic drawer without a separate DOM button
/// competing for screen real estate.
#[derive(Component)]
pub struct MinimapToggleButton;

/// Startup — spawn the minimap root in the top-right corner, plus a
/// green player dot at centre and one yellow pin dot per world
/// `Pin`. Ordered `.after(map::setup_map)` in `lib.rs` so the pin
/// entities exist before this queries them.
pub fn setup_minimap(mut commands: Commands, pins: Query<Entity, With<Pin>>) {
    commands
        .spawn((
            MinimapRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(6.0),
                right: Val::Px(6.0),
                width: Val::Px(MINIMAP_SIZE_PX),
                height: Val::Px(MINIMAP_SIZE_PX),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.15)),
        ))
        .with_children(|parent| {
            // Player dot — dead centre, dead reckoning. Never moves.
            let centre = (MINIMAP_SIZE_PX - MARKER_SIZE_PX) / 2.0;
            parent.spawn((
                MinimapPlayer,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(centre),
                    top: Val::Px(centre),
                    width: Val::Px(MARKER_SIZE_PX),
                    height: Val::Px(MARKER_SIZE_PX),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.35, 0.85, 0.55)),
            ));

            // One dot per pin. Positions are computed each frame by
            // `update_minimap` — spawned here at (0, 0) as a
            // placeholder.
            for pin_entity in &pins {
                parent.spawn((
                    MinimapPin {
                        world_pin: pin_entity,
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Px(MARKER_SIZE_PX),
                        height: Val::Px(MARKER_SIZE_PX),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(1.0, 0.9, 0.2)),
                    Visibility::Visible,
                ));
            }

            // Drawer toggle (≡) inside the minimap's bottom-right —
            // the `#bevy-error` red overlay at the top of the page
            // can be tall enough to cover the top of the minimap,
            // so the button lives below any error state. Icon is
            // three horizontal `Node` bars stacked vertically —
            // font-independent so we don't depend on the default
            // font shipping U+2261. Handled by
            // `handle_minimap_toggle_button`.
            parent
                .spawn((
                    Button,
                    MinimapToggleButton,
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(4.0),
                        bottom: Val::Px(4.0),
                        width: Val::Px(24.0),
                        height: Val::Px(24.0),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::SpaceEvenly,
                        align_items: AlignItems::Center,
                        padding: UiRect::vertical(Val::Px(5.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
                ))
                .with_children(|btn| {
                    for _ in 0..3 {
                        btn.spawn((
                            Node {
                                width: Val::Px(14.0),
                                height: Val::Px(2.0),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.9, 0.9, 0.9)),
                        ));
                    }
                });
        });
}

/// Update — watch the minimap's ≡ button for `Interaction::Pressed`
/// and forward to `drawer::request_drawer_toggle`, which sets the
/// same thread-local flag the JS `rave_drawer_toggle` extern used
/// to. `toggle_log_drawer` consumes it next tick.
pub fn handle_minimap_toggle_button(
    interactions: Query<&Interaction, (Changed<Interaction>, With<MinimapToggleButton>)>,
) {
    for interaction in &interactions {
        if matches!(interaction, Interaction::Pressed) {
            crate::drawer::request_drawer_toggle();
        }
    }
}

/// Update — project each pin's world position into minimap space
/// relative to the player's current world position, then set each
/// pin dot's absolute `Node` position. Dots that fall outside the
/// minimap bounds get their `Visibility` toggled to `Hidden` for
/// the frame so they don't render at (0, 0) or clip the border.
pub fn update_minimap(
    players: Query<&Transform, With<PlayerCell>>,
    pin_transforms: Query<&GlobalTransform, With<Pin>>,
    mut pin_dots: Query<(&MinimapPin, &mut Node, &mut Visibility)>,
) {
    let Some(player_tf) = players.iter().next() else {
        return;
    };
    let player_x = player_tf.translation.x;
    let player_z = player_tf.translation.z;
    let centre = MINIMAP_SIZE_PX / 2.0;
    let half_marker = MARKER_SIZE_PX / 2.0;

    for (dot, mut node, mut vis) in &mut pin_dots {
        let Ok(pin_tf) = pin_transforms.get(dot.world_pin) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let dx = (pin_tf.translation().x - player_x) * SCALE;
        let dz = (pin_tf.translation().z - player_z) * SCALE;
        let left = centre + dx - half_marker;
        let top = centre + dz - half_marker;

        let bound = 0.0..=MINIMAP_SIZE_PX - MARKER_SIZE_PX;
        if !bound.contains(&left) || !bound.contains(&top) {
            *vis = Visibility::Hidden;
            continue;
        }
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        *vis = Visibility::Visible;
    }
}
