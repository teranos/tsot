use mlua::{Lua, Table, Value};
use std::fs;
use std::path::Path;

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

#[derive(Debug, Clone)]
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
