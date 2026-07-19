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

#[derive(Component)]
pub struct NpcMarker;

pub fn advance_player(mut q: Query<(&mut Position, &Velocity), With<PlayerMarker>>) {
    for (mut p, v) in q.iter_mut() {
        p.0 += v.0;
    }
}

pub fn advance_npc(mut q: Query<(&mut Position, &Velocity), With<NpcMarker>>) {
    for (mut p, v) in q.iter_mut() {
        p.0 += v.0;
    }
}

/// Set an actor's `Position.y` to the ground height under it, so the
/// SIMULATION carries real elevation (camera, proximity, persistence),
/// not only the render-time drape.
fn sit_on_terrain(p: &mut Position) {
    p.0.y = crate::terrain::height(p.0.x, p.0.z);
}

/// Sit the player on the terrain, after movement + collision settle.
/// Scoped to the player; static colliders (trees, walls, props) keep their
/// authored `y` (walls carry roof height there), so slope-aware collision
/// for those is a separate step.
pub fn ground_follow_player(mut q: Query<&mut Position, With<PlayerMarker>>) {
    for mut p in q.iter_mut() {
        sit_on_terrain(&mut p);
    }
}

/// Sit NPCs on the terrain. See `ground_follow_player`.
pub fn ground_follow_npc(mut q: Query<&mut Position, With<NpcMarker>>) {
    for mut p in q.iter_mut() {
        sit_on_terrain(&mut p);
    }
}

/// Distance at which player is considered bumping the NPC.
/// (player radius) + (NPC visual half-width) ≈ 20 + 35.
pub const BUMP_DISTANCE: f32 = 55.0;

pub fn check_npc_bump(
    player_q: Query<&Position, With<PlayerMarker>>,
    npc_q: Query<&Position, With<NpcMarker>>,
    mut bang: ResMut<crate::bang::Bang>,
) {
    let Some(player_pos) = player_q.iter().next() else {
        return;
    };
    for npc_pos in npc_q.iter() {
        if (player_pos.0 - npc_pos.0).length() < BUMP_DISTANCE {
            // Anchor the bang above the NPC cube — cube is 140 tall
            // centred at Y=60, so top ≈ 130. Reconstruct the same
            // follow camera render_web uses, project to clip space
            // (== NDC for the ortho camera).
            let bang_world = [npc_pos.0.x, 150.0, npc_pos.0.z];
            let camera = crate::scene::SceneCamera::follow(
                [player_pos.0.x, player_pos.0.y, player_pos.0.z],
                crate::room::FLOOR_HALF,
            );
            let clip = camera.world_to_clip(bang_world);
            crate::bang::trigger(&mut bang, clip);
            crate::audio::play_alert();
            return;
        }
    }
}

/// Player speed magnitude — units per tick. Chosen so the 180-tick
/// CI run covers a visible fraction of the world (~500 units of
/// path length) without leaving the follow-cam's zoom radius.
pub const WANDER_SPEED: f32 = 3.0;
/// Radians per tick the wander direction rotates. Small positive
/// value: the player traces a smooth curve, not a straight line, so
/// the follow-cam has motion to react to and cross-commit drift in
/// the player system produces a visibly different trajectory.
pub const WANDER_TURN_RATE: f32 = 0.02;

/// Input surrogate — until seer wires a real keyboard/touch source
/// (headless-CI has neither), the player's velocity direction rotates
/// deterministically at WANDER_TURN_RATE per tick with fixed
/// WANDER_SPEED magnitude. Same commit → same trajectory.
///
/// `Local<u32>` carries the per-system tick counter so we don't need
/// a global FrameCount resource; ordering guarantees this runs before
/// advance_player, so the freshly-rotated velocity is what advance
/// integrates.
pub fn wander_input(
    mut q: Query<&mut Velocity, With<PlayerMarker>>,
    mut tick: Local<u32>,
) {
    *tick += 1;
    let angle = (*tick as f32) * WANDER_TURN_RATE;
    let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
    for mut v in q.iter_mut() {
        v.0 = dir * WANDER_SPEED;
    }
}

