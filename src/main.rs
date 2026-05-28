use mlua::Lua;
use std::path::Path;
use tsot::card::load_cards_dir;

fn main() -> mlua::Result<()> {
    let lua = Lua::new();
    let cards = load_cards_dir(&lua, Path::new("cards"))?;
    println!("loaded {} cards", cards.len());
    for c in &cards {
        let stats = c
            .stats
            .map(|s| format!(" {}/{}", s.x, s.y))
            .unwrap_or_default();
        println!("  {:<24} {:?}{}", c.id, c.kind, stats);
    }
    Ok(())
}
