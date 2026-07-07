// Hand-wired UI notifications from Rust to the JS overlay.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_show_exclamation(clip_x: f32, clip_y: f32);
}

pub fn show_exclamation(clip_x: f32, clip_y: f32) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        game_show_exclamation(clip_x, clip_y)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (clip_x, clip_y);
    }
}
