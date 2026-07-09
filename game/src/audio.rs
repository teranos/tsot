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
    /// Play a PCM sample buffer through the browser's AudioContext.
    /// JS wraps the samples in an AudioBuffer and starts a one-shot
    /// BufferSourceNode. Samples must remain valid for the duration
    /// of the extern call (JS copies before returning).
    fn game_audio_play_samples(sample_ptr: *const f32, sample_count: u32, sample_rate: u32);
}

// Rust owns audio synthesis. JS is a dumb sink: it copies our PCM
// samples into an AudioBuffer and plays. When music becomes
// reactive/interactive (Python gen-rave-ogg.py replacement), the
// same pipe carries continuous samples.
pub const SAMPLE_RATE: u32 = 44100;

#[derive(Clone, Copy)]
pub enum WaveType {
    Sine,
    Square,
    Triangle,
}

/// One-shot impact envelope: pitch-swept oscillator + exponential
/// gain decay to near-silent. Returns mono f32 samples at
/// SAMPLE_RATE. Same tonal recipe the JS oscillators used, now
/// materialised sample-by-sample in Rust.
pub fn synthesize_impact(
    freq_start: f32,
    freq_end: f32,
    dur_sec: f32,
    wave: WaveType,
    gain_start: f32,
) -> Vec<f32> {
    let n = (dur_sec * SAMPLE_RATE as f32) as usize;
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0_f32;
    let freq_ratio = (freq_end / freq_start.max(1e-3)).max(1e-3);
    let gain_end = 1e-4_f32;
    let gain_ratio = (gain_end / gain_start.max(1e-4)).max(1e-6);
    let inv_sr = 1.0 / SAMPLE_RATE as f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let freq = freq_start * freq_ratio.powf(t);
        phase += freq * inv_sr;
        if phase >= 1.0 {
            phase -= phase.floor();
        }
        let sample = match wave {
            WaveType::Sine => (phase * std::f32::consts::TAU).sin(),
            WaveType::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            WaveType::Triangle => {
                if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                }
            }
        };
        let gain = gain_start * gain_ratio.powf(t);
        out.push(sample * gain);
    }
    out
}

fn play_samples(samples: &[f32]) {
    if samples.is_empty() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    unsafe {
        game_audio_play_samples(samples.as_ptr(), samples.len() as u32, SAMPLE_RATE);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = samples;
    }
}

// Debounce per-kind — sliding along a wall or grinding into a peer
// shouldn't fire every frame.
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
    let samples = synthesize_impact(120.0, 45.0, 0.13, WaveType::Sine, 0.25);
    play_samples(&samples);
}

/// Cliff-block impact — mid-frequency "bunk" (unused until cliffs land).
pub fn play_bunk() {
    if !debounced(&LAST_BUNK_MS) {
        return;
    }
    let samples = synthesize_impact(220.0, 90.0, 0.09, WaveType::Triangle, 0.2);
    play_samples(&samples);
}

/// Player-vs-player or player-vs-NPC — high, short "pock".
pub fn play_pock() {
    if !debounced(&LAST_POCK_MS) {
        return;
    }
    let samples = synthesize_impact(650.0, 260.0, 0.05, WaveType::Square, 0.15);
    play_samples(&samples);
}

/// Firewood crackle — continuous soft hiss with occasional pop bursts.
/// PRNG-driven so it doesn't repeat identically between bursts;
/// `seed` advances per call. Returns mono f32 samples at SAMPLE_RATE.
pub fn synthesize_crackle(dur_sec: f32, seed: u64) -> Vec<f32> {
    let n = (dur_sec * SAMPLE_RATE as f32) as usize;
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n);
    let mut state = seed | 1;
    let mut pop_env: f32 = 0.0;
    let inv_u32 = 1.0 / u32::MAX as f32;
    for _ in 0..n {
        // xorshift64 — three samples per output frame (hiss, pop-trigger, pop-content)
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let r1 = (state as u32) as f32 * inv_u32 * 2.0 - 1.0;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let r2 = (state as u32) as f32 * inv_u32;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let r3 = (state as u32) as f32 * inv_u32 * 2.0 - 1.0;
        // Soft continuous hiss
        let hiss = r1 * 0.04;
        // Random pop trigger — ~0.3% of samples per frame
        if r2 > 0.997 {
            pop_env = 0.5 + r3.abs() * 0.3;
        }
        let pop = r3 * pop_env;
        pop_env *= 0.9992;
        out.push((hiss + pop).clamp(-0.9, 0.9));
    }
    out
}

static CRACKLE_SEED: AtomicU64 = AtomicU64::new(0x9e3779b97f4a7c15);

/// Play a firewood crackle burst at scaled volume. Advances the
/// internal PRNG seed so successive calls produce distinct bursts.
pub fn play_crackle(dur_sec: f32, volume: f32) {
    if volume < 1e-3 {
        return;
    }
    let seed = CRACKLE_SEED
        .fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed)
        .wrapping_add(1);
    let samples = synthesize_crackle(dur_sec, seed);
    let scaled: Vec<f32> = samples.iter().map(|s| s * volume).collect();
    play_samples(&scaled);
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
