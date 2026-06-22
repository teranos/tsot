//! Build-info constants embedded at compile time.
//!
//! The Makefile passes `ROAM_BUILD_COMMIT`, `ROAM_BUILD_TIME`, and
//! `ROAM_BUILD_PROFILE` as env vars to `cargo build`. `option_env!`
//! reads them at compile time so the wasm bundle is self-describing;
//! JS doesn't need to ship the same values via
//! `window.__ROAM_BUILD__` for Rust to see them.
//!
//! Fallbacks (`"unknown"` / `"dev"`) cover the case where someone
//! builds with `cargo build` directly outside the Makefile — the
//! constants still resolve, just without git context.

pub const COMMIT: &str = match option_env!("ROAM_BUILD_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

pub const BUILT_AT: &str = match option_env!("ROAM_BUILD_TIME") {
    Some(t) => t,
    None => "unknown",
};

pub const PROFILE: &str = match option_env!("ROAM_BUILD_PROFILE") {
    Some(p) => p,
    None => "dev",
};
