//! Lua parsing for the card schema. Reads `.lua` card files (and the
//! embedded card corpus) and turns them into the schema types defined
//! one level up in `card.rs`. Extracted so the schema reader doesn't
//! have to scroll past ~800 lines of mlua plumbing.

use include_dir::{include_dir, Dir};
use mlua::{Function, Lua, Table, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::*;

static EMBEDDED_CARDS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/cards");

fn normalize_color(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Lua-side type strings translate to `(kind, timing)`. "instant" and
/// "sorcery" are both Spell kind with different timing; "spell" (legacy
/// alias) is treated as sorcery timing.
fn parse_type(s: &str) -> Result<(CardType, Option<Timing>), String> {
    match s.to_ascii_lowercase().as_str() {
        "" => Ok((CardType::Unspecified, None)),
        "creature" => Ok((CardType::Creature, None)),
        "instant" => Ok((CardType::Spell, Some(Timing::Instant))),
        "sorcery" | "spell" => Ok((CardType::Spell, Some(Timing::Sorcery))),
        "artifact" => Ok((CardType::Artifact, None)),
        "environment" => Ok((CardType::Environment, None)),
        "mutation" => Ok((CardType::Mutation, None)),
        "symbol" => Ok((CardType::Symbol, None)),
        other => Err(format!("unknown type: {other}")),
    }
}

fn parse_source(s: &str) -> Result<CostSource, String> {
    match s.to_ascii_lowercase().as_str() {
        "hand" => Ok(CostSource::Hand),
        "mill" => Ok(CostSource::Mill),
        "graveyard" => Ok(CostSource::Graveyard),
        "sacrifice" => Ok(CostSource::Sacrifice),
        "self" => Ok(CostSource::SelfExile),
        "attached" => Ok(CostSource::Attached),
        other => Err(format!("unknown cost source: {other}")),
    }
}

fn read_string_vec(t: &Table, key: &str) -> mlua::Result<Vec<String>> {
    match t.get::<Value>(key)? {
        Value::Nil => Ok(Vec::new()),
        Value::Table(tt) => tt.sequence_values::<String>().collect(),
        other => Err(mlua::Error::runtime(format!(
            "field {key:?} must be a list of strings, got {other:?}"
        ))),
    }
}

/// `colors` accepts two shapes, mirroring `symbols`:
/// - Array: `colors = {"green", "red"}` → unordered identity list.
/// - Map:   `colors = { C = "green", T = "red" }` → identity + slot.
///
/// Mixed forms (some integer keys, some string keys) are rejected.
/// Slot form additionally rejects duplicate color values: each color
/// owns exactly one slot. Returns `(colors_vec, color_slots_map)`.
fn read_color_vec(t: &Table) -> mlua::Result<(Vec<String>, std::collections::BTreeMap<crate::card::Slot, String>)> {
    use std::collections::{BTreeMap, BTreeSet};
    let raw: Table = match t.get::<Value>("colors")? {
        Value::Nil => return Ok((Vec::new(), BTreeMap::new())),
        Value::Table(tt) => tt,
        other => {
            return Err(mlua::Error::runtime(format!(
                "card.colors must be a list or slot-keyed table, got {other:?}"
            )))
        }
    };
    let mut arr: Vec<String> = Vec::new();
    let mut map: BTreeMap<crate::card::Slot, String> = BTreeMap::new();
    let mut form: Option<&'static str> = None;
    for pair in raw.pairs::<Value, String>() {
        let (k, v) = pair?;
        let v = normalize_color(&v);
        match k {
            Value::Integer(_) => {
                if form == Some("map") {
                    return Err(mlua::Error::runtime(
                        "card.colors cannot mix array entries with slot-keyed entries".to_string()
                    ));
                }
                form = Some("array");
                arr.push(v);
            }
            Value::String(ks) => {
                if form == Some("array") {
                    return Err(mlua::Error::runtime(
                        "card.colors cannot mix array entries with slot-keyed entries".to_string()
                    ));
                }
                form = Some("map");
                let name = ks.to_str()?.to_string();
                let slot = name.parse::<crate::card::Slot>()
                    .map_err(mlua::Error::runtime)?;
                map.insert(slot, v);
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "card.colors keys must be integers or slot names; got {other:?}"
                )))
            }
        }
    }
    if !map.is_empty() {
        // Slot form. Reject duplicate colors — each color owns one slot.
        let mut seen: BTreeSet<&String> = BTreeSet::new();
        for c in map.values() {
            if !seen.insert(c) {
                return Err(mlua::Error::runtime(format!(
                    "card.colors slot form has duplicate color {c:?}; each color may occupy at most one slot"
                )));
            }
        }
        // Derive the identity Vec from the map values in canonical Slot::ALL
        // order so anything reading `colors` sees a stable list.
        let derived: Vec<String> = crate::card::Slot::ALL
            .iter()
            .filter_map(|s| map.get(s).cloned())
            .collect();
        Ok((derived, map))
    } else {
        Ok((arr, map))
    }
}

fn read_cost(t: &Table) -> mlua::Result<Vec<CostComponent>> {
    let raw: Table = match t.get::<Value>("cost")? {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(tt) => tt,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `cost` must be a list, got {other:?}"
            )))
        }
    };
    let mut out = Vec::new();
    for item in raw.sequence_values::<Table>() {
        let item = item?;
        let amount = item.get::<Option<i32>>("amount")?.unwrap_or(0);
        let is_x = item.get::<Option<bool>>("is_x")?.unwrap_or(false);
        let source_s = item.get::<String>("source")?;
        let source = parse_source(&source_s).map_err(mlua::Error::runtime)?;
        let kind = match item.get::<Option<String>>("kind")? {
            None => None,
            Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
        };
        out.push(CostComponent {
            amount,
            source,
            is_x,
            kind,
        });
    }
    Ok(out)
}

