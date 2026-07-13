//! Build-time materialisation of the CDDA map corpus.
//!
//! CDDA is a *dependency*, not vendored source. The mapgen + palette
//! JSON never lives in git. It comes from a CDDA source tree pinned to
//! a stable letter release (see `CDDA_RELEASE`):
//!
//!   - Nix build: the `cataclysm-dda` flake input sets `$CDDA_SRC` to
//!     the pinned store path (provenance locked in flake.lock).
//!   - Bare cargo / CI: `tools/fetch-cdda.sh` populates `.cdda-src/`
//!     (gitignored) from the same pinned release.
//!
//! This script copies the referenced files out of that tree into
//! `OUT_DIR/cdda/<basename>`, where `cdda.rs` / `palette.rs` pick them
//! up with `include_str!(concat!(env!("OUT_DIR"), …))`. Adding a
//! building means adding a line here — a *reference* into the pinned
//! corpus, never a copied file.

use std::path::{Path, PathBuf};

/// The one manifest of CDDA corpus files the game references, by their
/// path within the pinned source tree. Shared verbatim by the Nix
/// `sparseCheckout` and `tools/fetch-cdda.sh`, so "which files" has a
/// single source of truth. Adding a building = one line there.
const MANIFEST: &str = "cdda-files.txt";

/// Resolve the CDDA source-tree root: the Nix-pinned input if present,
/// else the local `.cdda-src/` cache. Missing → a loud, actionable
/// failure (errors are sacred: a broken corpus must stop the build with
/// the fix in the message, not silently drop buildings).
fn cdda_src_root() -> PathBuf {
    if let Ok(p) = std::env::var("CDDA_SRC") {
        let p = PathBuf::from(p);
        assert!(
            p.is_dir(),
            "CDDA_SRC={} is not a directory",
            p.display()
        );
        return p;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let local = manifest.join(".cdda-src");
    assert!(
        local.is_dir(),
        "CDDA corpus not found. Set $CDDA_SRC (the Nix flake does this) \
         or run `make cdda` / `tools/fetch-cdda.sh` to populate {}",
        local.display()
    );
    local
}

fn main() {
    let root = cdda_src_root();
    let manifest_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(MANIFEST);
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", manifest_path.display()));
    let files: Vec<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dst_dir = out_dir.join("cdda");
    std::fs::create_dir_all(&dst_dir).unwrap();

    for rel in &files {
        let src = root.join(rel);
        let name = Path::new(rel).file_name().unwrap();
        let dst = dst_dir.join(name);
        assert!(
            src.is_file(),
            "referenced CDDA file missing from the pinned corpus: {} \
             (looked in {})",
            rel,
            root.display()
        );
        std::fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!("copying {} -> {}: {e}", src.display(), dst.display())
        });
        // Rebuild if the pinned source file changes.
        println!("cargo:rerun-if-changed={}", src.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rerun-if-changed=CDDA_RELEASE");
    println!("cargo:rerun-if-env-changed=CDDA_SRC");
}
