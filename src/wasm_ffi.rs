//! FFI surface for the WASM frontend. All functions are `extern "C"`,
//! string-typed (via `*const c_char` / `*mut c_char`), and use JSON for
//! anything richer than a primitive.
//!
//! Browser JS calls these as:
//! ```js
//! const result = Module.ccall("tsot_hello", "string", [], []);
//! ```
//!
//! Memory model: returned strings are heap-allocated `CString`s,
//! leaked out via `into_raw()`. JS reads them via `Module.UTF8ToString`
//! and frees via `tsot_free_string(ptr)`.

use std::ffi::{c_char, CStr, CString};

/// Allocate a `CString` and return its raw pointer. Caller is
/// responsible for calling [`tsot_free_string`] to free the memory.
fn export(s: impl Into<Vec<u8>>) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Free a string previously returned by an FFI function. JS calls
/// this once it's done with the string.
///
/// # Safety
/// `ptr` must be a pointer previously returned from one of this
/// module's FFI functions, or `null`. Calling with any other pointer
/// is undefined behavior.
#[no_mangle]
pub unsafe extern "C" fn tsot_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(ptr) });
}

/// Smoke-test export. Returns a static greeting so JS can verify the
/// wasm is loaded and the FFI boundary works. Free with
/// [`tsot_free_string`].
#[no_mangle]
pub extern "C" fn tsot_hello() -> *mut c_char {
    export(format!("tsot wasm alive (build {})", env!("CARGO_PKG_VERSION")))
}

/// Echo a string back through the FFI. Used to verify input handling
/// before wiring real game APIs. Free the result with
/// [`tsot_free_string`].
///
/// # Safety
/// `input` must be a valid pointer to a null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn tsot_echo(input: *const c_char) -> *mut c_char {
    if input.is_null() {
        return export("");
    }
    let s = unsafe { CStr::from_ptr(input) }
        .to_str()
        .unwrap_or("<invalid utf-8>");
    export(format!("echo: {s}"))
}

// Asyncify proof-of-concept. `emscripten_sleep` yields to the JS
// event loop for the given number of milliseconds, then resumes the
// Rust call as if it had blocked. With `-sASYNCIFY=1` enabled and
// the JS caller using `{async: true}` on `ccall`, this returns a
// Promise that resolves once the resume completes.
//
// If this round-trips correctly, the same mechanism extends to
// `prompt_rx.recv()` — the engine "blocks" on a channel, Asyncify
// yields back to JS, JS pushes the next action via another FFI
// call which causes the channel to wake the resumed engine
// frame. Result: synchronous-looking engine code, async-looking JS
// API. No restructuring of `run_game_continue` needed.
extern "C" {
    fn emscripten_sleep(ms: u32);
}

/// Async smoke-test export. Sleeps 100ms (yielding to JS), then
/// returns a string. JS calls as:
/// ```js
/// const ptr = await Module.ccall("tsot_async_sleep", "number",
///                                [], [], {async: true});
/// console.log(Module.UTF8ToString(ptr));
/// ```
/// Free with [`tsot_free_string`].
#[no_mangle]
pub extern "C" fn tsot_async_sleep() -> *mut c_char {
    unsafe { emscripten_sleep(100); }
    export("yielded and resumed")
}
