// Wasm imports allow-list checker for seer.wasm.
//
// Usage: seer-imports-check <path-to-seer.wasm> <path-to-imports.allow>
//
// Reads the wasm's import section, sorts + dedupes, compares to the
// allow-list. Exit 0 = imports match. Exit 1 = drift (added, missing,
// or renamed). Prints a diff-style listing on failure so the CI
// summary shows exactly which imports drifted.

use std::collections::BTreeSet;
use wasmparser::{Parser, Payload};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let wasm_path = args
        .next()
        .ok_or("usage: seer-imports-check <wasm> <allow>")?;
    let allow_path = args
        .next()
        .ok_or("usage: seer-imports-check <wasm> <allow>")?;

    let bytes = std::fs::read(&wasm_path)?;
    let mut actual: BTreeSet<String> = BTreeSet::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        if let Payload::ImportSection(reader) = payload? {
            for imp in reader {
                let imp = imp?;
                actual.insert(format!("{}.{}", imp.module, imp.name));
            }
        }
    }

    let allowed_text = std::fs::read_to_string(&allow_path)?;
    let expected: BTreeSet<String> = allowed_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();

    let extra: Vec<&String> = actual.difference(&expected).collect();
    let missing: Vec<&String> = expected.difference(&actual).collect();

    if extra.is_empty() && missing.is_empty() {
        println!(
            "[imports-check] OK ({} imports match {allow_path})",
            actual.len()
        );
        return Ok(());
    }

    eprintln!(
        "[imports-check] DRIFT — wasm imports differ from allow list:",
    );
    if !extra.is_empty() {
        eprintln!(
            "  {} added in wasm (not in allow list — reject or add to allow):",
            extra.len()
        );
        for i in &extra {
            eprintln!("    + {i}");
        }
    }
    if !missing.is_empty() {
        eprintln!(
            "  {} in allow list but not in wasm (dead entry — remove from allow):",
            missing.len()
        );
        for i in &missing {
            eprintln!("    - {i}");
        }
    }
    std::process::exit(1);
}
