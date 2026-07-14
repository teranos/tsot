//! CDDA palette resolver — with per-building parameter selection.
//!
//! A mapgen references palettes; a palette maps symbols → terrain/
//! furniture ids, nests other palettes, and defers values to
//! *parameters* rolled once per scope (CDDA's varied-but-consistent
//! trick; see doc/JSON/MAPGEN.md). We resolve every parameter from a
//! per-building **seed** (hash of the building's chunk), so two houses
//! pick different variant palettes / fence types / etc. but each is
//! internally consistent and identical on every peer. Weights are
//! ignored in favour of a uniform spread — CDDA's weighting would make
//! ~every house the "standard" variant, defeating the variety.
//!
//! Corpus is CC-BY-SA 3.0 CDDA content, fetched from a pinned release
//! at build time (not vendored). See assets/cdda/ATTRIBUTION.md.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::hash::wang_hash;

// Palette corpus embedded from the build-time CDDA tree (build.rs
// copies it out of the pinned release — never vendored in git).
// CC-BY-SA 3.0, CleverRaven / CDDA; see assets/cdda/ATTRIBUTION.md.
const FILES: &[&str] = &[
    include_str!(concat!(env!("OUT_DIR"), "/cdda/house_general_palette.json")),
    include_str!(concat!(env!("OUT_DIR"), "/cdda/common_parameters.json")),
    include_str!(concat!(env!("OUT_DIR"), "/cdda/roof_palette.json")),
    include_str!(concat!(env!("OUT_DIR"), "/cdda/house_variant_palette.json")),
    include_str!(concat!(env!("OUT_DIR"), "/cdda/house_survivor_palette.json")),
    include_str!(concat!(env!("OUT_DIR"), "/cdda/house_general_abandoned.json")),
];

struct Registry {
    /// palette id → its JSON object.
    by_id: HashMap<String, Value>,
    /// parameter name → its default value (a distribution).
    param_defs: HashMap<String, Value>,
}

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut by_id = HashMap::new();
        // Dedicated palette files: every id'd entry is a palette.
        for src in FILES {
            if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(src) {
                for e in entries {
                    if let Some(id) = e.get("id").and_then(Value::as_str) {
                        by_id.insert(id.to_string(), e);
                    }
                }
            }
        }
        // Palettes declared *inline* inside a building's own mapgen file
        // (e.g. the school's `school_palette`). CDDA registers every
        // `type: palette` object globally regardless of which file it's
        // in; mirror that so those buildings resolve.
        for src in crate::cdda::building::SHIPPED_MAPGEN {
            if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(src) {
                for e in entries {
                    if e.get("type").and_then(Value::as_str) == Some("palette")
                        && let Some(id) = e.get("id").and_then(Value::as_str)
                    {
                        by_id.insert(id.to_string(), e);
                    }
                }
            }
        }
        let mut param_defs = HashMap::new();
        for e in by_id.values() {
            if let Some(ps) = e.get("parameters").and_then(Value::as_object) {
                for (name, def) in ps {
                    if let Some(d) = def.get("default") {
                        param_defs.insert(name.clone(), d.clone());
                    }
                }
            }
        }
        Registry { by_id, param_defs }
    })
}

/// All candidate ids in a distribution value (weights dropped).
fn options(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Object(o) => o.get("distribution").map(options).unwrap_or_default(),
        Value::Array(a) => a
            .iter()
            .flat_map(|e| match e {
                // Weighted pair [id, weight] → the id.
                Value::Array(pair) => pair
                    .first()
                    .and_then(Value::as_str)
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default(),
                other => options(other),
            })
            .collect(),
        _ => vec![],
    }
}

/// A stable per-(building, parameter) hash.
fn param_seed(seed: u32, name: &str) -> u32 {
    let nh = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    wang_hash(seed as i32, nh as i32, 0x5A17_5EED)
}

/// Uniform per-building pick from a named parameter's options.
fn pick_param(name: &str, seed: u32, reg: &Registry) -> Option<String> {
    let opts = options(reg.param_defs.get(name)?);
    if opts.is_empty() {
        return None;
    }
    Some(opts[(param_seed(seed, name) as usize) % opts.len()].clone())
}

