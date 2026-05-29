use mlua::Lua;
use std::path::Path;
use tsot::card::load_cards_dir;
use tsot::game::{GameState, Phase};

fn main() -> mlua::Result<()> {
    let lua = Lua::new();
    let cards = load_cards_dir(&lua, Path::new("cards"))?;
    println!("loaded {} cards", cards.len());

    // Per S.4 a standard deck is 50 cards. Tile the corpus to fill each player's deck.
    let deck_a: Vec<_> = cards.iter().cloned().cycle().take(50).collect();
    let deck_b: Vec<_> = cards.iter().cloned().cycle().take(50).collect();
    let mut state = GameState::new(deck_a, deck_b);

    println!();
    println!(
        "turn 1 begin: active {:?}, hand A: {:>2}, deck A: {:>2}, hand B: {:>2}, deck B: {:>2}",
        state.active_player,
        state.a.hand.len(),
        state.a.deck.len(),
        state.b.hand.len(),
        state.b.deck.len(),
    );

    let mut steps = 0;
    let cap = 10_000;

    while state.winner.is_none() && steps < cap {
        let prev_turn = state.turn;
        state.next_phase();
        steps += 1;

        // Log start-of-turn snapshots only (each turn = 6 phases, so this fires once per turn).
        if state.turn != prev_turn && state.winner.is_none() && state.phase == Phase::Untap {
            println!(
                "turn {} begin: active {:?}, hand A: {:>2}, deck A: {:>2}, hand B: {:>2}, deck B: {:>2}",
                state.turn,
                state.active_player,
                state.a.hand.len(),
                state.a.deck.len(),
                state.b.hand.len(),
                state.b.deck.len(),
            );
        }
    }

    println!();
    match state.winner {
        Some(winner) => {
            let loser = winner.opponent();
            println!(
                "game over at turn {}: player {:?} ran out of cards, {:?} wins",
                state.turn, loser, winner
            );
        }
        None => println!("hit step cap ({} steps) without a winner", cap),
    }

    Ok(())
}
