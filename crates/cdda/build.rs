//! Build-time materialisation of the CDDA map corpus.
//!
//! CDDA is a *dependency*, not vendored source. The mapgen + palette
//! JSON never lives in git. It comes from a CDDA source tree pinned to
//! a stable letter release (see `RELEASE`):
//!
//!   - Nix build: the `cataclysm-dda` flake input sets `$CDDA_SRC` to
//!     the pinned store path (provenance locked in flake.lock).
//!   - Bare cargo / CI: `tools/fetch.sh` populates `.cdda-src/`
//!     (gitignored) from the same pinned release.
//!
//! This script copies the referenced files out of that tree into
//! `OUT_DIR/cdda/<basename>`, where `cdda.rs` / `palette.rs` pick them
//! up with `include_str!(concat!(env!("OUT_DIR"), …))`. Adding a
//! building means adding a line here — a *reference* into the pinned
//! corpus, never a copied file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The one manifest of CDDA corpus files the game references, by their
/// path within the pinned source tree. Shared verbatim by the Nix
/// `sparseCheckout` and `tools/fetch.sh`, so "which files" has a
/// single source of truth. Adding a building = one line there.
const MANIFEST: &str = "files.txt";

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
         or run `make cdda` / `tools/fetch.sh` to populate {}",
        local.display()
    );
    // Staleness guard: fetch.sh stamps .rev with RELEASE; if
    // someone bumps RELEASE without re-fetching, we'd silently
    // compile the old corpus. Fail loudly with the fix in the message.
    // (Nix-provided CDDA_SRC skips this — it's already hash-pinned.)
    let release = std::fs::read_to_string(manifest.join("RELEASE"))
        .expect("reading RELEASE")
        .trim()
        .to_string();
    let rev_path = local.join(".rev");
    let rev = std::fs::read_to_string(&rev_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    assert!(
        rev == release,
        ".cdda-src is stale: RELEASE is {release:?} but .cdda-src/.rev is {rev:?}. \
         Run `make cdda` to refresh."
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

    // Guard against silent clobber: two manifest entries with the same
    // basename would overwrite each other under `OUT_DIR/cdda/<basename>`
    // and `include_str!` would embed the wrong bytes with no error.
    // Fail loudly here — the manifest is small, this scan is trivial.
    let mut seen: HashMap<&str, &str> = HashMap::new();
    for rel in &files {
        let name = Path::new(rel).file_name().unwrap().to_str().unwrap();
        if let Some(prev) = seen.insert(name, rel) {
            panic!(
                "files.txt has two entries whose basename collides on OUT_DIR/cdda/{name}: \
                 {prev} and {rel} — rename one or copy under the full relative path"
            );
        }
    }

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
        // If a previous build left the destination present + read-only
        // (fs::copy preserves the Nix-store 0444 bit onto the OUT_DIR
        // copy), a re-run would fail with "Permission denied". Clear
        // the write bit before copying over.
        if dst.exists() {
            let mut perms = std::fs::metadata(&dst).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            std::fs::set_permissions(&dst, perms).ok();
        }
        std::fs::copy(&src, &dst).unwrap_or_else(|e| {
            panic!("copying {} -> {}: {e}", src.display(), dst.display())
        });
        // Make the OUT_DIR copy writable so subsequent build.rs runs
        // can overwrite it (the Nix-store source is 0444; fs::copy
        // preserves that mode onto the destination).
        let mut perms = std::fs::metadata(&dst).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&dst, perms).ok();
        // Rebuild if the pinned source file changes.
        println!("cargo:rerun-if-changed={}", src.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rerun-if-changed=RELEASE");
    println!("cargo:rerun-if-env-changed=CDDA_SRC");
}