/// Resolve a terrain/furniture value to a concrete id.
fn resolve_value(v: &Value, seed: u32, reg: &Registry) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.iter().find_map(|e| resolve_value(e, seed, reg)),
        Value::Object(o) => {
            if let Some(p) = o.get("param").and_then(Value::as_str) {
                pick_param(p, seed, reg)
                    .or_else(|| o.get("fallback").and_then(Value::as_str).map(str::to_string))
            } else if let Some(sw) = o.get("switch") {
                let key = resolve_value(sw, seed, reg)?;
                o.get("cases")
                    .and_then(|c| c.get(&key))
                    .and_then(|c| resolve_value(c, seed, reg))
                    .or_else(|| sw.get("fallback").and_then(Value::as_str).map(str::to_string))
            } else if o.contains_key("distribution") {
                // Anonymous distribution — first option (no name to seed by).
                options(v).into_iter().next()
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve a palettes-list entry to a concrete palette id.
fn resolve_palette_ref(v: &Value, seed: u32, reg: &Registry) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => {
            if let Some(p) = o.get("param").and_then(Value::as_str) {
                pick_param(p, seed, reg)
            } else {
                options(v).into_iter().next()
            }
        }
        _ => None,
    }
}

fn merge_palette(
    id: &str,
    seed: u32,
    reg: &Registry,
    ter: &mut HashMap<char, String>,
    fur: &mut HashMap<char, String>,
    seen: &mut Vec<String>,
) {
    if id == "null_palette" || seen.iter().any(|s| s == id) {
        return;
    }
    seen.push(id.to_string());
    let Some(entry) = reg.by_id.get(id) else { return };
    if let Some(nested) = entry.get("palettes").and_then(Value::as_array) {
        for n in nested {
            if let Some(nid) = resolve_palette_ref(n, seed, reg) {
                merge_palette(&nid, seed, reg, ter, fur, seen);
            }
        }
    }
    for (map_key, out) in [("terrain", &mut *ter), ("furniture", &mut *fur)] {
        if let Some(obj) = entry.get(map_key).and_then(Value::as_object) {
            for (sym, val) in obj {
                if let (Some(ch), Some(id)) = (sym.chars().next(), resolve_value(val, seed, reg)) {
                    out.insert(ch, id);
                }
            }
        }
    }
}

/// Resolve a mapgen's `palettes` list into flat `char → id` maps, with
/// every parameter chosen from `seed`. Later palettes override earlier.
pub fn resolve(palettes: &[Value], seed: u32) -> (HashMap<char, String>, HashMap<char, String>) {
    let reg = registry();
    let mut ter = HashMap::new();
    let mut fur = HashMap::new();
    for p in palettes {
        if let Some(id) = resolve_palette_ref(p, seed, reg) {
            merge_palette(&id, seed, reg, &mut ter, &mut fur, &mut Vec::new());
        }
    }
    (ter, fur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_the_standard_house_vocabulary_through_nesting() {
        let (ter, fur) = resolve(&[json!("domestic_general_and_variant_palette")], 0);
        assert_eq!(fur.get(&'h').map(String::as_str), Some("f_chair"));
        assert_eq!(fur.get(&'f').map(String::as_str), Some("f_table"));
        assert!(
            ter.values().any(|v| v.contains("wall")),
            "expected some wall terrain in the resolved map"
        );
    }

    #[test]
    fn different_seeds_pick_different_variants() {
        // Across many seeds, the resolved furniture map should not be
        // identical every time — the per-building parameter pick varies.
        let base = resolve(&[json!("domestic_general_and_variant_palette")], 0).1;
        let differs = (1..40).any(|s| {
            let f = resolve(&[json!("domestic_general_and_variant_palette")], s).1;
            f != base
        });
        assert!(differs, "some seed should resolve a different variant");
    }

    #[test]
    fn unknown_palette_resolves_empty_not_panic() {
        let (ter, fur) = resolve(&[json!("no_such_palette")], 7);
        assert!(ter.is_empty() && fur.is_empty());
    }
}
