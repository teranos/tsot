//! Weekly CI soak for the `AiKind::Stress` policy.
//!
//! Not part of the normal suite — `#[ignore]`d so `cargo test` stays
//! fast. The `ccg-stress` workflow runs it once a week with
//! `--ignored`. It plays many full games on RANDOM pool decks (so a
//! broad slice of the card library's Lua actually fires) under
//! `AiKind::Stress` on both seats and asserts the engine never hangs,
//! never panics, and every game terminates with a winner. A hang shows
//! up as a `None` winner (the wall-clock watchdog assigns one only on
//! timeout, and its dump now names the offending `game_seed`), so a
//! stuck game is reproducible from the failure message.
//!
//! Volume is `TSOT_STRESS_GAMES` (default 200); the workflow raises it.

// The soak drives `run_game_continue` on a borrowed `&mut state` so it
// can roll the finished game back — the same reason the run.rs rollback
// tests use it. Production UCT rollouts have since moved to StepEngine,
// which deprecates the free function; suppress at the module level like
// `uct.rs` does for its own full-game smoke.
#![allow(deprecated)]

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::card::CardRegistry;
use crate::game::{GameState, Journal};
use crate::sim::genome::{random_genome, to_units};
use crate::sim::playable_pool::playable_pool;
use crate::sim::run::run_game_continue;
use crate::sim::AiKind;

#[test]
#[ignore = "weekly CI soak — run with `cargo test --release -- --ignored stress`"]
fn stress_soak_random_decks_terminate_and_roll_back() {
    let registry =
        std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
    let pool = playable_pool(registry.cards());
    assert!(pool.len() >= 10, "stress soak needs a non-trivial playable pool");

    let n: u64 = std::env::var("TSOT_STRESS_GAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);

    // One master stream draws every genome + per-game seed serially, so
    // the whole soak is reproducible from this one seed and a failing
    // game is pinned by the `game_seed` it prints.
    let mut master = StdRng::seed_from_u64(0x0057_8E55_5EED_0001);
    let ais = [AiKind::Stress, AiKind::Stress];

    for i in 0..n {
        let ga = random_genome(&pool, 50, 3, &mut master).expect("random genome A");
        let gb = random_genome(&pool, 50, 3, &mut master).expect("random genome B");
        // Build via `to_units`, not `to_deck` — `random_genome` drafts the
        // `__cardless__` sentinel (Z.8), which `to_units` materializes into
        // a real empty sleeve; `to_deck` chokes on it with UnknownCardId.
        let units_a = to_units(&registry, &ga).expect("to_units A");
        let units_b = to_units(&registry, &gb).expect("to_units B");
        let game_seed: u64 = master.gen();

        let mut state = GameState::from_units(units_a, units_b);
        state.replay_journal = Some(Journal::new());
        let mut rng = StdRng::seed_from_u64(game_seed);
        let mut log: Vec<String> = Vec::new();
        let _stats =
            run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, game_seed);

        assert!(
            state.winner.is_some(),
            "stress game {i} (game_seed=0x{game_seed:016x}) produced no winner — engine hang?",
        );

        // Sample rollback integrity. A full check every game roughly
        // doubles the soak; every 25th is enough to catch a broken
        // journal inverse under stress. Rollback must restore the
        // pre-game position, where no winner is set.
        if i % 25 == 0 {
            let journal = state.replay_journal.take().unwrap_or_default();
            journal.rollback(&mut state);
            assert!(
                state.winner.is_none(),
                "stress game {i} (game_seed=0x{game_seed:016x}): \
                 rollback did not clear the winner",
            );
        }
    }
}
