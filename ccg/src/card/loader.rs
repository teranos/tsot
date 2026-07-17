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
        "graveyard" | "gy" => Ok(CostSource::Graveyard),
        "sacrifice" | "sac" => Ok(CostSource::Sacrifice),
        "self" => Ok(CostSource::SelfExile),
        "attached" | "attach" => Ok(CostSource::Attached),
        "tap" => Ok(CostSource::Tap),
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
        // Per-entry parsing is shared with `granted_activated` (in
        // `read_static`) via `parse_one_activated_entry`. The label
        // appears in every error message so the developer knows
        // which Lua field a parse failure came from.
        out.push(parse_one_activated_entry(item?, "activation")?);
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
        Value::Table(t) => Some(parse_one_activated_entry(t, "granted_activated")?),
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.granted_activated must be a table, got {other:?}"
            )))
        }
    };
    // Phase 1 of the StaticEffect refactor: populate the unified
    // continuous-effect vec from the parsed legacy fields. The legacy
    // fields stay (read by existing dispatch sites); new dispatch code
    // reads from `effects`. See the StaticEffect enum comment.
    let mut effects: Vec<crate::card::StaticEffect> = Vec::new();
    // StatBoost only emitted when either axis is nonzero (Fixed(0.0) on
    // both = no boost). Non-Fixed variants (AttachedCount etc.) always
    // count as "boost present" since the value resolves at read time.
    let stat_present = match (&modifier_x, &modifier_y) {
        (crate::card::ModifierValue::Fixed(x), crate::card::ModifierValue::Fixed(y)) => {
            *x != 0.0 || *y != 0.0
        }
        _ => true,
    };
    if stat_present {
        effects.push(crate::card::StaticEffect::StatBoost {
            x: modifier_x.clone(),
            y: modifier_y.clone(),
        });
    }
    if let Some(k) = &modifier_keyword {
        effects.push(crate::card::StaticEffect::KeywordGrant(k.clone()));
    }
    for r in &restrictions {
        effects.push(crate::card::StaticEffect::Restrict(*r));
    }
    for cm in &cost_modifiers {
        effects.push(crate::card::StaticEffect::CostModify {
            source: cm.source,
            amount: cm.amount,
        });
    }
    if let Some(act) = &granted_activated {
        effects.push(crate::card::StaticEffect::GrantActivated(act.clone()));
    }
    for c in &granted_colors {
        effects.push(crate::card::StaticEffect::GrantColor(c.clone()));
    }
    for f in &granted_face {
        effects.push(crate::card::StaticEffect::GrantFace(f.clone()));
    }
    if makes_host_colorless {
        effects.push(crate::card::StaticEffect::MakesHostColorless);
    }
    if suppresses_host_abilities {
        effects.push(crate::card::StaticEffect::SuppressesHostAbilities);
    }

    Ok(Some(StaticDef {
        affects,
        condition,
        effects,
    }))
}

