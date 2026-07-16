//! The SFX (sound-effects) mixer channel — one shared level applied
//! to every non-music sample the game emits (thump, pock, alert,
//! crackle). Parallels `music::Music` but only holds a volume; a
//! separate slider in the settings panel drives it, and the level is
//! persisted so the next boot resumes with the same mix.

use bevy_ecs::prelude::*;

use crate::{audio, persist};

/// The chosen SFX level in [0,1]. Applied via a global atomic in
/// `audio` so the free-function play paths (physics, campfire) can
/// scale samples without threading a Bevy resource through every call.
#[derive(Resource)]
pub struct SfxMix {
    pub volume: f32,
}

impl SfxMix {
    /// Set the level (clamped), push to the audio bus, and persist so
    /// the next boot restores it.
    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        audio::set_sfx_volume(self.volume);
        persist::save_sfx(self.volume);
    }
}

/// One-shot setup: load persisted level (else default 1.0 = unchanged
/// SFX loudness), push into the audio bus, insert the resource.
pub fn setup_sfx(mut commands: Commands) {
    let volume = persist::load_sfx().unwrap_or(1.0);
    audio::set_sfx_volume(volume);
    commands.insert_resource(SfxMix { volume });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_volume_clamps() {
        let mut m = SfxMix { volume: 0.5 };
        m.set_volume(2.0);
        assert_eq!(m.volume, 1.0);
        m.set_volume(-1.0);
        assert_eq!(m.volume, 0.0);
        m.set_volume(0.3);
        assert!((m.volume - 0.3).abs() < 1e-6);
    }
}