fn read_activated(t: &Table) -> mlua::Result<Vec<ActivatedAbility>> {
    let raw: Table = match t.get::<Value>("activated")? {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(tt) => tt,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `activated` must be a list, got {other:?}"
            )))
        }
    };
    let mut out = Vec::new();
    for item in raw.sequence_values::<Table>() {
        let item = item?;
        // Two shapes supported for `cost`:
        //   1. String shorthand: `cost = "tap"` → tap-only.
        //   2. List of components: `cost = {{source = "...", amount = N}}` →
        //      one or more cost components, possibly including a tap
        //      pseudo-component `{source = "tap"}` (no amount).
        let cost_value: Value = item.get("cost")?;
        let (cost_tap, cost_components) = match cost_value {
            Value::String(s) => {
                let s = s.to_str()?.to_ascii_lowercase();
                if s == "tap" || s == "t" {
                    (true, Vec::new())
                } else {
                    return Err(mlua::Error::runtime(format!(
                        "activation cost string {s:?} not recognized (expected \"tap\")"
                    )));
                }
            }
            Value::Table(tt) => {
                let mut tap = false;
                let mut comps: Vec<CostComponent> = Vec::new();
                for comp in tt.sequence_values::<Table>() {
                    let comp = comp?;
                    let src_s: String = comp.get("source")?;
                    let lowered = src_s.to_ascii_lowercase();
                    if lowered == "tap" || lowered == "t" {
                        tap = true;
                        continue;
                    }
                    let amount = comp.get::<Option<i32>>("amount")?.unwrap_or(0);
                    let is_x = comp.get::<Option<bool>>("is_x")?.unwrap_or(false);
                    let source = parse_source(&lowered).map_err(mlua::Error::runtime)?;
                    let kind = match comp.get::<Option<String>>("kind")? {
                        None => None,
                        Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
                    };
                    comps.push(CostComponent {
                        amount,
                        source,
                        is_x,
                        kind,
                    });
                }
                (tap, comps)
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "activation cost must be a string or a list, got {other:?}"
                )))
            }
        };
        let text = item.get::<Option<String>>("text")?.unwrap_or_default();
        let timing_s = item
            .get::<Option<String>>("timing")?
            .unwrap_or_else(|| "sorcery".to_string());
        let timing = match timing_s.to_ascii_lowercase().as_str() {
            "instant" => Timing::Instant,
            "sorcery" => Timing::Sorcery,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown activation timing: {other:?} (must be \"instant\" or \"sorcery\")"
                )))
            }
        };
        let validate: Option<Function> = match item.get::<Value>("validate")? {
            Value::Nil => None,
            Value::Function(f) => Some(f),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "activation `validate` must be a function, got {other:?}"
                )))
            }
        };
        let effect: Function = item.get("effect")?;
        let target: Option<Target> = match item.get::<Option<String>>("target")? {
            None => None,
            Some(s) => match s.to_ascii_lowercase().as_str() {
                "chain" => Some(Target::Chain),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "unknown activation target category: {other:?}"
                    )))
                }
            },
        };
        out.push(ActivatedAbility {
            cost_tap,
            cost_components,
            text,
            timing,
            validate,
            target,
            effect,
        });
    }
    Ok(out)
}

fn read_handlers(t: &Table) -> mlua::Result<BTreeMap<EventName, Function>> {
    let mut out = BTreeMap::new();
    for ev in EventName::ALL {
        match t.get::<Value>(ev.lua_key())? {
            Value::Nil => {}
            Value::Function(f) => {
                out.insert(ev, f);
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "field `{}` must be a function, got {other:?}",
                    ev.lua_key()
                )))
            }
        }
    }
    Ok(out)
}

fn read_stats(t: &Table) -> mlua::Result<Option<Stats>> {
    match t.get::<Value>("stats")? {
        Value::Nil => Ok(None),
        Value::Table(s) => {
            let x = s.get::<Option<f32>>("x")?.unwrap_or(0.0);
            let y = s.get::<Option<f32>>("y")?.unwrap_or(0.0);
            Ok(Some(Stats { x, y }))
        }
        other => Err(mlua::Error::runtime(format!(
            "field `stats` must be a table, got {other:?}"
        ))),
    }
}