/// Parse one `{ cost, text, timing, effect, optional validate, optional target }`
/// table into an [`ActivatedAbility`].
///
/// Used by:
///   - [`read_activated`] (top-level `activated[]` field), `field_label`
///     passed as `"activation"`.
///   - [`read_static`] (`granted_activated` field of a `static` block),
///     `field_label` passed as `"granted_activated"`.
///
/// `field_label` is interpolated into every error message so the
/// developer knows which Lua field a parse failure came from — the
/// only meaningful difference between the two call sites.
///
/// Two shapes are supported for `cost`:
///   1. String shorthand: `cost = "tap"` → tap-only, no components.
///   2. List of components: `cost = {{source = "...", amount = N}}` →
///      one or more cost components, possibly including a tap
///      pseudo-component `{source = "tap"}` (no amount).
fn parse_one_activated_entry(
    item: Table,
    field_label: &str,
) -> mlua::Result<ActivatedAbility> {
    let cost_value: Value = item.get("cost")?;
    let (cost_tap, cost_components) = match cost_value {
        Value::String(s) => {
            let s = s.to_str()?.to_ascii_lowercase();
            if s == "tap" || s == "t" {
                (true, Vec::new())
            } else {
                return Err(mlua::Error::runtime(format!(
                    "{field_label} cost string {s:?} not recognized (expected \"tap\")"
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
                "{field_label} cost must be a string or list, got {other:?}"
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
                "{field_label} timing {other:?} must be \"instant\" or \"sorcery\""
            )))
        }
    };
    let validate: Option<Function> = match item.get::<Value>("validate")? {
        Value::Nil => None,
        Value::Function(f) => Some(f),
        other => {
            return Err(mlua::Error::runtime(format!(
                "{field_label} validate must be a function, got {other:?}"
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
                    "unknown {field_label} target category: {other:?}"
                )))
            }
        },
    };
    // RULES A.13: activation zone list. Default `[Board]` matches the
    // pre-#5 implicit behavior. Lua accepts a single string
    // (`from_zones = "graveyard"`) or a list (`from_zones = {"board", "graveyard"}`).
    let from_zones = match item.get::<Value>("from_zones")? {
        Value::Nil => vec![crate::card::ActivationZone::Board],
        Value::String(s) => vec![parse_activation_zone(s.to_str()?.as_ref(), field_label)?],
        Value::Table(t) => {
            let mut out = Vec::new();
            for entry in t.sequence_values::<String>() {
                out.push(parse_activation_zone(&entry?, field_label)?);
            }
            if out.is_empty() {
                vec![crate::card::ActivationZone::Board]
            } else {
                out
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "{field_label} from_zones must be a string or list of strings, got {other:?}"
            )))
        }
    };
    Ok(ActivatedAbility {
        cost_tap,
        cost_components,
        text,
        timing,
        validate,
        target,
        effect,
        from_zones,
    })
}

