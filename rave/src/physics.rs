//! Collision — sphere-vs-AABB for the player against the clearing's
//! solid props (DJ booth, speakers, bar), and sphere-vs-sphere for the
//! player against other peers (RemotePlayerCell).
//!
//! No external physics crate. The clearing has a handful of axis-
//! aligned cuboids and a known list of peer spheres; the per-frame
//! cost is O(static_colliders + remote_peers), which at human scale
//! is negligible. Trees aren't checked (hundreds of them; if you walk
//! into one you walk through it for now — a spatial-hash check
//! comes later if it matters).
//!
//! Runs AFTER `room::move_player` (which advances position from
//! velocity) and BEFORE `room::camera_follow` (so the camera reads
//! the post-resolve position).

use bevy::prelude::*;

#[cfg(target_arch = "wasm32")]
use crate::net_glue::RemotePlayerCell;
use crate::room::{PlayerCell, Velocity};

/// Local player sphere radius — matches the mesh spawn radius in
/// `room::setup_room`. Hard-coded constant so the collision system
/// doesn't need to query mesh extents at runtime.
pub const PLAYER_RADIUS: f32 = 20.0;

/// Remote peer sphere radius — matches `net_glue::render_remote_players`'s
/// `Sphere::new(20.0)`. If the wire ever carries per-peer sizes the
/// remote constant becomes a per-entity component.
pub const REMOTE_RADIUS: f32 = 20.0;

/// Axis-aligned bounding box collider. Half-extents along each axis,
/// centred on the entity's transform translation. Static — no broad
/// phase, no spatial hash; the clearing only has ~5 of these.
#[derive(Component, Clone, Copy)]
pub struct AabbCollider {
    pub half_extents: Vec3,
}

impl AabbCollider {
    pub fn cuboid(size: Vec3) -> Self {
        Self {
            half_extents: size * 0.5,
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn resolve_collisions(
    mut player_q: Query<(&mut Transform, &mut Velocity), With<PlayerCell>>,
    obstacles: Query<
        (&Transform, &AabbCollider),
        (Without<PlayerCell>, Without<RemotePlayerCell>),
    >,
    remotes: Query<&Transform, (With<RemotePlayerCell>, Without<PlayerCell>)>,
) {
    let Some((mut p_tf, mut p_vel)) = player_q.iter_mut().next() else {
        return;
    };

    // Sphere vs each obstacle's AABB.
    for (obs_tf, collider) in obstacles.iter() {
        let aabb_min = obs_tf.translation - collider.half_extents;
        let aabb_max = obs_tf.translation + collider.half_extents;
        let centre = p_tf.translation;
        let closest = Vec3::new(
            centre.x.clamp(aabb_min.x, aabb_max.x),
            centre.y.clamp(aabb_min.y, aabb_max.y),
            centre.z.clamp(aabb_min.z, aabb_max.z),
        );
        let delta = centre - closest;
        let dist_sq = delta.length_squared();
        if dist_sq < PLAYER_RADIUS * PLAYER_RADIUS && dist_sq > 1.0e-6 {
            let dist = dist_sq.sqrt();
            let normal = delta / dist;
            let overlap = PLAYER_RADIUS - dist;
            p_tf.translation += normal * overlap;
            // Zero the inward velocity component so we don't keep
            // accelerating into the obstacle; outward motion is
            // unaffected.
            let v_along = p_vel.0.dot(normal);
            if v_along < 0.0 {
                p_vel.0 -= normal * v_along;
            }
        }
    }

    // Sphere vs sphere for each remote peer. Only the local player
    // moves — remotes own their own position via the wire.
    let min_dist = PLAYER_RADIUS + REMOTE_RADIUS;
    let min_dist_sq = min_dist * min_dist;
    for r_tf in remotes.iter() {
        let delta = p_tf.translation - r_tf.translation;
        let dist_sq = delta.length_squared();
        if dist_sq < min_dist_sq && dist_sq > 1.0e-6 {
            let dist = dist_sq.sqrt();
            let normal = delta / dist;
            let overlap = min_dist - dist;
            p_tf.translation += normal * overlap;
            let v_along = p_vel.0.dot(normal);
            if v_along < 0.0 {
                p_vel.0 -= normal * v_along;
            }
        }
    }
}

/// Native variant — no `RemotePlayerCell` (net_glue is wasm32-only),
/// so we only check static obstacles. The clippy `dead_code` lint
/// would fire because `PLAYER_RADIUS` + obstacles iteration on native
/// is otherwise unused — this keeps the symbols live.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_collisions(
    mut player_q: Query<(&mut Transform, &mut Velocity), With<PlayerCell>>,
    obstacles: Query<(&Transform, &AabbCollider), Without<PlayerCell>>,
) {
    let Some((mut p_tf, mut p_vel)) = player_q.iter_mut().next() else {
        return;
    };
    for (obs_tf, collider) in obstacles.iter() {
        let aabb_min = obs_tf.translation - collider.half_extents;
        let aabb_max = obs_tf.translation + collider.half_extents;
        let centre = p_tf.translation;
        let closest = Vec3::new(
            centre.x.clamp(aabb_min.x, aabb_max.x),
            centre.y.clamp(aabb_min.y, aabb_max.y),
            centre.z.clamp(aabb_min.z, aabb_max.z),
        );
        let delta = centre - closest;
        let dist_sq = delta.length_squared();
        if dist_sq < PLAYER_RADIUS * PLAYER_RADIUS && dist_sq > 1.0e-6 {
            let dist = dist_sq.sqrt();
            let normal = delta / dist;
            let overlap = PLAYER_RADIUS - dist;
            p_tf.translation += normal * overlap;
            let v_along = p_vel.0.dot(normal);
            if v_along < 0.0 {
                p_vel.0 -= normal * v_along;
            }
        }
    }
}
