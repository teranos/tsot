// Sphere-vs-AABB collision for a single player against a set of static
// obstacles. Ported from rave/src/physics.rs — same algorithm, same
// component shape, minus Bevy Transform (which requires TransformPlugin
// to propagate GlobalTransform). We use a plain Position(Vec3) instead;
// the collision logic works the same way.
//
// This is the first real rave module to land in seer. Every allocation
// its systems trigger now flows through the obs bus with a Rust source
// path; the ECS pattern is exercised by the Bevy schedule; the module
// is a real gameplay component, not a synthetic workload.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

pub const PLAYER_RADIUS: f32 = 20.0;

#[derive(Component, Clone, Copy, Debug)]
pub struct Position(pub Vec3);

#[derive(Component, Clone, Copy, Debug)]
pub struct Velocity(pub Vec3);

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

#[derive(Component)]
pub struct PlayerMarker;

pub fn advance_player(mut q: Query<(&mut Position, &Velocity), With<PlayerMarker>>) {
    for (mut p, v) in q.iter_mut() {
        p.0 += v.0;
    }
}

pub fn resolve_collisions(
    mut player_q: Query<(&mut Position, &mut Velocity), With<PlayerMarker>>,
    obstacles: Query<(&Position, &AabbCollider), Without<PlayerMarker>>,
) {
    let Some((mut p_pos, mut p_vel)) = player_q.iter_mut().next() else {
        return;
    };

    for (obs_pos, collider) in obstacles.iter() {
        let aabb_min = obs_pos.0 - collider.half_extents;
        let aabb_max = obs_pos.0 + collider.half_extents;
        let centre = p_pos.0;
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
            p_pos.0 += normal * overlap;
            let v_along = p_vel.0.dot(normal);
            if v_along < 0.0 {
                p_vel.0 -= normal * v_along;
            }
        }
    }
}
