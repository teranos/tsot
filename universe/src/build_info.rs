//! Build-info constants embedded at compile time. Mirrors roam's pattern.
//!
//! The Makefile passes `UNIVERSE_BUILD_COMMIT` and `UNIVERSE_BUILD_TIME`
//! as env vars to `cargo build`. `option_env!` reads them at compile time
//! so the wasm bundle is self-describing — any screenshot identifies the
//! exact build.
//!
//! Fallbacks (`"unknown"`) cover direct `cargo build` outside the Makefile.

pub const COMMIT: &str = match option_env!("UNIVERSE_BUILD_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

pub const BUILT_AT: &str = match option_env!("UNIVERSE_BUILD_TIME") {
    Some(t) => t,
    None => "unknown",
};