fn read_static(t: &Table) -> mlua::Result<Option<StaticDef>> {
    let static_val = t.get::<Value>("static")?;
    let static_t = match static_val {
        Value::Nil => return Ok(None),
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `static` must be a table, got {other:?}"
            )))
        }
    };
    let affects = match static_t.get::<Value>("affects")? {
        Value::Nil => StaticAffects::default(),
        Value::Table(a) => {
            let subtypes = match a.get::<Value>("subtypes")? {
                Value::Nil => Vec::new(),
                Value::Table(st) => st
                    .sequence_values::<String>()
                    .collect::<mlua::Result<Vec<_>>>()?
                    .into_iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "static.affects.subtypes must be a table, got {other:?}"
                    )))
                }
            };
            let colors = match a.get::<Value>("colors")? {
                Value::Nil => Vec::new(),
                Value::Table(ct) => ct
                    .sequence_values::<String>()
                    .collect::<mlua::Result<Vec<_>>>()?
                    .into_iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "static.affects.colors must be a table, got {other:?}"
                    )))
                }
            };
            let controller = match a.get::<Option<String>>("controller")? {
                None => None,
                Some(s) => match s.to_ascii_lowercase().as_str() {
                    "owner" => Some(StaticController::Owner),
                    "opponent" => Some(StaticController::Opponent),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.affects.controller must be 'owner' or 'opponent', got '{other}'"
                        )))
                    }
                },
            };
            let exclude_self = a.get::<Option<bool>>("exclude_self")?.unwrap_or(false);
            let scope = match a.get::<Option<String>>("scope")? {
                None => StaticScope::Board,
                Some(s) => match s.to_ascii_lowercase().as_str() {
                    "board" => StaticScope::Board,
                    "attached_host" => StaticScope::AttachedHost,
                    "source_only" => StaticScope::SourceOnly,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.affects.scope must be 'board', 'attached_host', or 'source_only', got '{other}'"
                        )))
                    }
                },
            };
            let kind = match a.get::<Option<String>>("kind")? {
                None => None,
                Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
            };
            let has_keyword = a
                .get::<Option<String>>("has_keyword")?
                .map(|s| s.to_ascii_lowercase());
            StaticAffects {
                subtypes,
                colors,
                controller,
                exclude_self,
                scope,
                kind,
                has_keyword,
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.affects must be a table, got {other:?}"
            )))
        }
    };
    let (modifier_x, modifier_y, modifier_keyword, granted_colors, granted_face, makes_host_colorless, suppresses_host_abilities) =
        match static_t.get::<Value>("modifier")? {
            Value::Nil => (
                ModifierValue::Fixed(0.0),
                ModifierValue::Fixed(0.0),
                None,
                Vec::new(),
                Vec::new(),
                false,
                false,
            ),
            Value::Table(m) => {
                let x = read_modifier_value(m.get::<Value>("x")?)?;
                let y = read_modifier_value(m.get::<Value>("y")?)?;
                let keyword = m
                    .get::<Option<String>>("keyword")?
                    .map(|s| s.to_ascii_lowercase());
                let colors: Vec<String> = match m.get::<Option<Value>>("colors")? {
                    Some(Value::Table(t)) => {
                        let mut out = Vec::new();
                        for s in t.sequence_values::<String>() {
                            out.push(s?.to_ascii_lowercase());
                        }
                        out
                    }
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "static.modifier.colors must be a sequence of strings, got {other:?}"
                        )))
                    }
                    None => Vec::new(),
                };
                let face: Vec<String> = match m.get::<Option<Value>>("face")? {
                    Some(Value::Table(t)) => {
                        let mut out = Vec::new();
                        for s in t.sequence_values::<String>() {
                            out.push(s?.to_ascii_lowercase());
                        }
                        out
                    }
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "static.modifier.face must be a sequence of strings, got {other:?}"
                        )))
                    }
                    None => Vec::new(),
                };
                let colorless = m.get::<Option<bool>>("colorless")?.unwrap_or(false);
                let suppresses = m
                    .get::<Option<bool>>("suppresses_abilities")?
                    .unwrap_or(false);
                (x, y, keyword, colors, face, colorless, suppresses)
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "static.modifier must be a table, got {other:?}"
                )))
            }
        };
    let condition = match static_t.get::<Value>("condition")? {
        Value::Nil => None,
        Value::Table(c) => Some(read_condition(&c)?),
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.condition must be a table, got {other:?}"
            )))
        }
    };
    let restrictions = match static_t.get::<Value>("restrictions")? {
        Value::Nil => Vec::new(),
        Value::Table(r) => {
            let mut out = Vec::new();
            for s in r.sequence_values::<String>() {
                let s = s?;
                let restriction = match s.to_ascii_lowercase().as_str() {
                    "cannot_attack" => Restriction::CannotAttack,
                    "cannot_be_cost_paid" => Restriction::CannotBeCostPaid,
                    "cannot_be_attached_to" => Restriction::CannotBeAttachedTo,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.restrictions entry must be 'cannot_attack', 'cannot_be_cost_paid', or 'cannot_be_attached_to', got '{other}'"
                        )))
                    }
                };
                out.push(restriction);
            }
            out
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.restrictions must be a sequence of strings, got {other:?}"
            )))
        }
    };
    let cost_modifiers = match static_t.get::<Value>("cost_modifiers")? {
        Value::Nil => Vec::new(),
        Value::Table(t) => {
            let mut out = Vec::new();
            for item in t.sequence_values::<Table>() {
                let item = item?;
                let source_s: String = item.get("source")?;
                let source = parse_source(&source_s).map_err(mlua::Error::runtime)?;
                let amount = item.get::<Option<i32>>("amount")?.unwrap_or(1);
                out.push(CostModifier { source, amount });
            }
            out
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.cost_modifiers must be a sequence of tables, got {other:?}"
            )))
        }
    };
    // Phase 3: optional `granted_activated` field declares a single
    // activated ability that matching candidates gain. Same Lua shape
    // as a card-level `activated[1]` entry: { cost, text, timing,
    // effect, optional validate }.
    let granted_activated = match static_t.get::<Value>("granted_activated")? {
        Value::Nil => None,
        Value::Table(t) => Some(parse_one_activated_entry(t)?),
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.granted_activated must be a table, got {other:?}"
            )))
        }
    };
    Ok(Some(StaticDef {
        affects,
        modifier_x,
        modifier_y,
        modifier_keyword,
        condition,
        restrictions,
        cost_modifiers,
        granted_activated,
        granted_colors,
        granted_face,
        makes_host_colorless,
        suppresses_host_abilities,
    }))
}

fn parse_one_activated_entry(item: Table) -> mlua::Result<ActivatedAbility> {
    let cost_value: Value = item.get("cost")?;
    let (cost_tap, cost_components) = match cost_value {
        Value::String(s) => {
            let s = s.to_str()?.to_ascii_lowercase();
            if s == "tap" || s == "t" {
                (true, Vec::new())
            } else {
                return Err(mlua::Error::runtime(format!(
                    "granted_activated cost string {s:?} not recognized (expected \"tap\")"
                )));
            }
        }
        Value::Table(tt) => {
            let mut tap = false;
            let mut comps: Vec<CostComponent> = Vec::new();
            for comp in tt.sequence_values::<Table>() {
                let comp = comp?;
                let src_s: String = comp.get("source")?;
                let lowered = src_s.to_ascii_lowercase();
                if lowered == "tap" || lowered == "t" {
                    tap = true;
                    continue;
                }
                let amount = comp.get::<Option<i32>>("amount")?.unwrap_or(0);
                let is_x = comp.get::<Option<bool>>("is_x")?.unwrap_or(false);
                let source = parse_source(&lowered).map_err(mlua::Error::runtime)?;
                let kind = match comp.get::<Option<String>>("kind")? {
                    None => None,
                    Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
                };
                comps.push(CostComponent {
                    amount,
                    source,
                    is_x,
                    kind,
                });
            }
            (tap, comps)
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated cost must be a string or list, got {other:?}"
            )))
        }
    };
    let text = item.get::<Option<String>>("text")?.unwrap_or_default();
    let timing_s = item
        .get::<Option<String>>("timing")?
        .unwrap_or_else(|| "sorcery".to_string());
    let timing = match timing_s.to_ascii_lowercase().as_str() {
        "instant" => Timing::Instant,
        "sorcery" => Timing::Sorcery,
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated timing {other:?} must be \"instant\" or \"sorcery\""
            )))
        }
    };
    let validate: Option<Function> = match item.get::<Value>("validate")? {
        Value::Nil => None,
        Value::Function(f) => Some(f),
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated validate must be a function, got {other:?}"
            )))
        }
    };
    let effect: Function = item.get("effect")?;
    let target: Option<Target> = match item.get::<Option<String>>("target")? {
        None => None,
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "chain" => Some(Target::Chain),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown granted_activated target category: {other:?}"
                )))
            }
        },
    };
    Ok(ActivatedAbility {
        cost_tap,
        cost_components,
        text,
        timing,
        validate,
        target,
        effect,
    })
}

