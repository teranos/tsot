//! CDDA mapgen coverage report — HANDOVER checklist #2.
//!
//! Walks the pinned CDDA mapgen tree (`$CDDA_SRC` or `.cdda-src`), pulls
//! every `om_terrain` name it finds, tries to resolve each through
//! `cdda::mapgen_to_template`, and reports what worked, what didn't, and
//! *why*. Turns "add buildings" from vibes into a measured push: the
//! failure categories tell you which unhandled feature (nested mapgen,
//! multi-tile overmap, missing palette symbol) blocks the most buildings.
//!
//! Usage: `cargo run --bin cdda-coverage --release`.
//! Or with a specific corpus:
//! `CDDA_SRC=/path/to/cdda-src cargo run --bin cdda-coverage`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use game::cdda::placement::{CDDA_TILE, CddaError, mapgen_to_template};
use serde_json::Value;

fn corpus_root() -> PathBuf {
    if let Ok(p) = std::env::var("CDDA_SRC") {
        return PathBuf::from(p);
    }
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join(".cdda-src")
}

/// Walk `dir` recursively, collecting every `*.json` path.
fn walk_json(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            walk_json(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

/// Extract every om_terrain string in a JSON entry (a string, or a nested
/// array). Skips entries whose `object` is missing rows (palettes, etc).
fn om_terrains(entry: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => {
                for x in a {
                    walk(x, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&entry["om_terrain"], &mut out);
    out
}

/// Which unhandled CDDA features does this mapgen use? Presence-only.
fn unhandled_features(entry: &Value) -> Vec<&'static str> {
    let obj = &entry["object"];
    let mut found = Vec::new();
    for feat in [
        "place_nested",
        "place_vehicles",
        "place_monsters",
        "place_monster",
        "place_loot",
        "place_item",
        "place_items",
        "nested",
        "place_liquids",
        "place_npcs",
        "place_signs",
        "place_graffiti",
        "place_traps",
        "place_fields",
        "place_furniture",
        "place_terrain",
        "place_zones",
        "place_ter_furn_transforms",
    ] {
        if obj.get(feat).is_some() {
            found.push(feat);
        }
    }
    found
}

#[derive(Debug, Default)]
struct Report {
    total_entries_scanned: usize,
    /// om_terrain name → outcome.
    outcomes: BTreeMap<String, Outcome>,
}

#[derive(Debug)]
enum Outcome {
    /// The entry has a resolvable roof pair and produced ≥ N props.
    Resolved { props: usize },
    /// Ran `mapgen_to_template` but the result was empty — likely no
    /// wall/window/furniture chars mapped.
    Empty,
    /// Not a mapgen we can even try (missing `object`, no `rows`, …).
    /// The tag is kept in `Debug` for diagnostics; not read directly.
    #[allow(dead_code)]
    Skipped(&'static str),
    /// Attempted resolution failed inside our importer.
    Failed(String),
}

fn main() {
    let root = corpus_root();
    let mapgen_root = root.join("data/json/mapgen");
    if !mapgen_root.is_dir() {
        eprintln!("coverage: no mapgen tree at {}", mapgen_root.display());
        std::process::exit(1);
    }

    let mut files = Vec::new();
    walk_json(&mapgen_root, &mut files);
    files.sort();
    eprintln!("[coverage] scanning {} mapgen files", files.len());

    let mut report = Report::default();
    let mut unhandled_hits: BTreeMap<&'static str, usize> = BTreeMap::new();

    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let Ok(root_val) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let entries: Vec<Value> = match root_val {
            Value::Array(a) => a,
            other => vec![other],
        };
        for entry in entries {
            let ty = entry["type"].as_str().unwrap_or("");
            // Only mapgen entries — skip palettes, overlays, etc.
            if ty != "mapgen" && ty != "overmap_special" && !ty.is_empty() {
                continue;
            }
            let om_names = om_terrains(&entry);
            if om_names.is_empty() {
                continue;
            }
            for feat in unhandled_features(&entry) {
                *unhandled_hits.entry(feat).or_default() += 1;
            }
            for name in om_names {
                if report.outcomes.contains_key(&name) {
                    continue; // first occurrence wins
                }
                report.total_entries_scanned += 1;
                // Skip entries whose object is a nested-mapgen-only chunk.
                let has_rows = entry["object"]["rows"].is_array();
                if !has_rows {
                    report
                        .outcomes
                        .insert(name, Outcome::Skipped("no rows"));
                    continue;
                }
                let outcome = match mapgen_to_template(&text, &name, CDDA_TILE, 0) {
                    Ok(t) if t.props.is_empty() => Outcome::Empty,
                    Ok(t) => Outcome::Resolved { props: t.props.len() },
                    Err(CddaError::Parse(m)) => {
                        Outcome::Failed(format!("Parse: {m}"))
                    }
                    Err(CddaError::NotFound(_)) => {
                        // We passed the exact name we found — anything but
                        // ok here means the om_terrain array shape was odd.
                        Outcome::Skipped("shape")
                    }
                    Err(CddaError::NoObject(_)) => Outcome::Skipped("no object"),
                };
                report.outcomes.insert(name, outcome);
            }
        }
    }

    let mut resolved = 0;
    let mut empty = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut props_total = 0;
    for outcome in report.outcomes.values() {
        match outcome {
            Outcome::Resolved { props } => {
                resolved += 1;
                props_total += props;
            }
            Outcome::Empty => empty += 1,
            Outcome::Skipped(_) => skipped += 1,
            Outcome::Failed(_) => failed += 1,
        }
    }

    println!("=== CDDA mapgen coverage ===");
    println!("mapgen files scanned : {}", files.len());
    println!("distinct om_terrains : {}", report.outcomes.len());
    println!("  resolved (>0 props): {resolved}");
    println!("  empty (parsed, no matched props): {empty}");
    println!("  skipped (no rows / no object / wrong shape): {skipped}");
    println!("  failed  (importer error): {failed}");
    println!("  total props across resolved buildings: {props_total}");
    println!();

    println!("=== Unhandled feature hits (across all mapgen entries) ===");
    let mut feat_counts: Vec<_> = unhandled_hits.iter().collect();
    feat_counts.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (feat, n) in &feat_counts {
        println!("  {feat:<28} {n}");
    }
    println!();

    println!("=== Ranked: cheapest wins (currently-resolved by prop count) ===");
    let mut ranked: Vec<(&String, usize)> = report
        .outcomes
        .iter()
        .filter_map(|(k, v)| match v {
            Outcome::Resolved { props } => Some((k, *props)),
            _ => None,
        })
        .collect();
    ranked.sort_by_key(|(_, p)| std::cmp::Reverse(*p));
    for (name, props) in ranked.iter().take(30) {
        println!("  {props:>5}  {name}");
    }
    if ranked.len() > 30 {
        println!("  … ({} more resolved)", ranked.len() - 30);
    }
    println!();

    println!("=== Empty resolves — parsed but produced no props ===");
    let empties: Vec<&String> = report
        .outcomes
        .iter()
        .filter_map(|(k, v)| matches!(v, Outcome::Empty).then_some(k))
        .collect();
    for name in empties.iter().take(30) {
        println!("  {name}");
    }
    if empties.len() > 30 {
        println!("  … ({} more empty)", empties.len() - 30);
    }
    println!();

    println!("=== Failures ===");
    let mut fails: Vec<(&String, &String)> = report
        .outcomes
        .iter()
        .filter_map(|(k, v)| match v {
            Outcome::Failed(m) => Some((k, m)),
            _ => None,
        })
        .collect();
    fails.sort();
    for (name, msg) in fails.iter().take(30) {
        println!("  {name}: {msg}");
    }
    if fails.len() > 30 {
        println!("  … ({} more failed)", fails.len() - 30);
    }
}
