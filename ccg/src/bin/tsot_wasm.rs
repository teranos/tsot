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
    let _ = tsot::wasm_ffi::tsot_drain_partial_trace as usize;
    let _ = tsot::wasm_ffi::tsot_list_card_pool as usize;
    let _ = tsot::wasm_ffi::tsot_list_preset_decks as usize;
    let _ = tsot::wasm_ffi::tsot_save_game as usize;
    let _ = tsot::wasm_ffi::tsot_load_game as usize;
    let _ = tsot::wasm_ffi::tsot_test_panic as usize;
    let _ = tsot::wasm_ffi::tsot_preview_uct as usize;
    let _ = tsot::wasm_ffi::tsot_cancel_uct as usize;
    let _ = tsot::wasm_ffi::tsot_run_auto_game as usize;
}

fn main() {
    // Visible "I am alive" signal — proves emscripten invoked main()
    // in the wasm runtime. If this doesn't land in the LOG on
    // bootstrap, MODULARIZE+INVOKE_RUN isn't calling our entry point
    // and the panic hook never installed.
    tsot::trace::emit_info_public("main() ran");
    // Errors-as-first-class: every Rust panic in the wasm runtime is
    // captured into a TraceEvent::Error envelope and pushed to the
    // main thread BEFORE emscripten aborts. The LOG panel renders it
    // as a distinct block with full message + location + the trace
    // events that led up to the panic. Nothing about an error is
    // hidden behind DevTools.
    tsot::trace::install_panic_hook();
}