/// Parse a `ModifierValue` from a Lua value. Accepts either:
/// - Nil → `Fixed(0)` (back-compat for omitted entries)
/// - Integer N → `Fixed(N)`
/// - String "attached" → `AttachedCount`
/// - String "N*attached" (e.g., "2*attached") → `AttachedCountScaled(N)`
/// - String "attached:type:<kind>" → `AttachedCountByKind(kind)`
/// - String "attached:<color>" → `AttachedCountByColor(color)` (fallback)
fn read_modifier_value(v: Value) -> mlua::Result<ModifierValue> {
    match v {
        Value::Nil => Ok(ModifierValue::Fixed(0.0)),
        Value::Integer(n) => Ok(ModifierValue::Fixed(n as f32)),
        Value::Number(n) => Ok(ModifierValue::Fixed(n as f32)),
        Value::Table(t) => {
            // Two shapes:
            //   1. Scaled form: `{scale = -0.25, count = "board:face:shiny"}`
            //      → Scaled(scale, inner). Detected by the presence of
            //      string key `scale`.
            //   2. Sum form: array of values `{-0.5, {scale=-0.25, count=...}}`
            //      → Sum([Fixed(-0.5), Scaled(-0.25, ...)]).
            if let Ok(Some(scale)) = t.get::<Option<f32>>("scale") {
                let count_val: Value = t.get("count")?;
                let inner = read_modifier_value(count_val)?;
                return Ok(ModifierValue::Scaled(scale, Box::new(inner)));
            }
            // Sequence form → Sum.
            let mut parts: Vec<ModifierValue> = Vec::new();
            for item in t.sequence_values::<Value>() {
                parts.push(read_modifier_value(item?)?);
            }
            if parts.is_empty() {
                return Err(mlua::Error::runtime(
                    "modifier value table must have `scale`+`count` keys or a non-empty sequence",
                ));
            }
            Ok(ModifierValue::Sum(parts))
        }
        Value::String(s) => {
            let raw = s.to_str()?.to_string();
            let lower = raw.to_ascii_lowercase().replace(' ', "");
            if lower == "attached" {
                return Ok(ModifierValue::AttachedCount);
            }
            if lower == "board" {
                return Ok(ModifierValue::BoardCount);
            }
            if let Some(face) = lower.strip_prefix("board:face:") {
                return Ok(ModifierValue::BoardCountByFace(face.to_string()));
            }
            if lower == "board_types" {
                return Ok(ModifierValue::BoardTypeCount);
            }
            if lower == "hands" || lower == "hand" {
                return Ok(ModifierValue::HandCount);
            }
            // `N*attached` form (e.g., "2*attached", "3*attached").
            if let Some((mul_str, tail)) = lower.split_once('*') {
                if tail == "attached" {
                    let n: i32 = mul_str.parse().map_err(|_| {
                        mlua::Error::runtime(format!(
                            "modifier value 'N*attached' multiplier must be an integer, got {mul_str:?}"
                        ))
                    })?;
                    return Ok(ModifierValue::AttachedCountScaled(n));
                }
            }
            if let Some(kind_str) = lower.strip_prefix("attached:type:") {
                let (kind, _) = parse_type(kind_str).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "modifier value 'attached:type:<kind>' has unknown kind: {e}"
                    ))
                })?;
                return Ok(ModifierValue::AttachedCountByKind(kind));
            }
            if let Some(rest) = lower.strip_prefix("attached:") {
                return Ok(ModifierValue::AttachedCountByColor(rest.to_string()));
            }
            Err(mlua::Error::runtime(format!(
                "modifier value string must be 'attached', 'N*attached', 'attached:<color>', or 'attached:type:<kind>', got {raw:?}"
            )))
        }
        other => Err(mlua::Error::runtime(format!(
            "modifier value must be integer or string, got {other:?}"
        ))),
    }
}

fn read_condition(c: &Table) -> mlua::Result<StaticCondition> {
    let kind: String = c.get("kind")?;
    match kind.to_ascii_lowercase().as_str() {
        "owner_graveyard_size" => {
            let min = c.get::<i64>("min")?.max(0) as usize;
            Ok(StaticCondition::OwnerGraveyardSize { min })
        }
        "owner_graveyard_non_creatures" => {
            let min = c.get::<i64>("min")?.max(0) as usize;
            Ok(StaticCondition::OwnerGraveyardNonCreatures { min })
        }
        "deck_top_symbol_matches_attached" => Ok(StaticCondition::DeckTopSymbolMatchesAttached),
        other => Err(mlua::Error::runtime(format!(
            "static.condition.kind must be one of 'owner_graveyard_size', 'owner_graveyard_non_creatures', 'deck_top_symbol_matches_attached'; got '{other}'"
        ))),
    }
}

