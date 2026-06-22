//! v0.5.0 spike — minimal compile-only stub that proves Bevy resolves
//! and links into roam's wasm bundle alongside libp2p + eframe. No
//! runtime yet. Per `docs/adr/0003-bevy.md` and the README v0.5
//! phased roadmap. Deleted in v0.5.1 once the real port begins.

#![cfg(target_arch = "wasm32")]

pub fn _bevy_link_check() {
    let _app = bevy::app::App::new();
}
