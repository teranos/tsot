//! Persistent player position — saved to browser storage via the JS
//! shim, loaded on boot so you resume where you left off. Same
//! hand-wired boundary as identity: two env.* imports, Rust owns the
//! encode/decode (3 × f32 little-endian), JS owns the store.

use bevy_math::Vec3;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_position_load(out_ptr: *mut u8) -> u32;
    fn game_position_save(bytes_ptr: *const u8, bytes_len: u32);
}

#[cfg(any(target_arch = "wasm32", test))]
const POS_LEN: usize = 12;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_round_trips_through_bytes() {
        let p = Vec3::new(1234.5, -20.0, -6789.25);
        assert_eq!(decode(&encode(p)), p);
    }
}