/// Parse a single Lua table into a `Card`. Handles every field except
/// `variants` (which lives at the file level — see `load_card`). Reused
/// by both the base-card path and the per-variant merged-table path.
fn parse_card_table(table: &Table) -> mlua::Result<Card> {
    let id: String = table.get("id")?;
    let name = table.get::<Option<String>>("name")?.unwrap_or_default();
    // `symbols` accepts both forms:
    //   - Array: `symbols = {"꩜", "≡"}`  → unpositioned, spirals from C.
    //   - Map:   `symbols = { C = "꩜", TR = "≡" }`  → per-slot placement.
    // We disambiguate by peeking at the first key: integer = array,
    // string = map. The two forms are mutually exclusive.
    let (symbols, symbol_slots): (Vec<String>, std::collections::BTreeMap<crate::card::Slot, String>) =
        match table.get::<Option<Value>>("symbols")? {
            Some(Value::Table(t)) => {
                let mut arr = Vec::new();
                let mut map = std::collections::BTreeMap::<crate::card::Slot, String>::new();
                let mut form: Option<&'static str> = None;
                for pair in t.clone().pairs::<Value, String>() {
                    let (k, v) = pair?;
                    match k {
                        Value::Integer(_) => {
                            if form == Some("map") {
                                return Err(mlua::Error::runtime(
                                    "card.symbols cannot mix array entries with slot-keyed entries".to_string()
                                ));
                            }
                            form = Some("array");
                            arr.push(v);
                        }
                        Value::String(ks) => {
                            if form == Some("array") {
                                return Err(mlua::Error::runtime(
                                    "card.symbols cannot mix array entries with slot-keyed entries".to_string()
                                ));
                            }
                            form = Some("map");
                            let name = ks.to_str()?.to_string();
                            let slot = name.parse::<crate::card::Slot>()
                                .map_err(mlua::Error::runtime)?;
                            map.insert(slot, v);
                        }
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "card.symbols keys must be integers or slot names; got {other:?}"
                            )))
                        }
                    }
                }
                // If we got map entries, derive the symbols Vec in
                // canonical Slot::ALL order so anything reading the
                // array sees a stable glyph list.
                if !map.is_empty() {
                    let derived: Vec<String> = crate::card::Slot::ALL
                        .iter()
                        .filter_map(|s| map.get(s).cloned())
                        .collect();
                    (derived, map)
                } else {
                    (arr, map)
                }
            }
            Some(other) => {
                return Err(mlua::Error::runtime(format!(
                    "card.symbols must be a sequence or slot-keyed table, got {other:?}"
                )))
            }
            None => match table.get::<Option<String>>("symbol")? {
                Some(s) if !s.is_empty() => (vec![s], std::collections::BTreeMap::new()),
                _ => (Vec::new(), std::collections::BTreeMap::new()),
            },
    };
    // Array-form fallback: if symbol_slots is empty but symbols are
    // declared, assign each glyph to a slot per the SLOTS.md canonical
    // spiral (C, U, UR, R, DR, D, DL, L, UL, TL, T, TR, BR, B, BL).
    // This makes every loaded card carry positional symbol data — the
    // map form is just explicit-placement opt-in.
    let symbol_slots = if symbol_slots.is_empty() && !symbols.is_empty() {
        let mut filled = std::collections::BTreeMap::<crate::card::Slot, String>::new();
        for (glyph, slot) in symbols.iter().zip(crate::card::Slot::SPIRAL.iter()) {
            filled.insert(*slot, glyph.clone());
        }
        if symbols.len() > crate::card::Slot::SPIRAL.len() {
            return Err(mlua::Error::runtime(format!(
                "card `{id}`: declares {} symbols, exceeds the 15-slot grid",
                symbols.len()
            )));
        }
        filled
    } else {
        symbol_slots
    };
    // Symbols must be unicode glyphs (single non-ASCII codepoint each),
    // never ASCII shorthand codes like "ax". The engine matches symbols
    // by string equality, so allowing ASCII would silently fork the
    // vocabulary — Amsterdam-City's `⨳` count would miss any card
    // declaring `"ix"`. See CLAUDE.md.
    for s in &symbols {
        let mut chars = s.chars();
        let first = chars.next();
        let extra = chars.next();
        match (first, extra) {
            (Some(c), None) if !c.is_ascii() => {}
            (Some(_), Some(_)) => {
                return Err(mlua::Error::runtime(format!(
                    "card `{id}`: symbol {s:?} must be exactly one unicode codepoint, not a multi-char code"
                )))
            }
            _ => {
                return Err(mlua::Error::runtime(format!(
                    "card `{id}`: symbol {s:?} must be a non-ASCII unicode glyph (e.g. ꩜ ⨳ ⋈ ⊨ ≡), not ASCII shorthand"
                )))
            }
        }
    }
    let kind_s = table.get::<Option<String>>("type")?.unwrap_or_default();
    let (kind, timing) = parse_type(&kind_s).map_err(mlua::Error::runtime)?;
    let subtypes = read_string_vec(table, "subtypes")?;
    let cannot_block_subtypes = read_string_vec(table, "cannot_block_subtypes")?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let can_block_subtypes = read_string_vec(table, "can_block_subtypes")?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let abilities = read_string_vec(table, "abilities")?;
    let flavor = table.get::<Option<String>>("flavor")?.unwrap_or_default();
    let frame = table.get::<Option<String>>("frame")?.filter(|s| !s.is_empty());
    let face = read_string_vec(table, "face")?;
    // Positional holes — `holes = {"TL", "C", ...}` parsed into Vec<Slot>.
    let holes: Vec<crate::card::Slot> = match table.get::<Option<Value>>("holes")? {
        Some(Value::Table(t)) => {
            let mut out = Vec::new();
            for s in t.sequence_values::<String>() {
                let name = s?;
                let slot = name.parse::<crate::card::Slot>()
                    .map_err(mlua::Error::runtime)?;
                if !out.contains(&slot) {
                    out.push(slot);
                }
            }
            out
        }
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "card.holes must be a sequence of slot names (e.g. {{\"TL\", \"C\"}}), got {other:?}"
            )))
        }
        None => Vec::new(),
    };
    // C.13: a transparent-frame card has no symbols. The symbol-search
    // routine looks past it, so it can't carry one itself.
    if frame.as_deref() == Some("transparent") && !symbols.is_empty() {
        return Err(mlua::Error::runtime(format!(
            "card `{id}`: transparent-frame cards cannot declare symbols (got {symbols:?})"
        )));
    }
    let (colors, color_slots) = read_color_vec(table)?;
    let cost = read_cost(table)?;
    let stats = read_stats(table)?;
    let static_def = read_static(table)?;
    let handlers = read_handlers(table)?;
    let activated = read_activated(table)?;
    let gy_hand_substitute = table
        .get::<Option<bool>>("gy_hand_substitute")?
        .unwrap_or(false);
    let allow_x_zero = table
        .get::<Option<bool>>("allow_x_zero")?
        .unwrap_or(false);
    let target = match table.get::<Option<String>>("target")? {
        None => None,
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "chain" => Some(Target::Chain),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown target category: {other:?}"
                )));
            }
        },
    };
    Ok(Card {
        id,
        name,
        colors,
        kind,
        timing,
        subtypes,
        cannot_block_subtypes,
        can_block_subtypes,
        symbols,
        frame,
        holes,
        symbol_slots,
        color_slots,
        face,
        cost,
        abilities,
        flavor,
        stats,
        static_def,
        handlers,
        gy_hand_substitute,
        allow_x_zero,
        activated,
        target,
        is_variant: false,
        variant_of: None,
    })
}

/// Load a card .lua file. Returns the base card followed by any
/// variant cards declared in the file's `variants = { [key] = { ... }
/// }` table. Each variant id is `{base_id}-{key}`. The variant table
/// REPLACES top-level fields wholesale (no deep merge) — to tweak a
/// single ability, copy the whole `activated` array into the variant
/// with the tweak. Variants get `is_variant = true` and
/// `variant_of = Some(base_id)` so `main.rs::playable_pool` can
/// exclude them and `tsot balance-probe` can pick them up.
pub fn load_card(lua: &Lua, path: &Path) -> mlua::Result<Vec<Card>> {
    let source = fs::read_to_string(path).map_err(mlua::Error::external)?;
    let chunk_name = path.display().to_string();
    load_card_from_source(lua, &source, &chunk_name)
}

