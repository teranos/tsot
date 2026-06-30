#[cfg(target_arch = "wasm32")]
pub use bridge::{js_rave_load_identity, js_rave_save_identity};

#[cfg(target_arch = "wasm32")]
mod bridge {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    unsafe extern "C" {
        #[wasm_bindgen(js_namespace = window, js_name = "__raveLoadIdentity")]
        pub fn js_rave_load_identity() -> js_sys::Promise;

        #[wasm_bindgen(js_namespace = window, js_name = "__raveSaveIdentity")]
        pub fn js_rave_save_identity(bytes: js_sys::Uint8Array) -> js_sys::Promise;
    }
}
