//! CDDA mapgen JSON shape + walkers over its polymorphic value trees.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub(crate) struct Entry {
    #[serde(default)]
    pub om_terrain: Value,
    pub object: Option<Obj>,
}

#[derive(Deserialize)]
pub(crate) struct Obj {
    #[serde(default)]
    pub rows: Vec<String>,
    #[serde(default)]
    pub terrain: HashMap<String, Value>,
    #[serde(default)]
    pub palettes: Vec<Value>,
}

/// Does an om_terrain value (a string, or a nested array of strings)
/// name this om_terrain?
pub(crate) fn om_matches(v: &Value, name: &str) -> bool {
    match v {
        Value::String(s) => s == name,
        Value::Array(a) => a.iter().any(|x| om_matches(x, name)),
        _ => false,
    }
}

/// First id string in a CDDA terrain/furniture value, which may be a
/// bare string, an array of ids, or [id, weight] pairs.
pub(crate) fn first_id(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s.as_str()),
        Value::Array(a) => a.iter().find_map(first_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_id_handles_string_and_weighted_array() {
        assert_eq!(first_id(&json!("t_wall_log")), Some("t_wall_log"));
        assert_eq!(
            first_id(&json!([["t_pavement", 10], "t_dirt"])),
            Some("t_pavement")
        );
        assert_eq!(first_id(&json!(5)), None);
    }

    #[test]
    fn om_matches_bare_and_nested() {
        assert!(om_matches(&json!("s_garage_1"), "s_garage_1"));
        assert!(om_matches(&json!(["s_garage_1"]), "s_garage_1"));
        assert!(!om_matches(&json!(["s_garage_2"]), "s_garage_1"));
    }
}