fn load_card_from_source(lua: &Lua, source: &str, chunk_name: &str) -> mlua::Result<Vec<Card>> {
    let value: Value = lua.load(source).set_name(chunk_name.to_string()).eval()?;
    let table = match value {
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "card file must return a table, got {other:?}"
            )))
        }
    };

    let base = parse_card_table(&table)?;
    let base_id = base.id.clone();

    let variants_table: Option<Table> = table.get("variants")?;
    let mut out: Vec<Card> = vec![base];
    if let Some(vt) = variants_table {
        // Snapshot the base table's keys ONCE so we can replay them
        // into a merged table per variant. We skip `variants` itself
        // to avoid recursion.
        let mut base_pairs: Vec<(Value, Value)> = Vec::new();
        for pair in table.pairs::<Value, Value>() {
            let (k, v) = pair?;
            if let Value::String(ks) = &k {
                if ks.to_str()? == "variants" {
                    continue;
                }
            }
            base_pairs.push((k, v));
        }
        for pair in vt.pairs::<String, Table>() {
            let (key, override_table) = pair?;
            // Build a merged Lua table: base keys, then variant
            // overrides on top. Top-level fields are replaced
            // wholesale; nested fields are not deep-merged.
            let merged = lua.create_table()?;
            for (k, v) in &base_pairs {
                merged.set(k.clone(), v.clone())?;
            }
            for p in override_table.pairs::<Value, Value>() {
                let (k, v) = p?;
                merged.set(k, v)?;
            }
            // Force the variant id; the base's `id` field carried
            // through the base_pairs copy is the wrong one to keep.
            let variant_id = format!("{base_id}-{key}");
            merged.set("id", variant_id.clone())?;
            let mut variant = parse_card_table(&merged)?;
            variant.is_variant = true;
            variant.variant_of = Some(base_id.clone());
            out.push(variant);
        }
    }
    Ok(out)
}

pub fn load_cards_dir(lua: &Lua, dir: &Path) -> mlua::Result<Vec<Card>> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(mlua::Error::external)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    entries.sort();
    let mut all: Vec<Card> = Vec::new();
    for p in &entries {
        all.extend(load_card(lua, p)?);
    }
    Ok(all)
}

