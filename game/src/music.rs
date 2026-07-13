//! The running music track's control state — one resource shared by
//! every way the player can change the music: the left-side HUD toggle,
//! the in-world purple jukebox, and the settings volume slider.
//!
//! It never stops + reloads the track. `playing` is a *mute*, not a
//! teardown: the effective volume drops to zero when off and returns to
//! `volume` when on, applied live through the GainNode (see
//! `audio::set_volume`). So toggling is instant and the loop keeps its
//! position, and the volume slider slides the level with no gap.

use bevy_ecs::prelude::*;

use crate::audio::{self, GameAudioHandle};

/// The one music track, its handle and its live control state.
#[derive(Resource)]
pub struct Music {
    pub handle: GameAudioHandle,
    /// False = muted (effective volume 0), true = audible at `volume`.
    pub playing: bool,
    /// The chosen level in [0,1] — what `playing` restores to.
    pub volume: f32,
}

impl Music {
    /// The level actually sent to the mixer: `volume` when playing,
    /// silence when muted.
    pub fn effective_volume(&self) -> f32 {
        if self.playing { self.volume } else { 0.0 }
    }

    /// Push the current effective volume to the live track.
    pub fn apply(&self) {
        audio::set_volume(&self.handle, self.effective_volume());
    }

    /// Flip mute on/off and apply immediately.
    pub fn toggle(&mut self) {
        self.playing = !self.playing;
        self.apply();
    }

    /// Set the chosen level (clamped) and apply. Setting a non-zero
    /// level while muted leaves it muted — the slider changes what
    /// *un*muting will restore to, matching a real mixer channel.
    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        self.apply();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn music() -> Music {
        // Handle 0 → audio calls are no-ops; the state machine is what
        // we test here.
        Music {
            handle: GameAudioHandle::from_raw_for_test(0),
            playing: true,
            volume: 0.5,
        }
    }

    #[test]
    fn effective_volume_follows_the_mute() {
        let mut m = music();
        assert_eq!(m.effective_volume(), 0.5);
        m.toggle();
        assert!(!m.playing);
        assert_eq!(m.effective_volume(), 0.0, "muted track is silent");
        m.toggle();
        assert!(m.playing);
        assert_eq!(m.effective_volume(), 0.5, "unmute restores the level");
    }

    #[test]
    fn set_volume_clamps_and_persists_through_mute() {
        let mut m = music();
        m.set_volume(2.0);
        assert_eq!(m.volume, 1.0, "over-1 clamps to 1");
        m.set_volume(-1.0);
        assert_eq!(m.volume, 0.0, "under-0 clamps to 0");
        m.set_volume(0.3);
        m.toggle(); // mute
        assert_eq!(m.effective_volume(), 0.0);
        assert_eq!(m.volume, 0.3, "the chosen level survives muting");
        m.toggle(); // unmute
        assert_eq!(m.effective_volume(), 0.3);
    }
}