fn parse_activation_zone(s: &str, field_label: &str) -> mlua::Result<crate::card::ActivationZone> {
    use crate::card::ActivationZone;
    match s.to_ascii_lowercase().as_str() {
        "board" => Ok(ActivationZone::Board),
        "hand" => Ok(ActivationZone::Hand),
        "graveyard" | "gy" => Ok(ActivationZone::Graveyard),
        "exile" => Ok(ActivationZone::Exile),
        "deck" => Ok(ActivationZone::Deck),
        "attached" | "attach" => Ok(ActivationZone::Attached),
        other => Err(mlua::Error::runtime(format!(
            "unknown {field_label} from_zones entry: {other:?} \
             (allowed: board, hand, graveyard, exile, deck, attached)"
        ))),
    }
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
    // RULES Z.7: mutation cards declare `same_sleeve = true` to fuse
    // inside the host's sleeve rather than attach as a separate object.
    let same_sleeve = table
        .get::<Option<bool>>("same_sleeve")?
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
        same_sleeve,
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
        // Sacred-error: malformed individual card files surface a
        // typed Error and the corpus continues loading WITHOUT that
        // card. Previously `?` aborted the whole load on the first
        // bad card, which both hid which card was bad (mlua's error
        // doesn't always preserve file context) and silently shrank
        // the corpus to ZERO. Per-card try-and-emit lets the engine
        // boot with a known-incomplete corpus AND tells the developer
        // which file rejected.
        match load_card(lua, p) {
            Ok(cards) => all.extend(cards),
            Err(e) => {
                crate::error::emit_region(
                    crate::error::Severity::Warn,
                    "card-loader",
                    "malformed-card",
                    format!("card file rejected: {}", p.display()),
                    format!("{e}"),
                );
            }
        }
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
        let chunk_name = f.path().display().to_string();
        let source = match f.contents_utf8() {
            Some(s) => s,
            None => {
                crate::error::emit_region(
                    crate::error::Severity::Warn,
                    "card-loader",
                    "malformed-card",
                    format!("embedded card is not UTF-8: {chunk_name}"),
                    "skipping; file is non-text or contains invalid UTF-8".to_string(),
                );
                continue;
            }
        };
        // Same sacred-error pattern as load_cards_dir: one bad card
        // surfaces and the rest still load.
        match load_card_from_source(lua, source, &chunk_name) {
            Ok(cards) => all.extend(cards),
            Err(e) => {
                crate::error::emit_region(
                    crate::error::Severity::Warn,
                    "card-loader",
                    "malformed-card",
                    format!("embedded card rejected: {chunk_name}"),
                    format!("{e}"),
                );
            }
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost-source aliases — common short or typo forms accepted alongside
    /// the canonical names so cards don't fail to load over a missing 'd'
    /// or an abbreviation in the .lua file.
    #[test]
    fn parse_source_accepts_aliases() {
        // attached / attach
        assert_eq!(parse_source("attached").unwrap(), CostSource::Attached);
        assert_eq!(parse_source("attach").unwrap(), CostSource::Attached);
        // sacrifice / sac
        assert_eq!(parse_source("sacrifice").unwrap(), CostSource::Sacrifice);
        assert_eq!(parse_source("sac").unwrap(), CostSource::Sacrifice);
        // graveyard / gy
        assert_eq!(parse_source("graveyard").unwrap(), CostSource::Graveyard);
        assert_eq!(parse_source("gy").unwrap(), CostSource::Graveyard);
    }

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
        let path = Path::new("cards/MISSENSE-MUTATION.lua");
        let cards = load_card(&lua, path).expect("load MISSENSE-MUTATION");
        let card = cards.iter().find(|c| c.id == "MISSENSE-MUTATION").unwrap();
        assert_eq!(card.colors, vec!["cyan"]);
        assert_eq!(card.kind, crate::card::CardType::Mutation);
        assert_eq!(card.cost.len(), 2);
        let def = card.static_def.as_ref().expect("static_def present");
        assert!(matches!(def.affects.scope, crate::card::StaticScope::AttachedHost));
        use crate::card::{ModifierValue, StaticEffect};
        let (mx, my) = def.effects.iter().find_map(|e| match e {
            StaticEffect::StatBoost { x, y } => Some((x.clone(), y.clone())),
            _ => None,
        }).expect("stat boost effect present");
        // X = +1 per shiny board card.
        assert_eq!(
            mx,
            ModifierValue::Sum(vec![
                ModifierValue::Fixed(0.0),
                ModifierValue::Scaled(1.0, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ])
        );
        // Y = -0.5 + (-0.25 × shiny board count).
        assert_eq!(
            my,
            ModifierValue::Sum(vec![
                ModifierValue::Fixed(-0.5),
                ModifierValue::Scaled(-0.25, Box::new(ModifierValue::BoardCountByFace("shiny".into()))),
            ])
        );
        // Becomes cyan: grant cyan to the host.
        assert!(def.effects.iter().any(|e| matches!(e, StaticEffect::GrantColor(c) if c == "cyan")));
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
        use crate::card::{ModifierValue, StaticEffect};
        let my = def.effects.iter().find_map(|e| match e {
            StaticEffect::StatBoost { y, .. } => Some(y.clone()),
            _ => None,
        }).expect("stat boost effect present");
        assert_eq!(
            my,
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
        let mx = def.effects.iter().find_map(|e| match e {
            crate::card::StaticEffect::StatBoost { x, .. } => Some(x.clone()),
            _ => None,
        }).expect("stat boost effect present");
        assert_eq!(mx, crate::card::ModifierValue::BoardCountByFace("shiny".into()));
    }

    #[test]
    fn nonsense_mutation_card_loads_with_full_shape() {
        let lua = Lua::new();
        let path = Path::new("cards/NONSENSE-MUTATION.lua");
        let cards = load_card(&lua, path).expect("load NONSENSE-MUTATION");
        let card = cards.iter().find(|c| c.id == "NONSENSE-MUTATION").unwrap();
        assert_eq!(card.colors, vec!["purple"]);
        assert_eq!(card.kind, crate::card::CardType::Mutation);
        // 1 graveyard + 2 mill.
        assert_eq!(card.cost.len(), 2);
        let def = card.static_def.as_ref().expect("static_def present");
        assert!(matches!(def.affects.scope, crate::card::StaticScope::AttachedHost));
        use crate::card::StaticEffect;
        let (mx, my) = def.effects.iter().find_map(|e| match e {
            StaticEffect::StatBoost { x, y } => Some((x.clone(), y.clone())),
            _ => None,
        }).expect("stat boost effect present");
        assert_eq!(mx, crate::card::ModifierValue::Fixed(1.0));
        assert_eq!(my, crate::card::ModifierValue::Fixed(-1.0));
        assert!(def.effects.iter().any(|e| matches!(e, StaticEffect::MakesHostColorless)));
        assert!(def.effects.iter().any(|e| matches!(e, StaticEffect::SuppressesHostAbilities)));
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
        use crate::card::StaticEffect;
        assert!(
            def.effects.iter().any(|e| matches!(e, StaticEffect::MakesHostColorless)),
            "modifier.colorless = true → MakesHostColorless effect",
        );
        assert!(
            def.effects.iter().any(|e| matches!(e, StaticEffect::SuppressesHostAbilities)),
            "modifier.suppresses_abilities = true → SuppressesHostAbilities effect",
        );
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
        let (mx, my) = s.effects.iter().find_map(|e| match e {
            super::StaticEffect::StatBoost { x, y } => Some((x.clone(), y.clone())),
            _ => None,
        }).expect("stat boost effect present");
        assert_eq!(mx, super::ModifierValue::AttachedCountScaled(2));
        assert_eq!(my, super::ModifierValue::AttachedCountScaled(3));
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

    // ---- parse_one_activated_entry shared-helper tests ------------
    //
    // Pin the dedup: both `read_activated` (top-level `activated[]`)
    // and `read_static`'s `granted_activated` path funnel through
    // `parse_one_activated_entry(item, field_label)`. The helper
    // produces identical `ActivatedAbility` values for identical
    // inputs; only error-message wording diverges via `field_label`.
    //
    // Before the 2026-06-18 dedup these were two ~85-line parser
    // bodies with subtle divergences (e.g. "string or a list" vs
    // "string or list", "unknown activation timing: X (must be Y)"
    // vs "granted_activated timing X must be Y") — kept intact in
    // muscle memory by maintainers re-syncing fixes one-at-a-time.

    fn load_card_from_lua_or_err(src: &str) -> Result<Card, String> {
        // Variant of load_card_from_lua that returns the loader
        // error instead of panicking. Since the 2026-06-18 sacred-
        // error sweep, `load_cards_dir` no longer aborts on a
        // malformed card — it emits a typed Error and continues.
        // So the test pulls the latest typed-error message from
        // `crate::error::drain()` if the registry came back without
        // our `under-test` card.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_card_test_err_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("under-test.lua");
        std::fs::write(&path, src).unwrap();
        // Reset the error bus so our drain only sees errors from THIS
        // load. The bus is thread-local and shared across tests; if
        // another test left errors in it, our assertion could match
        // the wrong message.
        crate::error::reset();
        let result = CardRegistry::load(&tmp)
            .map_err(|e| format!("{e}"))
            .and_then(|registry| {
                registry
                    .cards()
                    .iter()
                    .find(|c| c.id == "under-test")
                    .cloned()
                    .ok_or_else(|| {
                        // Card didn't load — the loader emitted a typed
                        // Error on the bus instead of returning Err.
                        // Drain it and surface the message so the test
                        // can assert on the field label.
                        let errors = crate::error::drain();
                        errors
                            .last()
                            .map(|e| format!("{}: {}", e.title, e.why))
                            .unwrap_or_else(|| {
                                "card with id `under-test` not loaded \
                                 (and no typed Error on the bus)".to_string()
                            })
                    })
            });
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&tmp).ok();
        result
    }

    #[test]
    fn activated_happy_path_parses_via_shared_helper() {
        // Verifies read_activated → parse_one_activated_entry round-
        // trips. Exact same shape that previously had its own copy
        // of the parser.
        let card = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                activated = {
                    {
                        cost = "tap",
                        text = "T: draw a card",
                        timing = "sorcery",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        );
        assert_eq!(card.activated.len(), 1);
        let a = &card.activated[0];
        assert!(a.cost_tap, "cost_tap must be true for cost=\"tap\"");
        assert!(a.cost_components.is_empty());
        assert_eq!(a.text, "T: draw a card");
        assert!(matches!(a.timing, Timing::Sorcery));
    }

    #[test]
    fn granted_activated_happy_path_parses_via_shared_helper() {
        // Verifies read_static → parse_one_activated_entry round-
        // trips. Same shape as activated[] but lives under
        // static.granted_activated.
        let card = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                static = {
                    affects = { scope = "attached_host" },
                    granted_activated = {
                        cost = "tap",
                        text = "T: draw a card",
                        timing = "sorcery",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        );
        let st = card.static_def.as_ref().expect("static_def present");
        let ga = st.effects.iter().find_map(|e| match e {
            super::StaticEffect::GrantActivated(a) => Some(a),
            _ => None,
        }).expect("GrantActivated effect present");
        assert!(ga.cost_tap);
        assert!(ga.cost_components.is_empty());
        assert_eq!(ga.text, "T: draw a card");
        assert!(matches!(ga.timing, Timing::Sorcery));
    }

    #[test]
    fn activated_error_message_uses_activation_label() {
        // The whole point of the `field_label` parameter: the
        // developer should see WHICH Lua field a malformed entry
        // came from. read_activated's path → "activation ...".
        let err = load_card_from_lua_or_err(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                activated = {
                    {
                        cost = "wiggle",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        )
        .expect_err("malformed activated cost must error");
        assert!(
            err.contains("activation"),
            "error should mention `activation` field: {err}"
        );
        assert!(
            !err.contains("granted_activated"),
            "error from read_activated must NOT mention granted_activated: {err}"
        );
    }

    #[test]
    fn granted_activated_error_message_uses_granted_activated_label() {
        // The complement: read_static's path → "granted_activated ...".
        // A maintainer who breaks the granted_activated parser sees the
        // origin in the error, not a generic "cost must be ...".
        let err = load_card_from_lua_or_err(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                static = {
                    affects = { scope = "attached_host" },
                    granted_activated = {
                        cost = "wiggle",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        )
        .expect_err("malformed granted_activated cost must error");
        assert!(
            err.contains("granted_activated"),
            "error should mention `granted_activated` field: {err}"
        );
    }

    #[test]
    fn activated_and_granted_activated_produce_identical_ability_for_same_input() {
        // The strongest dedup contract: the two code paths now go
        // through the SAME function with only the label differing.
        // Given identical cost/text/timing/effect input, the
        // ActivatedAbility values they construct must be field-
        // identical (modulo the effect Function reference, which
        // can't be compared by eq).
        let lhs = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                activated = {
                    {
                        cost = {{source = "graveyard", amount = 2}, {source = "tap"}},
                        text = "graveyard fuel + tap",
                        timing = "instant",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        );
        let rhs = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                static = {
                    affects = { scope = "attached_host" },
                    granted_activated = {
                        cost = {{source = "graveyard", amount = 2}, {source = "tap"}},
                        text = "graveyard fuel + tap",
                        timing = "instant",
                        effect = function(g, s) end,
                    },
                },
            }"#,
        );
        let l = &lhs.activated[0];
        let r = rhs
            .static_def
            .as_ref()
            .unwrap()
            .effects
            .iter()
            .find_map(|e| match e {
                super::StaticEffect::GrantActivated(a) => Some(a),
                _ => None,
            })
            .expect("GrantActivated present");
        assert_eq!(l.cost_tap, r.cost_tap);
        assert_eq!(l.cost_components.len(), r.cost_components.len());
        for (lc, rc) in l.cost_components.iter().zip(r.cost_components.iter()) {
            assert_eq!(lc.amount, rc.amount);
            assert!(matches!(lc.source, CostSource::Graveyard));
            assert!(matches!(rc.source, CostSource::Graveyard));
            assert_eq!(lc.is_x, rc.is_x);
        }
        assert_eq!(l.text, r.text);
        assert!(matches!(l.timing, Timing::Instant));
        assert!(matches!(r.timing, Timing::Instant));
    }

    // ---- loader malformed-card surface verification ---------------
    //
    // ERROR.md "Engine internals" item:
    //   "src/card/loader.rs malformed-card handling — does a rejected
    //    card surface anywhere, or does the corpus silently shrink by
    //    one entry?"
    //
    // These tests pin the post-2026-06-18 contract: a malformed card
    // emits a typed `Severity::Warn` (surface="card-loader",
    // region="malformed-card") AND the rest of the corpus continues
    // to load. Closes the verification debt for the loader path
    // without requiring a browser session (the typed Error pushed
    // here is what the dev tool renders inline at the deckbuilder).

    #[test]
    fn load_cards_dir_surfaces_typed_warn_for_a_malformed_card() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_verify_loader_malformed_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        // One well-formed card.
        std::fs::write(
            tmp.join("aaa-good.lua"),
            r#"return {
                id = "verify-good",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
            }"#,
        )
        .unwrap();
        // One broken card. Note the leading 'aaa-' / 'zzz-' filenames
        // so `entries.sort()` puts the good one BEFORE the bad —
        // proving the bad card doesn't stop the next iteration.
        std::fs::write(tmp.join("zzz-bad.lua"), "this is )) not valid lua").unwrap();

        crate::error::reset();
        let registry = CardRegistry::load(&tmp)
            .expect("loader must NOT abort the whole corpus on a single bad card");
        let errors = crate::error::drain();

        // The good card loaded.
        let good_loaded = registry.cards().iter().any(|c| c.id == "verify-good");
        assert!(
            good_loaded,
            "verify-good must be present despite zzz-bad.lua being malformed"
        );

        // Exactly one typed Warn surfaced for the bad card.
        assert_eq!(
            errors.len(),
            1,
            "exactly one typed Error must surface for the bad card; got {}: {:?}",
            errors.len(),
            errors
        );
        let e = &errors[0];
        assert_eq!(e.severity, crate::error::Severity::Warn);
        assert_eq!(e.context.surface, "card-loader");
        assert_eq!(e.context.region.as_deref(), Some("malformed-card"));
        assert!(
            e.title.contains("zzz-bad.lua"),
            "the title must name the offending file; got: {}",
            e.title
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_cards_dir_continues_loading_a_third_card_after_a_bad_one_in_the_middle() {
        // Even harder case: bad card sits between two good ones in
        // the sort order. Proves the loop doesn't break out early
        // on the bad entry.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_verify_loader_middle_bad_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        for (name, src) in [
            (
                "aaa.lua",
                r#"return { id = "verify-a", type = "creature", colors = {"green"}, stats = {x=1,y=1} }"#,
            ),
            ("mmm.lua", "totally not valid lua"),
            (
                "zzz.lua",
                r#"return { id = "verify-z", type = "creature", colors = {"red"}, stats = {x=2,y=2} }"#,
            ),
        ] {
            std::fs::write(tmp.join(name), src).unwrap();
        }

        crate::error::reset();
        let registry = CardRegistry::load(&tmp).expect("registry must load with 2 good + 1 bad");
        let errors = crate::error::drain();
        let ids: Vec<&str> = registry.cards().iter().map(|c| c.id.as_str()).collect();

        assert!(
            ids.contains(&"verify-a"),
            "verify-a (before bad) must be present: {:?}",
            ids
        );
        assert!(
            ids.contains(&"verify-z"),
            "verify-z (after bad) must be present: {:?}",
            ids
        );
        assert_eq!(errors.len(), 1, "exactly one typed Warn for the middle bad card");
        assert_eq!(errors[0].severity, crate::error::Severity::Warn);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Slice #5 corpus wiring: every ghost in the 6-color cycle plus
    /// durian-elemental must load with the new `from_zones` schema on
    /// at least one activated ability. Pins the fact that the engine's
    /// ActivationZone parse path accepts what these cards declare.
    #[test]
    fn ghost_cycle_and_durian_load_with_zoned_activations() {
        use crate::card::{ActivationZone, CardRegistry};
        let registry = CardRegistry::load_embedded().expect("embedded registry must load");
        for id in [
            "blue-ghost",
            "green-ghost",
            "yellow-ghost",
            "purple-ghost",
            "red-ghost",
            "pink-ghost",
        ] {
            let c = registry
                .get(id)
                .unwrap_or_else(|| panic!("{id} must be in the registry"));
            assert_eq!(c.activated.len(), 2, "{id} has 2 activated abilities");
            assert!(
                c.activated[0].from_zones.contains(&ActivationZone::Attached),
                "{id} first activation is from attached"
            );
            assert!(
                c.activated[1].from_zones.contains(&ActivationZone::Graveyard),
                "{id} second activation is from graveyard"
            );
        }
        let durian = registry
            .get("durian-elemental")
            .expect("durian-elemental must be in the registry");
        assert_eq!(durian.activated.len(), 1, "durian has 1 activated ability");
        assert!(
            durian.activated[0]
                .from_zones
                .contains(&ActivationZone::Graveyard),
            "durian activation is from graveyard"
        );
    }
}
