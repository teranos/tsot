// Non-spatial music playback. Hand-wired boundary matching the rest
// of game's axiom (see game/CLAUDE.md): JS shim owns the AudioContext,
// Rust owns the handle lifetime. Three env.* imports:
//
//   game_audio_load(path_ptr, path_len) -> u32
//   game_audio_play(handle, volume_x1000, loop_flag)
//   game_audio_stop(handle)
//
// Async load — JS fetches + decodes off-thread. Play before decode
// finishes is a silent no-op on the JS side. Browsers also require a
// user gesture before AudioContext can produce sound; the JS shim
// buffers "wanted to play" and starts on first WASD keydown.
//
// If the asset file is missing at the URL, load succeeds with a
// handle whose play/stop are no-ops. Rave's "silent on missing" —
// no crash, no error pile-up, rest of the world keeps running.

pub const MUSIC_URL: &str = "/assets/rave.ogg";
pub const DEFAULT_VOLUME: f32 = 0.5;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_audio_load(path_ptr: *const u8, path_len: u32) -> u32;
    fn game_audio_play(handle: u32, volume_x1000: u32, loop_flag: u32);
    fn game_audio_stop(handle: u32);
    fn game_play_thump();
    fn game_play_bunk();
    fn game_play_pock();
}

// Impact SFX — three synthesized one-shots the JS shim produces via
// WebAudio oscillators. Debounced per-kind so sliding-along-a-wall
// doesn't fire every frame.
use std::sync::atomic::{AtomicU64, Ordering};
const IMPACT_DEBOUNCE_MS: u64 = 180;
static LAST_THUMP_MS: AtomicU64 = AtomicU64::new(0);
static LAST_BUNK_MS: AtomicU64 = AtomicU64::new(0);
static LAST_POCK_MS: AtomicU64 = AtomicU64::new(0);

fn debounced(last: &AtomicU64) -> bool {
    let now = crate::remote_players::now_ms();
    let prev = last.load(Ordering::Relaxed);
    if now.saturating_sub(prev) >= IMPACT_DEBOUNCE_MS {
        last.store(now, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Tree / obstacle impact — soft dull thud.
pub fn play_thump() {
    if !debounced(&LAST_THUMP_MS) {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    unsafe { game_play_thump() }
}

/// Cliff-block impact — mid-frequency "bunk" (unused until cliffs land).
pub fn play_bunk() {
    if !debounced(&LAST_BUNK_MS) {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    unsafe { game_play_bunk() }
}

/// Player-vs-player or player-vs-NPC — high, short "pock".
pub fn play_pock() {
    if !debounced(&LAST_POCK_MS) {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    unsafe { game_play_pock() }
}

pub struct GameAudioHandle(u32);

impl GameAudioHandle {
    pub fn raw(&self) -> u32 {
        self.0
    }
}

impl Drop for GameAudioHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            #[cfg(target_arch = "wasm32")]
            unsafe { game_audio_stop(self.0) }
        }
    }
}

pub fn load_music() -> GameAudioHandle {
    load(MUSIC_URL)
}

pub fn load(url: &str) -> GameAudioHandle {
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = url.as_bytes();
        let h = unsafe { game_audio_load(bytes.as_ptr(), bytes.len() as u32) };
        GameAudioHandle(h)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = url;
        GameAudioHandle(0)
    }
}

pub fn play(handle: &GameAudioHandle, volume: f32, loop_flag: bool) {
    if handle.0 == 0 {
        return;
    }
    let vol = (volume.clamp(0.0, 1.0) * 1000.0) as u32;
    let lp: u32 = if loop_flag { 1 } else { 0 };
    #[cfg(target_arch = "wasm32")]
    unsafe {
        game_audio_play(handle.0, vol, lp);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (vol, lp);
    }
}

pub fn stop(handle: &GameAudioHandle) {
    if handle.0 != 0 {
        #[cfg(target_arch = "wasm32")]
        unsafe { game_audio_stop(handle.0) }
    }
}
