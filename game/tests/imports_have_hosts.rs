// `imports.allow` is a contract with TWO implementations, and nothing
// was checking the second one.
//
// game.wasm runs in the browser, where `web/src/main.ts` supplies the
// env.* functions, and under wasmtime, where
// `crates/seer-host/src/imports.rs` supplies them. `seer-imports-check`
// verifies the wasm's imports match the allow list — but the allow list
// matching is not the same claim as somebody actually implementing
// them. A host missing one cannot instantiate the module at all: a
// blank page in the browser, a dead seer run in CI. Neither is a
// compile error, and both arrive long after the change that caused it.
//
// This is the check whose absence let `game_pointer_buttons` and
// `game_wheel_delta_y` reach CI implemented in the browser shim only.
//
// Text matching, deliberately: the alternative is instantiating the
// module against each host, which means building for wasm32 from a
// host-target test and linking wasmtime into game's dev-dependencies —
// far more machinery than the bug warrants. The delimiters make it
// exact rather than fuzzy: a trailing `:` for the TS object property
// and surrounding quotes for the Rust string literal, so
// `game_wheel_delta` cannot pass by being a prefix of
// `game_wheel_delta_y`.

use std::path::PathBuf;

fn game_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = game_dir().join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Every non-comment `module.name` entry, as the bare function name.
fn allowed_imports() -> Vec<String> {
    read("imports.allow")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            l.split_once('.')
                .unwrap_or_else(|| panic!("malformed imports.allow entry: {l:?}"))
                .1
                .to_string()
        })
        .collect()
}

#[test]
fn the_browser_shim_implements_every_allowed_import() {
    let js = read("web/src/main.ts");
    let missing: Vec<String> = allowed_imports()
        .into_iter()
        .filter(|n| !js.contains(&format!("{n}:")))
        .collect();
    assert!(
        missing.is_empty(),
        "web/src/main.ts implements no env.* entry for: {missing:?}\n\
         The wasm imports it, so the browser cannot instantiate the \
         module — game.sbvh.nl would be a blank page, not an error."
    );
}

#[test]
fn the_wasmtime_host_implements_every_allowed_import() {
    let host = read("../crates/seer-host/src/imports.rs");
    let missing: Vec<String> = allowed_imports()
        .into_iter()
        .filter(|n| !host.contains(&format!("\"{n}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "crates/seer-host/src/imports.rs has no func_wrap for: {missing:?}\n\
         The wasm imports it, so wasmtime cannot instantiate the module \
         and the seer run dies before it measures anything."
    );
}
