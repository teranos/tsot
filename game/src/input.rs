// Hand-wired input state. Two sources OR'd together:
//   1. Keyboard — JS shim maintains a u32 bitmask via window
//      keydown/keyup listeners; Rust polls via game_input_state.
//   2. Touch — Rust reads raw touch positions via game_touch_state,
//      hit-tests them against Bevy-owned D-pad button rectangles
//      in dpad.rs, and stores the resulting bits in TOUCH_BITS.
// The combined state is what physics::keyboard_input reads.

use std::sync::atomic::{AtomicU32, Ordering};

pub mod key {
    pub const W: u32 = 0x0001;
    pub const A: u32 = 0x0002;
    pub const S: u32 = 0x0004;
    pub const D: u32 = 0x0008;
    pub const ESC: u32 = 0x0010;
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_input_state() -> u32;
    /// Accumulated wheel deltaY since last read, in browser wheel-notch
    /// units (positive = scrolled up in our convention). Reading resets
    /// the accumulator; call at most once per consumer per frame.
    fn game_wheel_delta() -> i32;
    /// Current pointer position in NDC. Out-of-range (< -1 or > 1) means
    /// the pointer isn't over the canvas.
    fn game_pointer_ndc_x() -> f32;
    fn game_pointer_ndc_y() -> f32;
}

static TOUCH_BITS: AtomicU32 = AtomicU32::new(0);

/// Called by the D-pad hit-test system every frame with the bits
/// derived from active touches inside D-pad button rectangles.
pub fn set_touch_bits(bits: u32) {
    TOUCH_BITS.store(bits, Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
pub fn state() -> u32 {
    let kb = unsafe { game_input_state() };
    kb | TOUCH_BITS.load(Ordering::Relaxed)
}

#[cfg(not(target_arch = "wasm32"))]
pub fn state() -> u32 {
    TOUCH_BITS.load(Ordering::Relaxed)
}

/// Accumulated mouse-wheel delta since last call. Positive = wheel up.
/// Reading resets the accumulator on the JS side.
#[cfg(target_arch = "wasm32")]
pub fn wheel_delta() -> i32 {
    unsafe { game_wheel_delta() }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn wheel_delta() -> i32 {
    0
}

/// Current pointer position in NDC (\[−1, 1\] × \[−1, 1\]). Returns
/// `None` when the pointer isn't over the canvas — JS reports off-range
/// coords in that case and we filter here so callers can pattern-match.
#[cfg(target_arch = "wasm32")]
pub fn pointer_ndc() -> Option<[f32; 2]> {
    let x = unsafe { game_pointer_ndc_x() };
    let y = unsafe { game_pointer_ndc_y() };
    if x.abs() > 1.0 || y.abs() > 1.0 {
        None
    } else {
        Some([x, y])
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pointer_ndc() -> Option<[f32; 2]> {
    None
}