/// Same wander pattern as the native player input, but for NPCs. Runs
/// on both native and wasm so an NPC keeps circling in the browser
/// while the player uses WASD.
pub fn wander_npc(
    mut q: Query<&mut Velocity, With<NpcMarker>>,
    mut tick: Local<u32>,
) {
    *tick += 1;
    let angle = (*tick as f32) * WANDER_TURN_RATE;
    let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
    for mut v in q.iter_mut() {
        v.0 = dir * WANDER_SPEED;
    }
}

/// Player speed under keyboard input. Faster than the AI wander so
/// crossing the room takes seconds, not tens of seconds.
pub const KEYBOARD_SPEED: f32 = 18.0;

/// WASD → velocity, rotated 45° around Y to align with the isometric
/// camera. W is "up on screen" ≈ world (-X, 0, -Z); S is "down on
/// screen"; A/D mirror. Diagonals normalise so two-key combinations
/// don't move faster than single-key.
pub fn keyboard_input(mut q: Query<&mut Velocity, With<PlayerMarker>>) {
    let s = crate::input::state();
    let mut dir = Vec3::ZERO;
    if s & crate::input::key::W != 0 {
        dir += Vec3::new(-1.0, 0.0, -1.0);
    }
    if s & crate::input::key::S != 0 {
        dir += Vec3::new(1.0, 0.0, 1.0);
    }
    if s & crate::input::key::A != 0 {
        dir += Vec3::new(-1.0, 0.0, 1.0);
    }
    if s & crate::input::key::D != 0 {
        dir += Vec3::new(1.0, 0.0, -1.0);
    }
    let vel = if dir.length_squared() > 0.0 {
        dir.normalize() * KEYBOARD_SPEED
    } else {
        Vec3::ZERO
    };
    for mut v in q.iter_mut() {
        v.0 = vel;
    }
}

/// Player half-width added to the remote peer's visual half-width
/// (peer cubes are ~70 wide → half-width ≈ 35, same as NPC bump distance).
pub const REMOTE_PEER_BLOCK_DIST: f32 = 55.0;

