//! Spatial audio — bundled music plays at the two speakers, listener
//! follows the camera. Distance-based attenuation does the rest: quiet
//! at spawn, loud at the dancefloor.
//!
//! The track itself sits at `rave/assets/music/rave.ogg` (user-supplied,
//! CC0 from Free Music Archive / Pixabay / Mixkit). If the file is
//! missing the AssetServer load returns a handle that never resolves
//! and Bevy's audio plugin stays silent — no crash, no error pile-up,
//! the rest of the world still renders. Once the file is dropped in
//! and the Makefile copies it into `dist/assets/`, music plays on
//! the next deploy.

use bevy::prelude::*;

use crate::floorplan::Speaker;

/// Distance between the listener's two virtual ears (world units).
/// Bevy uses it to compute the stereo gap for panning. The player
/// sphere has radius 20; matching gap roughly to head width keeps
/// the panning natural-feeling at human scale.
const LISTENER_EAR_GAP: f32 = 6.0;

/// Asset path (relative to `assets/` directory) of the bundled track.
/// Single OGG Vorbis stream loops at each speaker.
const MUSIC_ASSET_PATH: &str = "music/rave.ogg";

/// Volume scalar applied to each speaker's playback. Bevy multiplies
/// by distance attenuation on top; this is the at-source baseline.
/// Two speakers playing the same track sum together at the listener,
/// so each is half of the intended at-clearing loudness.
const PER_SPEAKER_VOLUME: f32 = 0.5;

/// Attach an `AudioPlayer` to every entity tagged `Speaker` (spawned
/// by `floorplan::setup_floor_plan`), and a `SpatialListener` to
/// the camera so its world-space transform drives attenuation.
///
/// Run order: after `floorplan::setup_floor_plan` and
/// `setup_scene_lights` so the Speaker + Camera3d entities both exist.
pub fn setup_audio(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    speakers: Query<Entity, With<Speaker>>,
    cameras: Query<Entity, With<Camera3d>>,
) {
    let track: Handle<AudioSource> = asset_server.load(MUSIC_ASSET_PATH);
    for speaker in &speakers {
        commands.entity(speaker).insert((
            AudioPlayer::<AudioSource>::new(track.clone()),
            PlaybackSettings::LOOP
                .with_spatial(true)
                .with_volume(bevy::audio::Volume::Linear(PER_SPEAKER_VOLUME)),
        ));
    }
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(SpatialListener::new(LISTENER_EAR_GAP));
    }
}
