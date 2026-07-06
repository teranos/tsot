// Build-info constants embedded at compile time. Ported from
// rave/src/build_info.rs.
//
// The workflow passes SEER_BUILD_COMMIT + SEER_BUILD_TIME as env
// vars to `cargo build`. `option_env!` reads them at compile time
// so the wasm bundle is self-describing — the diagnostic report
// can display the sha the WASM was compiled for, independent of
// the GITHUB_SHA the host runtime sees.
//
// Divergence between the two = stale wasm running (cache hit that
// missed a rebuild). Same value = fresh build.
//
// Fallbacks ("unknown") cover direct `cargo build` outside CI.

pub const COMMIT: &str = match option_env!("SEER_BUILD_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

pub const BUILT_AT: &str = match option_env!("SEER_BUILD_TIME") {
    Some(t) => t,
    None => "unknown",
};
