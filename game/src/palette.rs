//! CDDA palette resolver.
//!
//! A mapgen references palettes (`"palettes": ["standard_domestic_..."]`);
//! a palette maps symbols → terrain/furniture ids, can nest other
//! palettes, and can defer values to *parameters* — values rolled once
//! per scope and reused (CDDA's trick for varied-but-consistent
//! buildings; see doc/JSON/MAPGEN.md). We don't roll: we resolve every
//! parameter to its **highest-weight default**, collapsing the whole
//! nested/parametrized structure into one flat `char → id` map the
//! importer can consume. (Swapping "top weight" for a per-building hash
//! pick is the future "varied houses, still identical per peer" step.)
//!
//! Corpus is CC-BY-SA 3.0 CDDA content under assets/cdda/palettes/.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const HOUSE_GENERAL: &str = include_str!("../assets/cdda/palettes/house_general_palette.json");
const COMMON_PARAMS: &str = include_str!("../assets/cdda/palettes/common_parameters.json");
const ROOF: &str = include_str!("../assets/cdda/palettes/roof_palette.json");

struct Registry {
    /// palette id → its JSON object.
    by_id: HashMap<String, Value>,
    /// parameter name → resolved default id (highest-weight option).
    params: HashMap<String, String>,
}

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| {
        let mut by_id = HashMap::new();
        for src in [HOUSE_GENERAL, COMMON_PARAMS, ROOF] {
            if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(src) {
                for e in entries {
                    if let Some(id) = e.get("id").and_then(Value::as_str) {
                        by_id.insert(id.to_string(), e);
                    }
                }
            }
        }
        // Collect every parameter's default across the whole corpus.
        let mut params = HashMap::new();
        for e in by_id.values() {
            if let Some(ps) = e.get("parameters").and_then(Value::as_object) {
                for (name, def) in ps {
                    if let Some(id) = def.get("default").and_then(top_of_distribution) {
                        params.insert(name.clone(), id);
                    }
                }
            }
        }
        Registry { by_id, params }
    })
}

/// Highest-weight id from a distribution value — `{distribution: [[id,w],…]}`,
/// a bare `[id,…]`, or a plain string.
fn top_of_distribution(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("distribution").and_then(top_of_distribution),
        Value::Array(a) => {
            // Weighted pairs [[id, w], …]?
            if a.iter().all(|e| e.is_array()) && !a.is_empty() {
                a.iter()
                    .filter_map(|e| {
                        let arr = e.as_array()?;
                        let id = arr.first()?.as_str()?.to_string();
                        let w = arr.get(1).and_then(Value::as_f64).unwrap_or(1.0);
                        Some((id, w))
                    })
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(id, _)| id)
            } else {
                // Bare list — first element.
                a.first().and_then(top_of_distribution)
            }
        }
        _ => None,
    }
}

/// Resolve a terrain/furniture value to a concrete id string.
fn resolve_value(v: &Value, reg: &Registry) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.iter().find_map(|e| resolve_value(e, reg)),
        Value::Object(o) => {
            if let Some(p) = o.get("param").and_then(Value::as_str) {
                reg.params
                    .get(p)
                    .cloned()
                    .or_else(|| o.get("fallback").and_then(Value::as_str).map(str::to_string))
            } else if let Some(sw) = o.get("switch") {
                let key = resolve_value(sw, reg)?;
                o.get("cases")
                    .and_then(|c| c.get(&key))
                    .and_then(|c| resolve_value(c, reg))
                    .or_else(|| sw.get("fallback").and_then(Value::as_str).map(str::to_string))
            } else if o.contains_key("distribution") {
                top_of_distribution(v).and_then(|id| resolve_value(&Value::String(id), reg))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Resolve a palettes-list entry (string, `{param}`, or `{distribution}`)
/// to a concrete palette id.
fn resolve_palette_ref(v: &Value, reg: &Registry) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => {
            if let Some(p) = o.get("param").and_then(Value::as_str) {
                reg.params.get(p).cloned()
            } else {
                top_of_distribution(v)
            }
        }
        _ => None,
    }
}

fn merge_palette(
    id: &str,
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
    // Nested palettes first (so this palette's own defs override them).
    if let Some(nested) = entry.get("palettes").and_then(Value::as_array) {
        for n in nested {
            if let Some(nid) = resolve_palette_ref(n, reg) {
                merge_palette(&nid, reg, ter, fur, seen);
            }
        }
    }
    for (map_key, out) in [("terrain", &mut *ter), ("furniture", &mut *fur)] {
        if let Some(obj) = entry.get(map_key).and_then(Value::as_object) {
            for (sym, val) in obj {
                if let (Some(ch), Some(id)) = (sym.chars().next(), resolve_value(val, reg)) {
                    out.insert(ch, id);
                }
            }
        }
    }
}

/// Resolve a mapgen's `palettes` list into flat `char → id` maps for
/// terrain and furniture. Later palettes override earlier ones.
pub fn resolve(palettes: &[Value]) -> (HashMap<char, String>, HashMap<char, String>) {
    let reg = registry();
    let mut ter = HashMap::new();
    let mut fur = HashMap::new();
    for p in palettes {
        if let Some(id) = resolve_palette_ref(p, reg) {
            merge_palette(&id, reg, &mut ter, &mut fur, &mut Vec::new());
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
        // domestic_general_and_variant_palette → variant param default →
        // migration → standard_domestic → …_no_items, where the 68-symbol
        // furniture vocabulary lives.
        let (ter, fur) = resolve(&[json!("domestic_general_and_variant_palette")]);
        assert_eq!(fur.get(&'h').map(String::as_str), Some("f_chair"));
        assert_eq!(fur.get(&'f').map(String::as_str), Some("f_table"));
        assert!(fur.contains_key(&'d'), "dresser symbol should resolve");
        // A wall terrain should resolve (via parametrized_walls_palette).
        assert!(
            ter.values().any(|v| v.contains("wall")),
            "expected some wall terrain in the resolved map"
        );
    }

    #[test]
    fn unknown_palette_resolves_empty_not_panic() {
        let (ter, fur) = resolve(&[json!("no_such_palette")]);
        assert!(ter.is_empty() && fur.is_empty());
    }
}
