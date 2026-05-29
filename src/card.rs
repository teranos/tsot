use mlua::{Function, Lua, Table, Value};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Owns the long-lived Lua VM and the cards loaded into it.
///
/// The VM outlives the cards because future card fields (event handlers like
/// `on_die`, `static`) will be `mlua::Function` values whose validity is tied
/// to this `Lua`. Built once at startup; not mutated during a game.
pub struct CardRegistry {
    lua: Lua,
    cards: Vec<Card>,
}

impl CardRegistry {
    /// Load every `.lua` file in `dir` into a fresh VM.
    pub fn load(dir: &Path) -> mlua::Result<Self> {
        let lua = Lua::new();
        let cards = load_cards_dir(&lua, dir)?;
        Ok(Self { lua, cards })
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Colorless,
    White,
    Blue,
    Black,
    Red,
    Green,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Unspecified,
    Creature,
    Instant,
    Spell,
    Artifact,
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostSource {
    Hand,
    Mill,
    Graveyard,
    Sacrifice,
    SelfExile,
}

#[derive(Debug, Clone)]
pub struct CostComponent {
    pub amount: i32,
    pub source: CostSource,
    pub is_x: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub x: i32,
    pub y: i32,
}

/// Event handler keys recognised on card files. Matches LUA.md Phase 1 taxonomy
/// plus `OnBlockedBy` (the squirrel-overrun canary — fires on the attacker when
/// any blocker is declared against it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventName {
    OnEnterBoard,
    OnDie,
    OnAttack,
    OnBlock,
    OnBlockedBy,
    OnPlay,
}

impl EventName {
    /// The Lua field name used to declare this handler on a card table.
    pub fn lua_key(self) -> &'static str {
        match self {
            EventName::OnEnterBoard => "on_enter_board",
            EventName::OnDie => "on_die",
            EventName::OnAttack => "on_attack",
            EventName::OnBlock => "on_block",
            EventName::OnBlockedBy => "on_blocked_by",
            EventName::OnPlay => "on_play",
        }
    }

    /// All known event names, for loader iteration.
    pub const ALL: [EventName; 6] = [
        EventName::OnEnterBoard,
        EventName::OnDie,
        EventName::OnAttack,
        EventName::OnBlock,
        EventName::OnBlockedBy,
        EventName::OnPlay,
    ];
}

#[derive(Clone)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub colors: Vec<Color>,
    pub kind: CardType,
    pub subtypes: Vec<String>,
    pub symbol: String,
    pub cost: Vec<CostComponent>,
    pub abilities: Vec<String>,
    pub stats: Option<Stats>,
    /// Lua event handlers loaded from `on_*` fields. Empty for data-only cards.
    /// Handles are refcounted into the owning `CardRegistry`'s VM and must not
    /// outlive it.
    pub handlers: HashMap<EventName, Function>,
}

impl std::fmt::Debug for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let handler_keys: Vec<&'static str> =
            self.handlers.keys().map(|e| e.lua_key()).collect();
        f.debug_struct("Card")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("colors", &self.colors)
            .field("kind", &self.kind)
            .field("subtypes", &self.subtypes)
            .field("symbol", &self.symbol)
            .field("cost", &self.cost)
            .field("abilities", &self.abilities)
            .field("stats", &self.stats)
            .field("handlers", &handler_keys)
            .finish()
    }
}

fn parse_color(s: &str) -> Result<Color, String> {
    match s.to_ascii_lowercase().as_str() {
        "colorless" => Ok(Color::Colorless),
        "white" => Ok(Color::White),
        "blue" => Ok(Color::Blue),
        "black" => Ok(Color::Black),
        "red" => Ok(Color::Red),
        "green" => Ok(Color::Green),
        other => Err(format!("unknown color: {other}")),
    }
}

fn parse_type(s: &str) -> Result<CardType, String> {
    match s.to_ascii_lowercase().as_str() {
        "" => Ok(CardType::Unspecified),
        "creature" => Ok(CardType::Creature),
        "instant" => Ok(CardType::Instant),
        "spell" => Ok(CardType::Spell),
        "artifact" => Ok(CardType::Artifact),
        "environment" => Ok(CardType::Environment),
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

fn read_color_vec(t: &Table) -> mlua::Result<Vec<Color>> {
    read_string_vec(t, "colors")?
        .into_iter()
        .map(|s| parse_color(&s).map_err(mlua::Error::runtime))
        .collect()
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
        out.push(CostComponent {
            amount,
            source,
            is_x,
        });
    }
    Ok(out)
}

fn read_handlers(t: &Table) -> mlua::Result<HashMap<EventName, Function>> {
    let mut out = HashMap::new();
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
            let x = s.get::<Option<i32>>("x")?.unwrap_or(0);
            let y = s.get::<Option<i32>>("y")?.unwrap_or(0);
            Ok(Some(Stats { x, y }))
        }
        other => Err(mlua::Error::runtime(format!(
            "field `stats` must be a table, got {other:?}"
        ))),
    }
}

pub fn load_card(lua: &Lua, path: &Path) -> mlua::Result<Card> {
    let source = fs::read_to_string(path).map_err(mlua::Error::external)?;
    let chunk_name = path.display().to_string();
    let value: Value = lua.load(&source).set_name(chunk_name).eval()?;
    let table = match value {
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "card file must return a table, got {other:?}"
            )))
        }
    };

    let id: String = table.get("id")?;
    let name = table.get::<Option<String>>("name")?.unwrap_or_default();
    let symbol = table.get::<Option<String>>("symbol")?.unwrap_or_default();
    let kind_s = table.get::<Option<String>>("type")?.unwrap_or_default();
    let kind = parse_type(&kind_s).map_err(mlua::Error::runtime)?;
    let subtypes = read_string_vec(&table, "subtypes")?;
    let abilities = read_string_vec(&table, "abilities")?;
    let colors = read_color_vec(&table)?;
    let cost = read_cost(&table)?;
    let stats = read_stats(&table)?;
    let handlers = read_handlers(&table)?;

    Ok(Card {
        id,
        name,
        colors,
        kind,
        subtypes,
        symbol,
        cost,
        abilities,
        stats,
        handlers,
    })
}

pub fn load_cards_dir(lua: &Lua, dir: &Path) -> mlua::Result<Vec<Card>> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(mlua::Error::external)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    entries.sort();
    entries.iter().map(|p| load_card(lua, p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handlers_from(lua: &Lua, src: &str) -> HashMap<EventName, Function> {
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
}
