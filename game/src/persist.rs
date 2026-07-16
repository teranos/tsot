//! Persistent player state — position + music preference — saved to
//! browser storage via the JS shim, loaded on boot so you resume where
//! you left off with the audio mix you had. Same hand-wired boundary
//! as identity: env.* imports, Rust owns the encode/decode, JS owns
//! the store.

use bevy_math::Vec3;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_position_load(out_ptr: *mut u8) -> u32;
    fn game_position_save(bytes_ptr: *const u8, bytes_len: u32);
    fn game_music_state_load(out_ptr: *mut u8) -> u32;
    fn game_music_state_save(bytes_ptr: *const u8, bytes_len: u32);
    fn game_sfx_state_load(out_ptr: *mut u8) -> u32;
    fn game_sfx_state_save(bytes_ptr: *const u8, bytes_len: u32);
}

#[cfg(any(target_arch = "wasm32", test))]
const POS_LEN: usize = 12;
#[cfg(any(target_arch = "wasm32", test))]
const MUSIC_LEN: usize = 5;
#[cfg(target_arch = "wasm32")]
const SFX_LEN: usize = 4;

#[cfg(any(target_arch = "wasm32", test))]
fn encode(p: Vec3) -> [u8; POS_LEN] {
    let mut b = [0u8; POS_LEN];
    b[0..4].copy_from_slice(&p.x.to_le_bytes());
    b[4..8].copy_from_slice(&p.y.to_le_bytes());
    b[8..12].copy_from_slice(&p.z.to_le_bytes());
    b
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode(b: &[u8; POS_LEN]) -> Vec3 {
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    Vec3::new(f(0), f(4), f(8))
}

#[cfg(any(target_arch = "wasm32", test))]
fn encode_music(playing: bool, volume: f32) -> [u8; MUSIC_LEN] {
    let mut b = [0u8; MUSIC_LEN];
    b[0] = if playing { 1 } else { 0 };
    b[1..5].copy_from_slice(&volume.to_le_bytes());
    b
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode_music(b: &[u8; MUSIC_LEN]) -> (bool, f32) {
    let volume = f32::from_le_bytes([b[1], b[2], b[3], b[4]]);
    (b[0] != 0, volume)
}

/// The stored player position, if any (None → first visit / native).
#[cfg(target_arch = "wasm32")]
pub fn load() -> Option<Vec3> {
    let mut buf = [0u8; POS_LEN];
    let n = unsafe { game_position_load(buf.as_mut_ptr()) };
    (n as usize == POS_LEN).then(|| decode(&buf))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load() -> Option<Vec3> {
    None
}

/// Persist the player position.
#[cfg(target_arch = "wasm32")]
pub fn save(p: Vec3) {
    let b = encode(p);
    unsafe { game_position_save(b.as_ptr(), POS_LEN as u32) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save(_p: Vec3) {}

/// The stored (playing, volume). None → first visit / native.
#[cfg(target_arch = "wasm32")]
pub fn load_music() -> Option<(bool, f32)> {
    let mut buf = [0u8; MUSIC_LEN];
    let n = unsafe { game_music_state_load(buf.as_mut_ptr()) };
    (n as usize == MUSIC_LEN).then(|| decode_music(&buf))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_music() -> Option<(bool, f32)> {
    None
}

/// Persist (playing, volume) — called from `Music::toggle` /
/// `Music::set_volume` so the next boot starts with the same mix.
#[cfg(target_arch = "wasm32")]
pub fn save_music(playing: bool, volume: f32) {
    let b = encode_music(playing, volume);
    unsafe { game_music_state_save(b.as_ptr(), MUSIC_LEN as u32) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_music(_playing: bool, _volume: f32) {}

/// The stored SFX level. None → first visit / native.
#[cfg(target_arch = "wasm32")]
pub fn load_sfx() -> Option<f32> {
    let mut buf = [0u8; SFX_LEN];
    let n = unsafe { game_sfx_state_load(buf.as_mut_ptr()) };
    (n as usize == SFX_LEN).then(|| f32::from_le_bytes(buf))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_sfx() -> Option<f32> {
    None
}

/// Persist the SFX level — called from `SfxMix::set_volume`.
#[cfg(target_arch = "wasm32")]
pub fn save_sfx(volume: f32) {
    let b = volume.to_le_bytes();
    unsafe { game_sfx_state_save(b.as_ptr(), SFX_LEN as u32) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_sfx(_volume: f32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_round_trips_through_bytes() {
        let p = Vec3::new(1234.5, -20.0, -6789.25);
        assert_eq!(decode(&encode(p)), p);
    }

    #[test]
    fn music_state_round_trips_through_bytes() {
        assert_eq!(decode_music(&encode_music(true, 0.42)), (true, 0.42));
        assert_eq!(decode_music(&encode_music(false, 0.0)), (false, 0.0));
        assert_eq!(decode_music(&encode_music(false, 1.0)), (false, 1.0));
    }

    #[test]
    fn sfx_level_round_trips_through_bytes() {
        let v = 0.42_f32;
        assert_eq!(f32::from_le_bytes(v.to_le_bytes()), v);
    }
}
