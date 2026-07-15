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

/// Tags an obstacle as a fence — a short barrier the player can HOP
/// OVER by sustained pushing into it. Walls without this marker block
/// permanently.
#[derive(Component)]
pub struct FenceMarker;

/// Ticks of continuous inbound push against a single fence before the
/// player hops over it. At 60 FPS this is ~0.5s — long enough to be
/// deliberate, short enough not to feel stuck.
pub const FENCE_HOP_TICKS: u32 = 30;

/// Per-fence-entity push counter. Reset when the player stops pushing
/// against a fence (velocity into it drops to ~0 or they leave the
/// contact). Lives in a system-local `HashMap` — no cross-frame
/// resource needed; the Bevy Local storage is per-system.
pub type FenceHopState = std::collections::HashMap<Entity, u32>;

/// Where does the player land after hopping over a fence? Pure function
/// of the current positions + fence extents: flip the player across the
/// fence's thin axis (its normal) so they end up on the far side, clear
/// of the fence collider plus the player's own radius.
pub fn hopped_position(
    player: Vec3,
    fence_pos: Vec3,
    fence_half: Vec3,
    player_radius: f32,
) -> Vec3 {
    // Which axis is the fence thin on? That's the axis normal to the
    // barrier; we hop across it. Break ties toward Z (arbitrary — a
    // square-cross-section Fence isolated post won't come up often).
    let (axis_idx, thin_half) = if fence_half.x <= fence_half.z {
        (0usize, fence_half.x)
    } else {
        (2usize, fence_half.z)
    };
    let mut out = player;
    let (p_axis, f_axis) = if axis_idx == 0 {
        (player.x, fence_pos.x)
    } else {
        (player.z, fence_pos.z)
    };
    // Sign of the far side: opposite of where the player is now.
    let sign = if p_axis < f_axis { 1.0 } else { -1.0 };
    let far = f_axis + sign * (thin_half + player_radius + 2.0);
    if axis_idx == 0 {
        out.x = far;
    } else {
        out.z = far;
    }
    out
}

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
    obstacles: Query<(Entity, &Position, &AabbCollider, Option<&FenceMarker>), Without<PlayerMarker>>,
    mut hop_state: Local<FenceHopState>,
) {
    let Some((mut p_pos, mut p_vel)) = player_q.iter_mut().next() else {
        return;
    };

    // Track which fences the player is actively pushing into THIS frame.
    // Fences absent from the set have their counter reset (player
    // walked away — hop timer starts fresh next contact).
    let mut still_pushing: std::collections::HashSet<Entity> =
        std::collections::HashSet::new();

    for (entity, obs_pos, collider, fence) in obstacles.iter() {
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
            let v_along = p_vel.0.dot(normal);
            if fence.is_some() && v_along < 0.0 {
                // Pushing INTO the fence — increment hop timer.
                let ticks = hop_state.entry(entity).or_insert(0);
                *ticks += 1;
                still_pushing.insert(entity);
                if *ticks >= FENCE_HOP_TICKS {
                    p_pos.0 = hopped_position(
                        p_pos.0,
                        obs_pos.0,
                        collider.half_extents,
                        PLAYER_RADIUS,
                    );
                    // Preserve velocity through the hop — the player
                    // keeps moving in the direction they were pushing.
                    hop_state.remove(&entity);
                    still_pushing.remove(&entity);
                    continue;
                }
            }
            // Standard block: push out + cancel inbound velocity.
            let overlap = PLAYER_RADIUS - dist;
            p_pos.0 += normal * overlap;
            if v_along < 0.0 {
                p_vel.0 -= normal * v_along;
            }
            crate::audio::play_thump();
        }
    }

    // Any fence the player stopped pushing on this frame — reset.
    hop_state.retain(|e, _| still_pushing.contains(e));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hopped_position_lands_the_player_on_the_far_side_of_the_fence() {
        // FenceEW (thin in Z). Player is JUST NORTH of the fence
        // (player.z < fence.z). After hop, player lands JUST SOUTH,
        // clear of the collider + their own radius + 2 units of margin.
        let fence_pos = Vec3::new(0.0, 30.0, 0.0);
        let fence_half = Vec3::new(40.0, 30.0, 4.0);
        let player_north = Vec3::new(0.0, 0.0, -5.0);
        let landed = hopped_position(player_north, fence_pos, fence_half, PLAYER_RADIUS);
        assert!(
            landed.z > fence_pos.z + fence_half.z,
            "player should land past the fence's far edge; landed at z={}, fence far z={}",
            landed.z,
            fence_pos.z + fence_half.z
        );
        // X and Y unchanged — we hop across the thin axis only.
        assert_eq!(landed.x, player_north.x);
        assert_eq!(landed.y, player_north.y);

        // Same fence, player on the SOUTH side — lands north.
        let player_south = Vec3::new(0.0, 0.0, 5.0);
        let landed = hopped_position(player_south, fence_pos, fence_half, PLAYER_RADIUS);
        assert!(landed.z < fence_pos.z - fence_half.z);

        // FenceNS (thin in X). Player east → hop lands west.
        let ns_half = Vec3::new(4.0, 30.0, 40.0);
        let player_east = Vec3::new(5.0, 0.0, 0.0);
        let landed = hopped_position(player_east, fence_pos, ns_half, PLAYER_RADIUS);
        assert!(landed.x < fence_pos.x - ns_half.x);
        assert_eq!(landed.z, player_east.z);
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
