// Hand-wired keyboard state. JS shim maintains a u32 bitmask
// updated by window keydown/keyup listeners. Rust polls it each
// frame via the game_input_state env.* import.

pub mod key {
    pub const W: u32 = 0x0001;
    pub const A: u32 = 0x0002;
    pub const S: u32 = 0x0004;
    pub const D: u32 = 0x0008;
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_input_state() -> u32;
}

#[cfg(target_arch = "wasm32")]
pub fn state() -> u32 {
    unsafe { game_input_state() }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn state() -> u32 {
    0
}