/// Local player vs remote peers — sphere-sphere collision on the XZ plane
/// (vertical differences don't block). Push local player out of overlap
/// with any remote peer; cancel inbound velocity along the contact normal
/// so movement stops instead of teleport-bouncing. Same shape as
/// `resolve_collisions` but reads the RemotePlayers resource instead of
/// an ECS obstacle query.
pub fn resolve_remote_player_collisions(
    mut player_q: Query<(&mut Position, &mut Velocity), With<PlayerMarker>>,
    remotes: Res<crate::remote_players::RemotePlayers>,
) {
    let Some((mut p_pos, mut p_vel)) = player_q.iter_mut().next() else {
        return;
    };
    let min_dist_sq = REMOTE_PEER_BLOCK_DIST * REMOTE_PEER_BLOCK_DIST;
    for entry in remotes.0.values() {
        let dx = p_pos.0.x - entry.pos.x;
        let dz = p_pos.0.z - entry.pos.z;
        let dist_sq = dx * dx + dz * dz;
        if dist_sq < min_dist_sq && dist_sq > 1.0e-6 {
            let dist = dist_sq.sqrt();
            let nx = dx / dist;
            let nz = dz / dist;
            let overlap = REMOTE_PEER_BLOCK_DIST - dist;
            p_pos.0.x += nx * overlap;
            p_pos.0.z += nz * overlap;
            let v_along = p_vel.0.x * nx + p_vel.0.z * nz;
            if v_along < 0.0 {
                p_vel.0.x -= nx * v_along;
                p_vel.0.z -= nz * v_along;
            }
            crate::audio::play_pock();
        }
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
        // XZ (ground-plane) collision. Every actor and collider sits on the
        // terrain surface, so the vertical gap between them is just "how
        // high the ground is under each" — it must NOT gate blocking, or a
        // player standing on a hill walks through walls pinned at y=0.
        // (Matches resolve_remote_player_collisions, already XZ-only.)
        let closest_x = centre.x.clamp(aabb_min.x, aabb_max.x);
        let closest_z = centre.z.clamp(aabb_min.z, aabb_max.z);
        let (dx, dz) = (centre.x - closest_x, centre.z - closest_z);
        let dist_sq = dx * dx + dz * dz;
        if dist_sq < PLAYER_RADIUS * PLAYER_RADIUS && dist_sq > 1.0e-6 {
            let dist = dist_sq.sqrt();
            let (nx, nz) = (dx / dist, dz / dist);
            let overlap = PLAYER_RADIUS - dist;
            p_pos.0.x += nx * overlap;
            p_pos.0.z += nz * overlap;
            let v_along = p_vel.0.x * nx + p_vel.0.z * nz;
            if v_along < 0.0 {
                p_vel.0.x -= nx * v_along;
                p_vel.0.z -= nz * v_along;
            }
            crate::audio::play_thump();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_blocks_in_xz_even_when_the_player_is_high_on_terrain() {
        use bevy_ecs::prelude::*;
        let mut world = World::new();
        // Player just outside the obstacle's +X face in XZ, but far above
        // it in Y (as ground_follow puts it on a hill). It must still be
        // blocked — collision is on the ground plane, not in 3D.
        let player = world
            .spawn((
                PlayerMarker,
                Position(Vec3::new(45.0, 300.0, 0.0)),
                Velocity(Vec3::ZERO),
            ))
            .id();
        world.spawn((
            Position(Vec3::new(0.0, 0.0, 0.0)),
            AabbCollider::cuboid(Vec3::new(60.0, 40.0, 60.0)), // half (30, 20, 30)
        ));
        let mut sched = Schedule::default();
        sched.add_systems(resolve_collisions);
        sched.run(&mut world);

        let p = world.get::<Position>(player).unwrap().0;
        // Pushed out along +X to the face (30) + radius (20) = 50.
        assert!(p.x >= 49.5, "player walked through the obstacle: x={}", p.x);
        // Collision leaves Y alone — that's ground_follow's job.
        assert!((p.y - 300.0).abs() < 1e-3, "collision moved Y: {}", p.y);
    }

    #[test]
    fn ground_follow_puts_actors_on_the_terrain() {
        use bevy_ecs::prelude::*;
        let mut world = World::new();
        let player = world
            .spawn((PlayerMarker, Position(Vec3::new(500.0, 20.0, -1200.0))))
            .id();
        // On the school pad → the flat pad height.
        let npc = world
            .spawn((NpcMarker, Position(Vec3::new(10_800.0, 20.0, 44_400.0))))
            .id();
        let mut sched = Schedule::default();
        sched.add_systems((ground_follow_player, ground_follow_npc));
        sched.run(&mut world);

        let p = world.get::<Position>(player).unwrap().0;
        let ph = crate::terrain::height(500.0, -1200.0);
        assert!((p.y - ph).abs() < 1e-3, "player off the terrain: {} vs {ph}", p.y);
        let n = world.get::<Position>(npc).unwrap().0;
        let nh = crate::terrain::height(10_800.0, 44_400.0);
        assert!((n.y - nh).abs() < 1e-3, "npc off the pad height: {} vs {nh}", n.y);
    }

    #[test]
    fn wander_direction_is_unit_scaled_by_speed() {
        // The direction vector at any tick has magnitude ==
        // WANDER_SPEED (within f32 noise). Not a Bevy-driven test —
        // just verifies the math the system embeds.
        for tick in 0..500 {
            let angle = tick as f32 * WANDER_TURN_RATE;
            let v = Vec3::new(angle.cos(), 0.0, angle.sin()) * WANDER_SPEED;
            let mag = v.length();
            assert!(
                (mag - WANDER_SPEED).abs() < 1e-4,
                "tick {tick}: |v| = {mag}, expected {WANDER_SPEED}"
            );
        }
    }
}
