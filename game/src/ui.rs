// Hand-wired UI notifications from Rust to the JS overlay.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_show_exclamation();
}

pub fn show_exclamation() {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        game_show_exclamation()
    }
}
