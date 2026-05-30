//! `GameStats` data shape + per-game bump helpers. The `run_game` loop in
//! [`super::run`] writes into a GameStats; [`super::aggregate`] reads
//! from many of them.

use std::collections::{BTreeMap, BTreeSet};
use tsot::card::EventName;
use tsot::game::PlayerId;

use super::variants::DeckVariant;

#[derive(Debug, Clone)]
pub struct GameStats {
    pub turns: u32,
    pub winner: PlayerId,
    pub variant_a: DeckVariant,
    pub variant_b: DeckVariant,
    /// Deck token encoding A's deck (Crockford base32, 16 chars).
    /// Sufficient to reproduce the deck from `(master_seed, side, v_a, v_b, game_index)`.
    pub token_a: String,
    /// Deck token encoding B's deck. Same format.
    pub token_b: String,
    /// Game-within-cell index (0-based). Forms part of the deck token.
    pub game_index: u32,
    /// Unique card IDs in A's starting deck. Same card repeated in the
    /// 50-card deck only counts once. Used for per-card win-rate analysis
    /// in the HTML report.
    pub deck_a_ids: BTreeSet<String>,
    pub deck_b_ids: BTreeSet<String>,
    /// Unique card IDs that actually got played at least once during the
    /// game (via the play loop). Compared against `deck_*_ids` to surface
    /// "was this card drawn-and-played" vs "just sitting in the deck."
    pub a_played_card_ids: BTreeSet<String>,
    pub b_played_card_ids: BTreeSet<String>,
    /// Per-card (min_turn, max_turn) the card was played by EITHER side.
    pub card_play_turns: BTreeMap<String, (u32, u32)>,
    /// Per-card count of "this card_id was sacrificed as a cost."
    pub card_sacrificed_count: BTreeMap<String, u32>,
    /// Per-card count of "this card_id was discarded via game.discard."
    /// Sourced from `GameState.action_counts` at game end (`discarded:<id>`
    /// prefix keys).
    pub card_discarded_count: BTreeMap<String, u32>,
    pub a_played: u32,
    pub b_played: u32,
    pub a_attacks: u32,
    pub b_attacks: u32,
    pub a_deaths: u32,
    pub b_deaths: u32,
    pub a_milled_to_exile: u32,
    pub b_milled_to_exile: u32,
    pub a_final_board: u32,
    pub b_final_board: u32,
    pub a_final_gy: u32,
    pub b_final_gy: u32,
    pub a_preview_attempts: u32,
    pub b_preview_attempts: u32,
    pub a_preview_rollbacks: u32,
    pub b_preview_rollbacks: u32,
    pub a_preview_journal_size_total: u64,
    pub b_preview_journal_size_total: u64,
    pub replay_journal_entries: u64,
    pub event_fires: BTreeMap<EventName, [u32; 2]>,
    pub action_counts: BTreeMap<String, [u32; 2]>,
}

pub fn bump_played(stats: &mut GameStats, p: PlayerId) {
    match p {
        PlayerId::A => stats.a_played += 1,
        PlayerId::B => stats.b_played += 1,
    }
}

pub fn bump_attacks(stats: &mut GameStats, p: PlayerId, n: u32) {
    match p {
        PlayerId::A => stats.a_attacks += n,
        PlayerId::B => stats.b_attacks += n,
    }
}

pub fn bump_milled(stats: &mut GameStats, defender: PlayerId, n: u32) {
    match defender {
        PlayerId::A => stats.a_milled_to_exile += n,
        PlayerId::B => stats.b_milled_to_exile += n,
    }
}

pub fn bump_preview_attempt(stats: &mut GameStats, p: PlayerId, journal_size: u64) {
    match p {
        PlayerId::A => {
            stats.a_preview_attempts += 1;
            stats.a_preview_journal_size_total += journal_size;
        }
        PlayerId::B => {
            stats.b_preview_attempts += 1;
            stats.b_preview_journal_size_total += journal_size;
        }
    }
}

pub fn bump_preview_rollback(stats: &mut GameStats, p: PlayerId) {
    match p {
        PlayerId::A => stats.a_preview_rollbacks += 1,
        PlayerId::B => stats.b_preview_rollbacks += 1,
    }
}