pub fn load_cards_embedded(lua: &Lua) -> mlua::Result<Vec<Card>> {
    let mut files: Vec<_> = EMBEDDED_CARDS
        .files()
        .filter(|f| f.path().extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    files.sort_by_key(|f| f.path().to_path_buf());
    let mut all: Vec<Card> = Vec::new();
    for f in &files {
        let source = f
            .contents_utf8()
            .ok_or_else(|| mlua::Error::runtime(format!("non-utf8 card: {}", f.path().display())))?;
        let chunk_name = f.path().display().to_string();
        all.extend(load_card_from_source(lua, source, &chunk_name)?);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pale Apparition is the spec card for fractional stats — see
    /// cards/pale-apparition.lua and RULES.md B.2 / B.7 / B.8. The
    /// loader must preserve `0.5` exactly; truncating to `0` is the
    /// pre-refactor failure mode that motivated this whole branch.
    #[test]
    fn pale_apparition_loads_with_fractional_stats() {
        let lua = Lua::new();
        let path = Path::new("cards/pale-apparition.lua");
        let cards = load_card(&lua, path).expect("load pale-apparition");
        let card = cards.iter().find(|c| c.id == "pale-apparition").unwrap();
        let stats = card.stats.expect("pale-apparition has stats");
        assert_eq!(stats.x, 0.5, "X must round-trip as 0.5, not truncate");
        assert_eq!(stats.y, 1.0);
    }

    #[test]
    fn colors_list_form_populates_only_colors() {
        let card = load_card_from_lua(r#"
            return {
                id = "under-test",
                type = "creature",
                colors = {"green", "red"},
                stats = {x = 1, y = 1},
            }
        "#);
        assert_eq!(card.colors, vec!["green", "red"]);
        assert!(card.color_slots.is_empty(), "list form must leave color_slots empty");
    }

    #[test]
    fn colors_slot_form_populates_both_colors_and_color_slots() {
        let card = load_card_from_lua(r#"
            return {
                id = "under-test",
                type = "creature",
                colors = { C = "green", T = "red" },
                stats = {x = 1, y = 1},
            }
        "#);
        // Identity Vec is derived in Slot::ALL canonical order — T comes
        // before C, so the values come out (red, green).
        assert_eq!(card.colors, vec!["red", "green"]);
        assert_eq!(card.color_slots.len(), 2);
        assert_eq!(card.color_slots.get(&crate::card::Slot::C).map(String::as_str), Some("green"));
        assert_eq!(card.color_slots.get(&crate::card::Slot::T).map(String::as_str), Some("red"));
    }

    #[test]
    fn colors_slot_form_rejects_duplicate_colors() {
        let lua = Lua::new();
        let value: Value = lua
            .load(r#"return { id = "dup", colors = { C = "green", T = "green" } }"#)
            .eval()
            .unwrap();
        let table = match value { Value::Table(t) => t, _ => panic!() };
        assert!(read_color_vec(&table).is_err(), "duplicate color must error");
    }

    #[test]
    fn colors_mixed_form_rejected() {
        // Lua treats this as a mix of integer (1) and string (T) keys.
        let lua = Lua::new();
        let value: Value = lua
            .load(r#"return { id = "mixed", colors = { "green", T = "red" } }"#)
            .eval()
            .unwrap();
        let table = match value { Value::Table(t) => t, _ => panic!() };
        assert!(read_color_vec(&table).is_err(), "mixed form must error");
    }

    fn handlers_from(lua: &Lua, src: &str) -> BTreeMap<EventName, Function> {
        let value: Value = lua.load(src).eval().unwrap();
        let table = match value {
            Value::Table(t) => t,
            _ => panic!("expected table"),
        };
        read_handlers(&table).unwrap()
    }

    #[test]
    fn handler_field_captures_lua_function() {
        let lua = Lua::new();
        let handlers = handlers_from(
            &lua,
            r#"
            return {
                id = "fixture",
                on_blocked_by = function(game, self, blocker)
                    return "ran"
                end,
            }
        "#,
        );
        let handler = handlers.get(&EventName::OnBlockedBy).unwrap();
        let result: String = handler.call((Value::Nil, Value::Nil, Value::Nil)).unwrap();
        assert_eq!(result, "ran");
    }

    #[test]
    fn missing_handler_keys_are_absent() {
        let lua = Lua::new();
        let handlers = handlers_from(&lua, r#"return { id = "fixture" }"#);
        assert!(handlers.is_empty());
    }

    #[test]
    fn non_function_handler_value_errors() {
        let lua = Lua::new();
        let value: Value = lua
            .load(r#"return { id = "x", on_die = 5 }"#)
            .eval()
            .unwrap();
        let table = match value {
            Value::Table(t) => t,
            _ => panic!(),
        };
        assert!(read_handlers(&table).is_err());
    }

    #[test]
    fn registry_keeps_handlers_callable() {
        // The whole reason CardRegistry owns the Lua: handlers stay valid
        // as long as the registry lives.
        let tmp = std::env::temp_dir().join("tsot_card_handlers_test");
        std::fs::create_dir_all(&tmp).unwrap();
        let card_path = tmp.join("test-handler.lua");
        std::fs::write(
            &card_path,
            r#"return {
                id = "test-handler",
                on_die = function(game, self) return "fired" end,
            }"#,
        )
        .unwrap();

        let registry = CardRegistry::load(&tmp).unwrap();
        let card = registry
            .cards()
            .iter()
            .find(|c| c.id == "test-handler")
            .unwrap();
        let handler = card.handlers.get(&EventName::OnDie).unwrap();
        let result: String = handler.call((Value::Nil, Value::Nil)).unwrap();
        assert_eq!(result, "fired");

        std::fs::remove_file(&card_path).ok();
    }

    fn load_card_from_lua(src: &str) -> Card {
        // Unique temp-dir name per call without going through rand::random
        // (which is disallowed project-wide for determinism reasons — see
        // clippy.toml). A monotonic counter per process is enough for
        // test uniqueness; tests don't need randomness here.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_card_test_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("under-test.lua");
        std::fs::write(&path, src).unwrap();
        let registry = CardRegistry::load(&tmp).unwrap();
        let card = registry
            .cards()
            .iter()
            .find(|c| c.id == "under-test")
            .expect("card loaded")
            .clone();
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&tmp).ok();
        card
    }

    #[test]
    fn cryogenic_chamber_card_loads_with_full_shape() {
        let lua = Lua::new();
        let path = Path::new("cards/cryogenic-chamber.lua");
        let cards = load_card(&lua, path).expect("load cryogenic-chamber");
        let card = cards.iter().find(|c| c.id == "cryogenic-chamber").unwrap();
        let mut colors_sorted = card.colors.clone();
        colors_sorted.sort();
        assert_eq!(colors_sorted, vec!["azure", "white"]);
        assert!(matches!(card.kind, crate::card::CardType::Artifact));
        assert_eq!(card.cost.len(), 1);
        let cost0 = &card.cost[0];
        assert_eq!(cost0.amount, 1);
        assert!(matches!(cost0.source, crate::card::CostSource::Graveyard));
        use crate::card::Slot;
        let actual: std::collections::BTreeSet<Slot> = card.holes.iter().copied().collect();
        let expected: std::collections::BTreeSet<Slot> = [
            Slot::L, Slot::R, Slot::T, Slot::TR, Slot::B, Slot::BL,
        ]
        .into_iter()
        .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn missense_mutation_card_loads_with_full_shape() {
        let lua = Lua::new();
        let path = Path::new("cards/missense-mutation.lua");
        let cards = load_card(&lua, path).expect("load missense-mutation");
        let card = cards.iter().find(|c| c.id == "missense-mutation").unwrap();
        assert_eq!(card.colors, vec!["cyan"]);
        assert_eq!(card.kind, crate::card::CardType::Mutation);
        assert_eq!(card.cost.len(), 2);
        let def = card.static_def.as_ref().expect("static_def present");
        assert!(matches!(def.affects.scope, crate::card::StaticScope::AttachedHost));
        use crate::card::ModifierValue;
        // X = +1 per shiny board card.
        assert_eq!(
            def.modifier_x,
            ModifierValue::Sum(vec![
                ModifierValue::Fixed(0.0),
                ModifierValue::Scaled(1.0, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ])
        );
        // Y = -0.5 + (-0.25 × shiny board count).
        assert_eq!(
            def.modifier_y,
            ModifierValue::Sum(vec![
                ModifierValue::Fixed(-0.5),
                ModifierValue::Scaled(-0.25, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ])
        );
        // Becomes cyan: grant cyan to the host.
        assert!(def.granted_colors.iter().any(|c| c == "cyan"));
    }

    #[test]
    fn modifier_sum_and_scaled_table_form_parses_to_sum_scaled() {
        // Lua side: `y = { -0.5, {scale = -0.25, count = "board:face:shiny"} }`
        // The sequence form is a Sum. Each entry is either a scalar
        // (Fixed) or a table with `scale` + `count` (Scaled).
        let card = load_card_from_lua(r#"
            return {
                id = "under-test",
                type = "mutation",
                static = {
                    affects = { scope = "attached_host" },
                    modifier = {
                        x = 0,
                        y = { -0.5, {scale = -0.25, count = "board:face:shiny"} },
                    },
                },
            }
        "#);
        let def = card.static_def.expect("static_def present");
        use crate::card::ModifierValue;
        assert_eq!(
            def.modifier_y,
            ModifierValue::Sum(vec![
                ModifierValue::Fixed(-0.5),
                ModifierValue::Scaled(-0.25, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ])
        );
    }

    #[test]
    fn modifier_descriptor_board_face_parses_to_board_count_by_face() {
        let card = load_card_from_lua(r#"
            return {
                id = "under-test",
                type = "mutation",
                static = {
                    affects = { scope = "attached_host" },
                    modifier = { x = "board:face:shiny", y = 0 },
                },
            }
        "#);
        let def = card.static_def.expect("static_def present");
        assert_eq!(def.modifier_x, crate::card::ModifierValue::BoardCountByFace("shiny".into()));
    }

    #[test]
    fn nonsense_mutation_card_loads_with_full_shape() {
        let lua = Lua::new();
        let path = Path::new("cards/nonsense-mutation.lua");
        let cards = load_card(&lua, path).expect("load nonsense-mutation");
        let card = cards.iter().find(|c| c.id == "nonsense-mutation").unwrap();
        assert_eq!(card.colors, vec!["purple"]);
        assert_eq!(card.kind, crate::card::CardType::Mutation);
        // 1 graveyard + 2 mill.
        assert_eq!(card.cost.len(), 2);
        let def = card.static_def.as_ref().expect("static_def present");
        assert!(matches!(def.affects.scope, crate::card::StaticScope::AttachedHost));
        assert_eq!(def.modifier_x, crate::card::ModifierValue::Fixed(1.0));
        assert_eq!(def.modifier_y, crate::card::ModifierValue::Fixed(-1.0));
        assert!(def.makes_host_colorless);
        assert!(def.suppresses_host_abilities);
    }

    #[test]
    fn static_modifier_colorless_and_suppresses_abilities_load() {
        let card = load_card_from_lua(r#"
            return {
                id = "under-test",
                type = "mutation",
                static = {
                    affects = { scope = "attached_host" },
                    modifier = {
                        x = 1, y = -1,
                        colorless = true,
                        suppresses_abilities = true,
                    },
                },
            }
        "#);
        let def = card.static_def.expect("static_def present");
        assert!(def.makes_host_colorless, "modifier.colorless = true → makes_host_colorless");
        assert!(def.suppresses_host_abilities, "modifier.suppresses_abilities = true → suppresses_host_abilities");
    }

    #[test]
    fn symbol_shorthand_parses_to_one_element_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test", symbol = "꩜" }"#);
        assert_eq!(card.symbols, vec!["꩜".to_string()]);
    }

    #[test]
    fn symbols_array_parses_in_order() {
        let card = load_card_from_lua(
            r#"return { id = "under-test", symbols = {"꩜", "⨳", "⋈"} }"#,
        );
        assert_eq!(
            card.symbols,
            vec!["꩜".to_string(), "⨳".to_string(), "⋈".to_string()]
        );
    }

    #[test]
    fn no_symbol_fields_yields_empty_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test" }"#);
        assert!(card.symbols.is_empty());
    }

    #[test]
    fn symbols_array_takes_priority_when_both_fields_present() {
        let card = load_card_from_lua(
            r#"return { id = "under-test", symbol = "X", symbols = {"꩜", "⨳"} }"#,
        );
        assert_eq!(card.symbols, vec!["꩜".to_string(), "⨳".to_string()]);
    }

    #[test]
    fn empty_symbol_shorthand_yields_empty_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test", symbol = "" }"#);
        assert!(card.symbols.is_empty());
    }

    /// Load a directory of cards instead of looking one up. Used by
    /// the `variants` tests below which need to see BOTH the base and
    /// the synthesized variants in the registry.
    fn load_dir_cards(src: &str) -> Vec<Card> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_variants_test_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("under-test.lua");
        std::fs::write(&path, src).unwrap();
        let registry = CardRegistry::load(&tmp).unwrap();
        let cards: Vec<Card> = registry.cards().to_vec();
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&tmp).ok();
        cards
    }

    #[test]
    fn card_without_variants_loads_as_single_card() {
        let cards = load_dir_cards(
            r#"return { id = "under-test", type = "creature", stats = {x = 1, y = 1} }"#,
        );
        assert_eq!(cards.len(), 1, "base only — no variants table present");
        assert_eq!(cards[0].id, "under-test");
        assert!(!cards[0].is_variant);
        assert!(cards[0].variant_of.is_none());
    }

    #[test]
    fn variants_emit_one_card_per_entry_with_suffixed_ids() {
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                type = "creature",
                stats = {x = 1, y = 1},
                variants = {
                    ["small"] = { stats = {x = 1, y = 2} },
                    ["big"]   = { stats = {x = 4, y = 4} },
                },
            }"#,
        );
        // Order in `cards` is implementation-defined (Lua pairs() over
        // string keys). Check by id rather than index.
        let by_id: std::collections::BTreeMap<&str, &Card> =
            cards.iter().map(|c| (c.id.as_str(), c)).collect();
        assert!(by_id.contains_key("under-test"), "base id present");
        assert!(by_id.contains_key("under-test-small"), "variant id present");
        assert!(by_id.contains_key("under-test-big"), "variant id present");
        assert_eq!(cards.len(), 3);
        let base = by_id["under-test"];
        assert!(!base.is_variant, "base is_variant = false");
        let small = by_id["under-test-small"];
        assert!(small.is_variant, "variant is_variant = true");
        assert_eq!(small.variant_of.as_deref(), Some("under-test"));
        assert_eq!(small.stats.unwrap().y, 2.0, "variant stats override applied");
        let big = by_id["under-test-big"];
        assert_eq!(big.stats.unwrap().x, 4.0);
        assert_eq!(big.stats.unwrap().y, 4.0);
    }

    #[test]
    fn modifier_value_scaled_attached_parses() {
        // `"2*attached"` → AttachedCountScaled(2) via the static block.
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                type = "creature",
                stats = {x = 0, y = 0},
                static = {
                    affects = { scope = "source_only" },
                    modifier = {x = "2*attached", y = "3*attached"},
                },
            }"#,
        );
        let s = cards[0].static_def.as_ref().expect("static set");
        assert_eq!(s.modifier_x, super::ModifierValue::AttachedCountScaled(2));
        assert_eq!(s.modifier_y, super::ModifierValue::AttachedCountScaled(3));
    }

    #[test]
    fn variant_keys_not_declared_inherit_from_base() {
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                name = "Base Name",
                type = "creature",
                colors = {"green"},
                stats = {x = 2, y = 2},
                variants = {
                    ["v1"] = { stats = {x = 5, y = 5} },  -- only stats overridden
                },
            }"#,
        );
        let by_id: std::collections::BTreeMap<&str, &Card> =
            cards.iter().map(|c| (c.id.as_str(), c)).collect();
        let v1 = by_id["under-test-v1"];
        // Inherited fields:
        assert_eq!(v1.name, "Base Name", "name inherited");
        assert_eq!(v1.colors, vec!["green"], "colors inherited");
        // Overridden:
        assert_eq!(v1.stats.unwrap().x, 5.0);
        assert_eq!(v1.stats.unwrap().y, 5.0);
    }

    #[test]
    fn cost_source_attached_parses() {
        let card = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                cost = {{amount = 2, source = "attached"}},
            }"#,
        );
        assert_eq!(card.cost.len(), 1);
        assert_eq!(card.cost[0].amount, 2);
        assert!(matches!(card.cost[0].source, CostSource::Attached));
    }

    #[test]
    fn sandbox_denies_dangerous_stdlib() {
        // Empty registry — just inspect the VM's globals.
        let tmp = std::env::temp_dir().join("tsot_sandbox_probe");
        std::fs::create_dir_all(&tmp).unwrap();
        if let Ok(rd) = std::fs::read_dir(&tmp) {
            for entry in rd.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let registry = CardRegistry::load(&tmp).unwrap();
        let globals = registry.lua().globals();

        for forbidden in ["os", "io", "package", "debug", "loadstring", "dofile", "loadfile", "require"] {
            let v: Value = globals.get(forbidden).unwrap();
            assert!(
                matches!(v, Value::Nil),
                "expected `{forbidden}` to be nil in sandboxed VM, got {v:?}"
            );
        }
        for allowed in ["math", "string", "table"] {
            let v: Value = globals.get(allowed).unwrap();
            assert!(
                matches!(v, Value::Table(_)),
                "expected `{allowed}` to be present in sandboxed VM"
            );
        }
    }
}
