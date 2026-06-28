#[cfg(target_arch = "wasm32")]
fn main() {
    rave::run();
}

// rave is a wasm32-only Bevy game. The native target only exists so
// `cargo test` (which spawns the host-side libp2p integration test
// in tests/positions_via_relayer.rs) can build without pulling
// Bevy + winit + wgpu — none of which are needed for the native
// test client.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}
