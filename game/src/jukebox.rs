//! The purple jukebox — an in-world object that toggles the music when
//! you walk up to it. It's the diegetic counterpart to the left-side
//! HUD toggle: same `Music` resource, reached by moving instead of
//! tapping. Sited a short walk from the spawn point so it's the first
//! interactive thing you meet.
//!
//! The toggle is edge-triggered on *entering* the radius: stepping in
//! flips the music once, and you have to leave and come back to flip it
//! again. (A per-frame distance test without the edge would thrash the
//! mute every tick you stand next to it.)

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use crate::music::Music;
use crate::physics::{AabbCollider, PlayerMarker, Position};

/// Render/identity tag. Rendered as a solid purple box by `scene.rs`.
#[derive(Component)]
pub struct Jukebox;

/// Where it sits — a short walk east of the southern spawn (see
/// `room::SPAWN_POS`), far enough that spawning doesn't trip the
/// toggle.
pub const JUKEBOX_POS: Vec3 = Vec3::new(200.0, 0.0, 2400.0);

/// Full box size (world units). Squat and chest-high, tinted purple.
pub const JUKEBOX_SIZE: Vec3 = Vec3::new(70.0, 130.0, 70.0);

/// Purple, so it reads as the jukebox at a glance.
pub const JUKEBOX_COLOR: [f32; 3] = [0.55, 0.20, 0.75];

/// Enter within this distance (XZ) of the box to flip the music. Larger
/// than the box half-width so the toggle fires as you bump it, not only
/// when you overlap its centre.
pub const TOGGLE_RADIUS: f32 = 130.0;

pub fn setup_jukebox(mut commands: Commands) {
    commands.spawn((
        Jukebox,
        Position(JUKEBOX_POS),
        AabbCollider::cuboid(JUKEBOX_SIZE),
    ));
}

/// XZ distance test — vertical offset doesn't matter for "walked up to
/// it". Kept pure so the edge logic is unit-testable.
fn within(player: Vec3, juke: Vec3, radius: f32) -> bool {
    let dx = player.x - juke.x;
    let dz = player.z - juke.z;
    dx * dx + dz * dz < radius * radius
}

/// Flip the music the first frame the player steps inside any jukebox's
/// radius. `was_near` carries the previous frame's state so only the
/// rising edge toggles.
pub fn jukebox_proximity_system(
    player_q: Query<&Position, With<PlayerMarker>>,
    juke_q: Query<&Position, With<Jukebox>>,
    music: Option<ResMut<Music>>,
    mut was_near: Local<bool>,
) {
    let Some(mut music) = music else {
        return;
    };
    let Ok(player) = player_q.single() else {
        return;
    };
    let now_near = juke_q
        .iter()
        .any(|j| within(player.0, j.0, TOGGLE_RADIUS));
    if now_near && !*was_near {
        music.toggle();
    }
    *was_near = now_near;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::GameAudioHandle;
    use bevy_ecs::schedule::Schedule;
    use bevy_ecs::system::RunSystemOnce;

    #[test]
    fn within_is_an_xz_radius_test() {
        let j = Vec3::new(200.0, 0.0, 2400.0);
        // Vertical offset is ignored; XZ distance decides.
        assert!(within(Vec3::new(200.0, 999.0, 2400.0), j, 130.0));
        assert!(within(Vec3::new(300.0, 0.0, 2400.0), j, 130.0)); // 100 < 130
        assert!(!within(Vec3::new(400.0, 0.0, 2400.0), j, 130.0)); // 200 > 130
    }

    #[test]
    fn stepping_into_the_radius_toggles_once() {
        // A real Schedule so the system's Local<bool> edge state
        // persists across frames (RunSystemOnce would reset it).
        let mut world = World::new();
        world.insert_resource(Music {
            handle: GameAudioHandle::from_raw_for_test(0),
            playing: true,
            volume: 0.5,
        });
        let player = world
            .spawn((PlayerMarker, Position(Vec3::new(2000.0, 0.0, 2400.0))))
            .id();
        world.run_system_once(setup_jukebox).unwrap();

        let mut schedule = Schedule::default();
        schedule.add_systems(jukebox_proximity_system);
        let frame = |world: &mut World, schedule: &mut Schedule| schedule.run(world);

        // Far away: no toggle.
        frame(&mut world, &mut schedule);
        assert!(world.resource::<Music>().playing);

        // Walk onto the jukebox: one toggle → muted.
        world.get_mut::<Position>(player).unwrap().0 = JUKEBOX_POS;
        frame(&mut world, &mut schedule);
        assert!(!world.resource::<Music>().playing, "entering flips music");

        // Standing there doesn't keep toggling.
        frame(&mut world, &mut schedule);
        assert!(!world.resource::<Music>().playing, "no per-frame thrash");

        // Leave, then return → toggles again.
        world.get_mut::<Position>(player).unwrap().0 = Vec3::new(2000.0, 0.0, 2400.0);
        frame(&mut world, &mut schedule);
        world.get_mut::<Position>(player).unwrap().0 = JUKEBOX_POS;
        frame(&mut world, &mut schedule);
        assert!(world.resource::<Music>().playing, "re-entering flips back");
    }
}
