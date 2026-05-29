use mlua::Lua;
use std::path::Path;
use tsot::card::load_cards_dir;
use tsot::game::GameState;

fn main() -> mlua::Result<()> {
    let lua = Lua::new();
    let cards = load_cards_dir(&lua, Path::new("cards"))?;
    println!("loaded {} cards", cards.len());

    // Build placeholder decks: each player's deck is one copy of every card in the corpus.
    // No shuffling; deal order is corpus order. Real games will shuffle.
    let state = GameState::new(cards.clone(), cards);

    println!();
    println!("game state initialized:");
    println!("  turn         {}", state.turn);
    println!("  phase        {:?}", state.phase);
    println!("  active       {:?}", state.active_player);
    println!(
        "  player A     hand: {:>2} cards   deck: {:>2} cards",
        state.a.hand.len(),
        state.a.deck.len()
    );
    println!(
        "  player B     hand: {:>2} cards   deck: {:>2} cards",
        state.b.hand.len(),
        state.b.deck.len()
    );

    match state.check_loss() {
        Some(loser) => println!("  loss check   player {:?} already has empty deck", loser),
        None => println!("  loss check   neither player has lost"),
    }

    Ok(())
}
