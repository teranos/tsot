//! WASM entry point. Emscripten requires a `main()` symbol in the
//! linked binary; this is the smallest one we can give it. The
//! actual FFI surface lives in `tsot::wasm_ffi` and is reachable
//! through `Module.ccall` from JS.

// Force the wasm_ffi module's `extern "C"` symbols to be linked into
// the binary. Without an explicit reference the linker would
// dead-strip them.
#[cfg(target_arch = "wasm32")]
#[allow(dead_code)]
fn _retain_ffi_symbols() {
    let _ = tsot::wasm_ffi::tsot_hello as usize;
    let _ = tsot::wasm_ffi::tsot_echo as usize;
    let _ = tsot::wasm_ffi::tsot_free_string as usize;
    let _ = tsot::wasm_ffi::tsot_start_game as usize;
    let _ = tsot::wasm_ffi::tsot_apply_action as usize;
}

fn main() {}
