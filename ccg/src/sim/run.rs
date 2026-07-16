// S12: `run_game_continue` is deprecated. This module's wrappers
// (`run_game`, `run_game_with_ai`) + its tests legitimately still call
// it until D8 retires the cli_serve legacy path. Suppress the
// deprecation warning at the file level rather than peppering every
// call site.
#![allow(deprecated)]

//! Per-game turn loop. Calls into [`super::ai`] for AI decisions, writes
//! into [`super::stats::GameStats`] as the game progresses, returns the
//! final stats + the game-long replay journal.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::card::{CardRegistry, CardType, CostSource};
use crate::choice::{ChoiceOracle, ChooseIntRequest, RandomOracle, RecordingOracle};
use crate::game::{EventContext, GameState, InstanceId, Phase, PlayChoices, PlayerId};

use super::ai::{
    attached_keep_value, pick_blocks, pick_heuristic_playable_in_hand,
    sacrifice_keep_value, select_attackers, PickKindFilter,
};
use super::stats::{
    bump_attacks, bump_milled, bump_played, bump_preview_attempt, bump_preview_rollback, GameStats,
};
use super::variants::DeckVariant;

/// Original entry point — takes ownership of state, manages its own
/// `replay_journal`, returns both stats and the captured journal. The
/// `--replay` workflow and existing callers depend on this shape.
///
/// Internally delegates to [`run_game_continue`]. New callers that
/// want to drive multiple games on the same state (MCTS rollouts,
/// Pick an ANSI 256-color index from a curated 16-color palette
/// keyed by the game's RNG seed. All `[HEARTBEAT]`, `[GAME TIMEOUT]`,
/// `[SLOW CAST]` lines from one game share the same color so the
/// operator can visually correlate interleaved parallel-EA output.
/// Deterministic: same seed → same color across runs.
fn game_color_for_seed(seed: u64) -> u8 {
    // Palette skips bright red (clashes with error output) and the
    // grayscale range (too dim on dark terminals). Covers cyan,
    // green, yellow, orange, purple, and light-blue families.
    const PALETTE: [u8; 16] = [
        39, 45, 51, 82, 118, 154, 190, 208, 214, 220, 165, 171, 177, 99, 105, 111,
    ];
    PALETTE[(seed as usize) % PALETTE.len()]
}

/// scripted multi-game tests, multiplayer rollback) use
/// `run_game_continue` directly with `&mut GameState`.
pub fn run_game(
    state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    registry: &std::sync::Arc<CardRegistry>,
    game_seed: u64,
) -> (GameStats, crate::game::Journal) {
    let ais = [super::AiKind::Game, super::AiKind::Game];
    run_game_with_ai(state, rng, log, registry, &ais, game_seed)
}

/// Like [`run_game`] but with per-player AI selection. Used by the
/// EA when opponents play MCTS (step 8 — `--opponent-ai mcts`) and
/// anywhere else that wants the wrapper's journal-lifecycle setup
/// without being locked to Heuristic-on-both-sides.
pub fn run_game_with_ai(
    mut state: GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    registry: &std::sync::Arc<CardRegistry>,
    ais: &[super::AiKind; 2],
    game_seed: u64,
) -> (GameStats, crate::game::Journal) {
    state.replay_journal = Some(crate::game::Journal::new());
    let mut stats = run_game_continue(&mut state, rng, log, registry, ais, game_seed);
    let replay_journal = state.replay_journal.take().unwrap_or_default();
    stats.replay_journal_entries = replay_journal.len() as u64;
    (stats, replay_journal)
}

/// Resumable game loop. Operates on `&mut GameState` without touching
/// the caller's `replay_journal` lifecycle — caller decides whether to
/// open one, whether to take/rollback at the end. Required entry point
/// for journal-based AI search (MCTS) and any scenario where the game
/// loop is one step inside a larger rollback-able operation.
///
/// Preconditions:
///   - `state` is in a runnable position (winner = None, phase + active
///     player consistent). New games via `GameState::new(...)` qualify;
///     mid-game states from a save also qualify.
///   - Caller has set `state.replay_journal` if they want a recording.
///     The function does NOT reset it.
///
/// The returned `GameStats.replay_journal_entries` field is left at
/// `0` — the wrapper that owns the journal lifecycle sets it. Bare
/// callers can set it themselves after taking the journal.
/// Outcome of `build_pattern_b_choices`. Variants beyond `Choices`
/// signal Pattern B should special-case (skip the play, advance the
/// loop, etc.).
pub(crate) enum BuildChoiceResult {
    /// PlayChoices ready to feed to `state.play_card`.
    Choices(PlayChoices),
    /// Picked an X-cost card and the X-pick computed max_x < 1 (no
    /// affordable X ≥ 1). Caller should advance Pattern B: skip the
    /// play, and mark `played_creature` to prevent re-picking if
    /// the candidate was a creature.
    UnaffordableX { picked_is_creature: bool },
    /// An oracle call returned `Err(ChoicePending)` — the human still
    /// owes the engine an answer. Caller (StepEngine S7) lifts this
    /// into a `NeedHuman` yield and retries `build_pattern_b_choices`
    /// once the answer is supplied via the replay history. The legacy
    /// `run_game_continue` path never hits this branch because
    /// `HumanAwareOracle` resolves human answers synchronously via the
    /// channel API.
    Pending(crate::choice::ChoicePending),
}

/// Build the same `PlayChoices` Pattern B builds inline today —
/// extracted so MCTS rollouts construct choices identically to the
/// heuristic AI (rather than its own simpler version that was
/// systematically underestimating candidates with non-trivial cost).
///
/// Mirrors the inline logic exactly:
///   - X-cost: cap by tightest resource, oracle picks X, resolve hand
///     payment (smart-pitch via oracle) + GY substitutes + GY-pay
///   - Creature / Spell / Artifact / Mutation: hand payment + GY pay
///     + mutation target selection
///   - Sacrifice slots (any card kind): low-value picker
///   - P.31 ATTACHED-source slots
///
/// Mutates `state` for sacrifice picking + activations are not
/// journaled directly here; the journaled helpers (set_*, move_card,
/// etc.) handle that, so MCTS rollouts can rollback the entire
/// build_pattern_b_choices + play_card sequence.
pub(crate) fn build_pattern_b_choices(
    state: &mut GameState,
    active: PlayerId,
    picked: &InstanceId,
    oracle: &mut dyn ChoiceOracle,
) -> BuildChoiceResult {
    let kind = state
        .card_pool
        .get(picked)
        .map(|c| c.card().kind)
        .unwrap_or(CardType::Unspecified);
    let picked_is_creature = matches!(kind, CardType::Creature);
    let mut choices = PlayChoices::default();
    let cost = state
        .card_pool
        .get(picked)
        .map(|c| c.card().cost.clone())
        .unwrap_or_default();
    let has_is_x = cost.iter().any(|c| c.is_x);

    if has_is_x {
        let p = state.player(active);
        let hand_size = p.hand.len();
        let deck_size = p.deck.len();
        let gy_size = p.graveyard.len();
        let board_creatures = p
            .board
            .iter()
            .filter(|iid| {
                state
                    .card_pool
                    .get(*iid)
                    .map(|i| i.card().kind == CardType::Creature)
                    .unwrap_or(false)
            })
            .count();
        let identity_count = state.eligible_hand_payments(active, picked).len();
        let gy_subs_available = p
            .graveyard
            .iter()
            .filter(|gid| {
                state
                    .card_pool
                    .get(*gid)
                    .map(|i| i.card().gy_hand_substitute)
                    .unwrap_or(false)
            })
            .count();
        // P.24a/c/e: how many HAND components a tap-substitution can
        // cover. P.24c caps a cast at one substitution mechanism, so
        // jewel and Symbol are mutually exclusive — take whichever
        // covers more. Jewel covers UP TO 2 components (mixed
        // HAND/GRAVEYARD, hand drained first); Symbol covers exactly
        // 1 (HAND or GRAVEYARD); crystal covers 1 HAND. For the X-cap
        // we use the HAND-side coverage only; the GY side is moot for
        // an X-hand-cost cast that has no GRAVEYARD component.
        let substitution_coverage = if let Some(sub_iid) =
            state.find_jewel_tap_candidate(active, picked)
        {
            // find_jewel_tap_candidate returns either a JEWEL (2-mixed)
            // or a CRYSTAL (1 HAND only). For the X-hand cap we only
            // care about the HAND-side coverage; crystal still covers
            // 1 HAND slot. Both differentiate the same way the engine
            // does at play.rs:285.
            let is_crystal = state
                .card_pool
                .get(&sub_iid)
                .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("crystal")))
                .unwrap_or(false);
            if is_crystal { 1 } else { 2 }
        } else if state.find_symbol_tap_candidate(active).is_some() {
            1
        } else {
            0
        };
        // P.12b identity-coverage gate (mirrored from play.rs:407-449):
        // when the cast has any identity and no Graveyard cost component
        // can supply a color-anchor, play_card requires hand_payment_ids
        // non-empty whenever gy_hand_payment_ids is non-empty. Filling
        // hand slots entirely with GY substitutes plus substitution
        // still trips the gate because substitution doesn't satisfy
        // identity. So:
        //   - identity_count >= 1: substitutes can extend X, since at
        //     least one hand_payment will be a real identity card.
        //   - identity_count == 0: X must be small enough that ZERO GY
        //     substitutes are needed. That means hand_needed (= X minus
        //     substitution_coverage) must be 0, i.e. X <= substitution_coverage.
        // gy_anchor (P.12b) suspends the gate, but only when the cast
        // has gy_need > 0 AND a color-matching GY card exists — only
        // applies to casts with a Graveyard cost component.
        let cast_ident = state.card_identity(picked);
        let has_gy_cost = cost
            .iter()
            .any(|c| matches!(c.source, CostSource::Graveyard));
        let cast_colors_lc: std::collections::BTreeSet<String> = state
            .card_pool
            .get(picked)
            .map(|i| {
                i.card()
                    .colors
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let gy_anchor_possible = has_gy_cost
            && !cast_colors_lc.is_empty()
            && p.graveyard.iter().any(|gid| {
                state
                    .card_pool
                    .get(gid)
                    .map(|i| {
                        i.card()
                            .colors
                            .iter()
                            .any(|c| cast_colors_lc.contains(&c.to_ascii_lowercase()))
                    })
                    .unwrap_or(false)
            });
        let mut caps: Vec<usize> = Vec::new();
        for c in &cost {
            if !c.is_x {
                continue;
            }
            match c.source {
                CostSource::Hand => {
                    let hand_avail = identity_count.min(hand_size.saturating_sub(1));
                    let gates_block_subs =
                        !cast_ident.is_empty() && identity_count == 0 && !gy_anchor_possible;
                    let subs_room = if gates_block_subs {
                        0
                    } else {
                        gy_subs_available
                    };
                    caps.push(hand_avail + substitution_coverage + subs_room);
                }
                CostSource::Mill => caps.push(deck_size),
                CostSource::Graveyard => caps.push(gy_size),
                CostSource::Sacrifice => caps.push(board_creatures),
                CostSource::SelfExile => {}
                // P.31 (X-attached): cap by how many attached cards
                // are actually eligible to pay. Without this, casts
                // with X-attached components (wither: X-hand +
                // X-graveyard + X-attached) let the oracle pick X
                // higher than attached availability, then build's
                // attached-fill returns 1 when play_card expects 2 —
                // WrongAttachedPaymentCount { expected: X, got: <X }.
                CostSource::Attached => {
                    caps.push(state.eligible_attached_payments(active, picked).len());
                }
            }
        }
        let mut max_x = caps.into_iter().min().unwrap_or(0).min(10) as i32;
        // P.12a anchor-feasibility cap on X. When the cast has a
        // GRAVEYARD cost component AND non-empty colors AND no
        // color-matching anchor exists in the player's graveyard,
        // the engine refuses any cast that ends up with
        // graveyard_needed > 0 at apply time (play.rs:467
        // NoGraveyardPaymentForColor). Without this cap, build picks
        // X higher than the substitution coverage can fully drain
        // (witnessed on read-the-embers: X-hand + X-graveyard + X-mill,
        // red; with a red jewel on board the picker treats is_x as 1
        // and credits jewel covering both 1-hand + 1-gy → playable,
        // but build then picks X=2 and at X=2 the jewel saturates on
        // hand, gy_needed stays at 2, anchor check fires). Simulate
        // the engine's coverage for each X candidate and pick the
        // largest X where post-coverage gy_needed = 0.
        if !gy_anchor_possible && has_gy_cost && !cast_colors_lc.is_empty() {
            let is_x_hand_count: usize = cost
                .iter()
                .filter(|c| c.is_x && matches!(c.source, CostSource::Hand))
                .count();
            let is_x_gy_count: usize = cost
                .iter()
                .filter(|c| c.is_x && matches!(c.source, CostSource::Graveyard))
                .count();
            // Determine substitution shape (matches engine's branch
            // selection at play.rs:285). Crystal covers HAND only;
            // jewel covers up to 2 mixed (hand drained first); symbol
            // covers 1 mixed (hand-or-gy, hand drained first); none.
            #[derive(Copy, Clone)]
            enum Sub {
                None,
                Crystal,
                Jewel,
                Symbol,
            }
            let sub = if let Some(sub_iid) = state.find_jewel_tap_candidate(active, picked) {
                let is_crystal = state
                    .card_pool
                    .get(&sub_iid)
                    .map(|i| {
                        i.card()
                            .subtypes
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case("crystal"))
                    })
                    .unwrap_or(false);
                if is_crystal {
                    Sub::Crystal
                } else {
                    Sub::Jewel
                }
            } else if state.find_symbol_tap_candidate(active).is_some() {
                Sub::Symbol
            } else {
                Sub::None
            };
            let mut anchor_x: i32 = 0;
            for try_x in 0..=max_x {
                let hand_n = is_x_hand_count * try_x as usize;
                let gy_n = is_x_gy_count * try_x as usize;
                let post_gy: usize = match sub {
                    Sub::Jewel => {
                        let take_h = hand_n.min(2);
                        let budget = 2 - take_h;
                        let take_g = gy_n.min(budget);
                        gy_n - take_g
                    }
                    Sub::Symbol => {
                        if hand_n > 0 {
                            gy_n
                        } else {
                            gy_n.saturating_sub(1)
                        }
                    }
                    Sub::Crystal | Sub::None => gy_n,
                };
                if post_gy == 0 {
                    anchor_x = try_x;
                } else {
                    break;
                }
            }
            max_x = max_x.min(anchor_x);
        }
        if max_x < 1 {
            return BuildChoiceResult::UnaffordableX { picked_is_creature };
        }
        let x = match oracle.choose_int(
            state,
            ChooseIntRequest {
                min: 1,
                max: max_x,
                prompt: format!("X for {}", short(picked)),
            },
        ) {
            Ok(x) => x,
            Err(pending) => return BuildChoiceResult::Pending(pending),
        };
        state.bump_action("choose_int", active);
        choices.x_value = Some(x);
        // BOTH is_x AND non-is_x hand components must be counted.
        // The X-branch used to only sum is_x components, silently
        // dropping cards like spectrum-cull (1 non-X hand + X gy + X
        // mill) — play_card sees the non-X slot, build doesn't fill
        // it, and the cycle hangs. Mirror the same shape gy uses
        // just below.
        let raw_hand_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Hand))
            .map(|c| {
                if c.is_x {
                    x.max(0) as usize
                } else {
                    c.amount.max(0) as usize
                }
            })
            .sum();
        // play_card applies cost_reduction(Hand) BEFORE checking
        // hand_payment_ids.len(); build must mirror or it over-fills
        // hand_payment_ids and play_card rejects with
        // WrongHandPaymentCount { expected: 0, got: N } when a static
        // (modern-lcd-clock) reduces the cost.
        let hand_red_x = state.cost_reduction(picked, CostSource::Hand).max(0) as usize;
        let mut hand_needed: usize = raw_hand_needed.saturating_sub(hand_red_x);
        let raw_gy_needed_x: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| {
                if c.is_x {
                    x.max(0) as usize
                } else {
                    c.amount.max(0) as usize
                }
            })
            .sum();
        let gy_red_x = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let mut gy_needed: usize = raw_gy_needed_x.saturating_sub(gy_red_x);
        // P.24a (rewritten) + P.24e: jewel covers up to 2 mixed
        // HAND/GRAVEYARD components, Symbol-tap covers 1 (HAND-or-GY)
        // when no jewel applies. Apply BEFORE filling hand_payment_ids
        // / gy_hand_payment_ids / graveyard_payment_ids so build
        // mirrors the engine's apply site at game/play.rs.
        if hand_needed > 0 || gy_needed > 0 {
            if let Some(sub) = state.find_jewel_tap_candidate(active, picked) {
                let is_crystal = state
                    .card_pool
                    .get(&sub)
                    .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("crystal")))
                    .unwrap_or(false);
                choices.jewel_tap = Some(sub);
                if is_crystal {
                    if hand_needed > 0 {
                        hand_needed = hand_needed.saturating_sub(1);
                    }
                } else {
                    let mut budget: usize = 2;
                    let take_h = hand_needed.min(budget);
                    hand_needed -= take_h;
                    budget -= take_h;
                    let take_g = gy_needed.min(budget);
                    gy_needed -= take_g;
                }
            } else if let Some(symbol) = state.find_symbol_tap_candidate(active) {
                choices.jewel_tap = Some(symbol);
                if hand_needed > 0 {
                    hand_needed -= 1;
                } else {
                    gy_needed -= 1;
                }
            }
        }
        if hand_needed > 0 {
            let mut remaining = hand_needed;
            if identity_count < remaining {
                let want_gy = remaining - identity_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                remaining -= used;
            }
            if remaining > 0 {
                choices.hand_payment_ids =
                    match state.resolve_hand_payment(active, picked, remaining, oracle) {
                        Ok(ids) => ids,
                        Err(pending) => return BuildChoiceResult::Pending(pending),
                    };
            }
        }
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
        }
    } else if matches!(kind, CardType::Creature) {
        let raw_hand_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Hand))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let hand_red = state.cost_reduction(picked, CostSource::Hand).max(0) as usize;
        let mut hand_needed = raw_hand_needed.saturating_sub(hand_red);
        let raw_gy_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let gy_red = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let mut gy_needed = raw_gy_needed.saturating_sub(gy_red);
        // P.24a (rewritten): jewel covers up to 2 components mixed
        // HAND/GRAVEYARD. Crystal (P.24b) covers exactly 1 HAND.
        // Differentiate by subtype to match engine's apply site at
        // play.rs:285. Drain HAND first then GRAVEYARD.
        if hand_needed > 0 || gy_needed > 0 {
            if let Some(sub) = state.find_jewel_tap_candidate(active, picked) {
                let is_crystal = state
                    .card_pool
                    .get(&sub)
                    .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("crystal")))
                    .unwrap_or(false);
                choices.jewel_tap = Some(sub);
                if is_crystal {
                    if hand_needed > 0 {
                        hand_needed = hand_needed.saturating_sub(1);
                    }
                } else {
                    let mut budget: usize = 2;
                    let take_h = hand_needed.min(budget);
                    hand_needed -= take_h;
                    budget -= take_h;
                    let take_g = gy_needed.min(budget);
                    gy_needed -= take_g;
                }
            } else if let Some(symbol) = state.find_symbol_tap_candidate(active) {
                // P.24e: single-component HAND-or-GY substitution
                // when no jewel/crystal takes the slot.
                choices.jewel_tap = Some(symbol);
                if hand_needed > 0 {
                    hand_needed -= 1;
                } else {
                    gy_needed -= 1;
                }
            }
        }
        if hand_needed > 0 {
            let identity_match_count = state.eligible_hand_payments(active, picked).len();
            if identity_match_count < hand_needed {
                let want_gy = hand_needed - identity_match_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                hand_needed -= used;
            }
        }
        // Z.8c: fill any remaining shortfall with cardless bodies from hand
        // (non-anchor, the cardless analogue of GY substitutes). Identity
        // casts only — a wildcard cast draws cardless through
        // resolve_hand_payment's own pool. Affordability guarantees
        // identity_match_count >= 1 here, so this always leaves >=1 slot
        // for resolve_hand_payment to fill with a real anchor; the bundle
        // is never all-cardless (which play_card would reject).
        if hand_needed > 0 && !state.card_identity(picked).is_empty() {
            let identity_match_count = state.eligible_hand_payments(active, picked).len();
            if identity_match_count < hand_needed {
                let want = hand_needed - identity_match_count;
                let bodies = state.find_cardless_hand_bodies(
                    active,
                    picked,
                    &choices.hand_payment_ids,
                    want,
                );
                let used = bodies.len();
                choices.hand_payment_ids.extend(bodies);
                hand_needed -= used;
            }
        }
        if hand_needed > 0 {
            match state.resolve_hand_payment(active, picked, hand_needed, oracle) {
                Ok(ids) => choices.hand_payment_ids.extend(ids),
                Err(pending) => return BuildChoiceResult::Pending(pending),
            }
        }
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
        }
        // No rig: creatures pay their printed cost via the same path
        // as any other card kind. The earlier `rig_creature_free_haste`
        // shortcut (free-cast + auto-haste for AI sides without a
        // setup cost) was a sim handicap, not a rule, and is gone.
    } else if matches!(
        kind,
        CardType::Spell | CardType::Artifact | CardType::Mutation | CardType::Unspecified
    ) {
        let raw_hand_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Hand))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let hand_red = state.cost_reduction(picked, CostSource::Hand).max(0) as usize;
        let mut hand_needed = raw_hand_needed.saturating_sub(hand_red);
        let raw_gy_needed: usize = cost
            .iter()
            .filter(|c| matches!(c.source, CostSource::Graveyard))
            .map(|c| c.amount.max(0) as usize)
            .sum();
        let gy_red = state.cost_reduction(picked, CostSource::Graveyard).max(0) as usize;
        let mut gy_needed = raw_gy_needed.saturating_sub(gy_red);
        // P.24a (rewritten) + P.24b (crystal: 1 HAND) + P.24e (Symbol).
        // Differentiate jewel vs crystal by subtype — engine treats
        // them differently at play.rs:285 and a picker/builder that
        // doesn't will produce NoGraveyardPaymentForColor on cards
        // like witch-bat (1-hand + 1-gy) against a same-color crystal.
        if hand_needed > 0 || gy_needed > 0 {
            if let Some(sub) = state.find_jewel_tap_candidate(active, picked) {
                let is_crystal = state
                    .card_pool
                    .get(&sub)
                    .map(|i| i.card().subtypes.iter().any(|s| s.eq_ignore_ascii_case("crystal")))
                    .unwrap_or(false);
                choices.jewel_tap = Some(sub);
                if is_crystal {
                    if hand_needed > 0 {
                        hand_needed = hand_needed.saturating_sub(1);
                    }
                } else {
                    let mut budget: usize = 2;
                    let take_h = hand_needed.min(budget);
                    hand_needed -= take_h;
                    budget -= take_h;
                    let take_g = gy_needed.min(budget);
                    gy_needed -= take_g;
                }
            } else if let Some(symbol) = state.find_symbol_tap_candidate(active) {
                choices.jewel_tap = Some(symbol);
                if hand_needed > 0 {
                    hand_needed -= 1;
                } else {
                    gy_needed -= 1;
                }
            }
        }
        if hand_needed > 0 {
            let identity_match_count = state.eligible_hand_payments(active, picked).len();
            if identity_match_count < hand_needed {
                let want_gy = hand_needed - identity_match_count;
                let gy_subs = state.find_gy_hand_substitutes(active, picked, want_gy);
                let used = gy_subs.len();
                choices.gy_hand_payment_ids = gy_subs;
                hand_needed -= used;
            }
        }
        // Z.8c: fill any remaining shortfall with cardless bodies from hand
        // (non-anchor, the cardless analogue of GY substitutes). Identity
        // casts only — a wildcard cast draws cardless through
        // resolve_hand_payment's own pool. Affordability guarantees
        // identity_match_count >= 1 here, so this always leaves >=1 slot
        // for resolve_hand_payment to fill with a real anchor; the bundle
        // is never all-cardless (which play_card would reject).
        if hand_needed > 0 && !state.card_identity(picked).is_empty() {
            let identity_match_count = state.eligible_hand_payments(active, picked).len();
            if identity_match_count < hand_needed {
                let want = hand_needed - identity_match_count;
                let bodies = state.find_cardless_hand_bodies(
                    active,
                    picked,
                    &choices.hand_payment_ids,
                    want,
                );
                let used = bodies.len();
                choices.hand_payment_ids.extend(bodies);
                hand_needed -= used;
            }
        }
        if hand_needed > 0 {
            match state.resolve_hand_payment(active, picked, hand_needed, oracle) {
                Ok(ids) => choices.hand_payment_ids.extend(ids),
                Err(pending) => return BuildChoiceResult::Pending(pending),
            }
        }
        if gy_needed > 0 {
            choices.graveyard_payment_ids =
                state.resolve_graveyard_payment(active, picked, gy_needed);
        }
        if matches!(kind, CardType::Mutation) {
            // Same eligibility set as the picker uses (and that
            // play_card validates) — no picker/resolver disagreement
            // on mutation targets possible.
            let mut pool = state.eligible_mutation_targets(picked);
            pool.sort_by(|a, b| {
                let key = |t: &InstanceId| {
                    let inst = state.card_pool.get(t);
                    let own = inst.map(|i| i.controller == active).unwrap_or(false);
                    let x = state.effective_stats(t).0;
                    (if own { 0 } else { 1 }, -x)
                };
                let (ko_a, kx_a) = key(a);
                let (ko_b, kx_b) = key(b);
                ko_a.cmp(&ko_b)
                    .then_with(|| kx_a.partial_cmp(&kx_b).unwrap_or(std::cmp::Ordering::Equal))
            });
            choices.mutation_target = pool.first().cloned();
        }
    }

    // Sacrifice slots (any kind): pick lowest-value first.
    // is_x components scale with the chosen X value (set just above
    // in the X-branch); non-X components use c.amount. Without
    // handling is_x, casts like `reckoning` (X sacrifices) silently
    // skip sac-slot creation, play_card rejects with
    // WrongSacrificeCount, and the pick/resolve loops.
    let x_value = choices.x_value.unwrap_or(0).max(0) as usize;
    let sacrifice_slots: Vec<Option<CardType>> = cost
        .iter()
        .filter(|c| matches!(c.source, CostSource::Sacrifice))
        .flat_map(|c| {
            let n = if c.is_x {
                x_value
            } else {
                c.amount.max(0) as usize
            };
            std::iter::repeat_n(c.kind, n)
        })
        .collect();
    if !sacrifice_slots.is_empty() {
        let mut used: std::collections::BTreeSet<InstanceId> = std::collections::BTreeSet::new();
        for required_kind in sacrifice_slots {
            let mut sac_candidates: Vec<InstanceId> = state
                .player(active)
                .board
                .iter()
                .filter(|iid| !used.contains(*iid))
                .filter(|iid| !state.has_keyword(iid, "can't be sacrificed"))
                .filter(|iid| {
                    if let Some(k) = required_kind {
                        state
                            .card_pool
                            .get(*iid)
                            .map(|i| i.card().kind == k)
                            .unwrap_or(false)
                    } else {
                        true
                    }
                })
                .cloned()
                .collect();
            sac_candidates.sort_by_key(|iid| sacrifice_keep_value(state, iid));
            if let Some(pick) = sac_candidates.into_iter().next() {
                used.insert(pick.clone());
                choices.sacrifice_ids.push(pick);
            }
        }
    }

    // P.31 ATTACHED-source.
    let raw_attached_need: usize = cost
        .iter()
        .filter(|c| matches!(c.source, CostSource::Attached))
        .map(|c| {
            if c.is_x {
                choices.x_value.unwrap_or(0).max(0) as usize
            } else {
                c.amount.max(0) as usize
            }
        })
        .sum();
    let att_red = state.cost_reduction(picked, CostSource::Attached).max(0) as usize;
    let attached_need = raw_attached_need.saturating_sub(att_red);
    if attached_need > 0 {
        // Use the shared eligibility helper so the picker's
        // attached_have count and the resolver's actual pool never
        // disagree on which attached iids may pay this cast. (C.14's
        // frame gate is lifted; the helper returns all controlled
        // attached cards.)
        let mut pool = state.eligible_attached_payments(active, picked);
        pool.sort_by_key(|aid| attached_keep_value(state, aid));
        pool.truncate(attached_need);
        choices.attached_payment_ids = pool;
    }

    BuildChoiceResult::Choices(choices)
}

/// S12: scheduled for deletion. `StepEngine::run_to_end` is the
/// canonical drive loop (`run_game` / `run_game_with_ai` are still
/// here as thin wrappers that pre-open the replay journal). The
/// remaining caller is the `cli_serve` legacy HTTP path — when D8
/// deletes that, `run_game_continue` follows. Internally still owns
/// the channel-blocking Human path; the StepEngine drives human
/// games via the FFI yield protocol instead, so any new caller
/// should use `StepEngine::run_to_end` (AI-only) or
/// One AI-decision the per-game wall-clock instrumentation captures.
/// Used by `PickTiming` so the GAME TIMEOUT dump can answer "where
/// did the 30s go?" — was it many ordinary picks, or one giant one?
#[derive(Debug, Clone)]
struct PickTimingEntry {
    turn: u32,
    active: PlayerId,
    /// What site fired the pick — `pattern_b` for the main-phase
    /// pick loop, future variants could include `activation` /
    /// `attack` / `block` as those get instrumented.
    site: &'static str,
    /// `card.id` of the chosen card (most-recently-resolved pick),
    /// `"pass"` when the AI declined.
    card_id: String,
    wall_us: u128,
}

#[derive(Debug, Default)]
struct PickTiming {
    total_picks: u32,
    total_wall_us: u128,
    /// Top-10 slowest entries, kept sorted descending by wall_us.
    top_slowest: Vec<PickTimingEntry>,
}

impl PickTiming {
    fn record(&mut self, entry: PickTimingEntry) {
        self.total_picks += 1;
        self.total_wall_us += entry.wall_us;
        self.top_slowest.push(entry);
        self.top_slowest.sort_by_key(|e| std::cmp::Reverse(e.wall_us));
        self.top_slowest.truncate(10);
    }
}

/// `wasm_ffi`'s session driver (human-mixed).
#[deprecated(
    since = "0.1.0",
    note = "use `StepEngine::run_to_end` for AI-only games; the wasm_ffi session driver for \
            human-mixed games. `run_game_continue` survives only because cli_serve.rs's \
            HTTP-shim thread+channel architecture depends on it; D8 retires both together."
)]
pub fn run_game_continue(
    state: &mut GameState,
    rng: &mut StdRng,
    log: &mut Vec<String>,
    registry: &std::sync::Arc<CardRegistry>,
    ais: &[super::AiKind; 2],
    game_seed: u64,
) -> GameStats {
    let lua = registry.lua();
    let oracle_seed: u64 = rng.gen();
    // If either side is Human, wrap the random oracle so its choose_*
    // calls route to the human when the asker matches.
    let human_pair: Option<(PlayerId, std::sync::Arc<super::human::HumanInterface>)> = ais
        .iter()
        .enumerate()
        .find_map(|(idx, ai)| match ai {
            super::AiKind::Human(iface) => Some((
                if idx == 0 { PlayerId::A } else { PlayerId::B },
                iface.clone(),
            )),
            _ => None,
        });
    let mut oracle = RecordingOracle::new(super::human::HumanAwareOracle::new(
        RandomOracle::new(StdRng::seed_from_u64(oracle_seed)),
        human_pair,
    ));

    let mut stats = GameStats {
        turns: 0,
        winner: PlayerId::A,
        game_seed,
        variant_a: DeckVariant::Ra,
        variant_b: DeckVariant::Rb,
        token_a: String::new(),
        token_b: String::new(),
        game_index: 0,
        deck_a_ids: BTreeSet::new(),
        deck_b_ids: BTreeSet::new(),
        a_played_card_ids: BTreeSet::new(),
        b_played_card_ids: BTreeSet::new(),
        card_play_turns: BTreeMap::new(),
        card_play_turn_events: Vec::new(),
        card_sacrificed_count: BTreeMap::new(),
        card_discarded_count: BTreeMap::new(),
        a_played: 0,
        b_played: 0,
        a_attacks: 0,
        b_attacks: 0,
        a_deaths: 0,
        b_deaths: 0,
        a_milled_to_exile: 0,
        b_milled_to_exile: 0,
        a_final_board: 0,
        b_final_board: 0,
        a_final_gy: 0,
        b_final_gy: 0,
        a_preview_attempts: 0,
        b_preview_attempts: 0,
        a_preview_rollbacks: 0,
        b_preview_rollbacks: 0,
        a_preview_journal_size_total: 0,
        b_preview_journal_size_total: 0,
        replay_journal_entries: 0,
        event_fires: BTreeMap::new(),
        action_counts: BTreeMap::new(),
    };

    // Per-game wall-clock watchdog. Default 600s (10 min) — the
    // per-pick wall budget (UctConfig::per_pick_wall_ms) is the
    // SEARCH-space cap that actually matters; this outer cap is
    // just an insurance against a genuine engine hang (Lua handler
    // infinite loop, rollout-stall failing to fire, etc.). Sum of
    // pick-budgets bounds the legitimate game cost, so a real game
    // can be slow but the outer guard catches stuck runs that no
    // amount of search budget would unstick. Tunable via
    // `TSOT_GAME_TIMEOUT_SECS`.
    let timeout = std::env::var("TSOT_GAME_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(600));
    // Per-game ANSI color so parallel-EA stderr can be visually
    // demuxed. Drawn from one `rng.gen()` so it's reproducible from
    // the outer EA seed. All heartbeat / timeout / slow-cast lines
    // from this game wear this color via the `color_print!` macro.
    let game_color: u8 = game_color_for_seed(rng.gen());
    let game_start = Instant::now();
    // Reset per turn so the timeout report identifies the actual offending
    // card, not a stale pick from an earlier successful turn.
    let mut last_picked: Option<InstanceId> = None;
    let mut last_activated: Option<(InstanceId, usize)> = None;
    // Per-pick wall-clock accumulator. Surfaced in the GAME TIMEOUT
    // dump so the operator can see whether the 30s went to "many
    // ordinary picks" (legit UCT cost) vs "one or two giant picks"
    // (a state where a single pick balloons — usually a specific card
    // multiplier or a search-space explosion). Without this the
    // dumps only show terminal state, not where the seconds went.
    let mut pick_timing = PickTiming::default();
    // Heartbeat: log progress every 5s for games that don't finish
    // promptly. Helps identify slow-but-not-hung games before the
    // wall-clock cap fires.
    let mut last_heartbeat = game_start;

    let mut safety = 1000;
    while state.winner.is_none() && safety > 0 {
        if game_start.elapsed() > timeout {
            report_game_timeout(
                state,
                "outer turn loop",
                game_seed,
                last_picked.as_ref(),
                last_activated.as_ref(),
                Some(&pick_timing),
                game_color,
            );
            state.set_winner(Some(state.active_player.opponent()), "watchdog_outer_loop");
            break;
        }
        // Heartbeat: 30s cadence. Per-pick wall is hard-capped, so
        // genuinely stuck games surface via [GAME TIMEOUT] / [SLOW
        // CAST] rather than heartbeat absence; 30s keeps the EA's
        // parallel stderr readable.
        if last_heartbeat.elapsed() > Duration::from_secs(30) {
            eprintln!(
                "\x1b[38;5;{}m[HEARTBEAT] elapsed={:.1?} turn={} phase={:?} active={:?} \
                 A_board={} B_board={} A_deck={} B_deck={} chain={}\x1b[0m",
                game_color,
                game_start.elapsed(),
                state.turn,
                state.phase,
                state.active_player,
                state.a.board.len(),
                state.b.board.len(),
                state.a.deck.len(),
                state.b.deck.len(),
                state.priority.as_ref().map(|p| p.chain.len()).unwrap_or(0),
            );
            last_heartbeat = Instant::now();
        }
        safety -= 1;
        let active = state.active_player;
        let turn = state.turn;
        last_picked = None;
        last_activated = None;
        let mut events: Vec<String> = Vec::new();
        crate::sim::instrument::set_current_op(format!(
            "turn {turn} ({active:?}) phase={:?} outer-loop-tick",
            state.phase
        ));

        while state.phase != Phase::Main1 && state.winner.is_none() {
            crate::sim::instrument::set_current_op(format!(
                "turn {turn} ({active:?}) next_phase from {:?}",
                state.phase
            ));
            let mut oracle = RandomOracle::new(StdRng::seed_from_u64(rng.gen()));
            state
                .next_phase(Some(&mut EventContext::new(lua, &mut oracle)))
                .expect("autonomous EA uses RandomOracle which never yields ChoicePending");
        }
        if state.winner.is_some() {
            crate::sim::instrument::tee_log(log, format!("turn {turn} ({active:?}): deck-out before Main1"));
            break;
        }

        // Multi-card-per-turn (Pattern B): at most one creature per turn,
        // but as many non-creatures as the AI can afford. Inner safety
        // cap (`pattern_b_iter`) catches "picker keeps returning the
        // same card → `continue` → re-pick" infinite loops (e.g., an
        // X-cost spell that affords pick + is unaffordable at X-pick
        // resolution, in lockstep with another bug). 200 iterations is
        // a generous ceiling — a real turn never plays more than ~10.
        let mut played_creature = false;
        let mut pattern_b_iter: u32 = 0;
        loop {
            pattern_b_iter += 1;
            if pattern_b_iter > 200 {
                report_game_timeout(
                    state,
                    "Pattern B inner loop",
                    game_seed,
                    last_picked.as_ref(),
                    last_activated.as_ref(),
                    Some(&pick_timing),
                    game_color,
                );
                state.set_winner(Some(state.active_player.opponent()), "watchdog_pattern_b_iter");
                break;
            }
            if game_start.elapsed() > timeout {
                report_game_timeout(
                    state,
                    "Pattern B inner loop (wall-clock)",
                    game_seed,
                    last_picked.as_ref(),
                    last_activated.as_ref(),
                    Some(&pick_timing),
                    game_color,
                );
                state.set_winner(Some(state.active_player.opponent()), "watchdog_pattern_b_walltime");
                break;
            }
            if state.winner.is_some() {
                break;
            }
            let kind_filter = if played_creature {
                PickKindFilter::NonCreatureOnly
            } else {
                PickKindFilter::Any
            };
            // Per-player AI: each player can run its own decision policy.
            // Default (Heuristic for both) is byte-identical to pre-MCTS
            // behavior; mixed Heuristic/Mcts is how matchup-mcts compares.
            crate::sim::instrument::set_current_op(format!(
                "turn {turn} ({active:?}) phase={:?} pick_play ai={:?} kind_filter={:?}",
                state.phase, ais[active.index()], kind_filter
            ));
            let pick_t0 = Instant::now();
            let pick = match &ais[active.index()] {
                super::AiKind::Game | super::AiKind::Fast | super::AiKind::Stress => {
                    // The shared no-search policy. UCT iteration mode:
                    // while a search is running, pick_play calls (on
                    // either side) consume planned actions from the UCT
                    // plan first. When the plan runs out, fall back to
                    // the random-weighted heuristic picker. The UCT
                    // search ALWAYS rolls out with `AiKind::Game` (the
                    // override is the steering wheel).
                    if let Some(planned) = super::uct::take_planned_action() {
                        Some(planned)
                    } else {
                        pick_heuristic_playable_in_hand(state, active, rng, kind_filter)
                    }
                }
                super::AiKind::Mcts(mcts_cfg) => {
                    super::mcts::pick_play(state, active, kind_filter, mcts_cfg, registry)
                }
                super::AiKind::Uct(uct_cfg) => {
                    // Deprecated run_game_continue path: the UCT trace
                    // is for the wasm UI log only; native runs discard it.
                    super::uct::pick_play_uct(state, active, kind_filter, uct_cfg, registry).0
                }
                super::AiKind::Human(iface) => {
                    let candidates =
                        super::ai::enumerate_playable_in_hand(state, active, kind_filter);
                    let activations = enumerate_human_activations(state, active);
                    let iface = iface.clone();
                    match iface.main_phase_choice(state, active, candidates, kind_filter, activations) {
                        super::human::MainPhaseChoice::Pass => None,
                        super::human::MainPhaseChoice::Play(iid) => Some(iid),
                        super::human::MainPhaseChoice::Activate { iid, ability_index, x } => {
                            last_activated = Some((iid.clone(), ability_index));
                            // Fire the activation. Failures are logged
                            // and the loop continues — the user might
                            // pick another action.
                            if let Err(e) = state.activate_ability(
                                &iid,
                                ability_index,
                                x,
                                // Slice #4: SACRIFICE/SELF in activated cost.
                                // HumanAction::Activate doesn't yet carry
                                // sacrifice_ids; SACRIFICE-cost activations
                                // from the human side return
                                // WrongSacrificeCount until the UI plumbs
                                // sacrifice picks. SELF-cost activations
                                // need no extra input — the source is the
                                // implicit cost.
                                crate::game::ActivateChoices::default(),
                                Some(&mut EventContext::new(lua, &mut oracle)),
                            ) {
                                crate::sim::instrument::tee_log(log, format!(
                                    "turn {turn} ({active:?}): human activation {iid}[{ability_index}] failed: {e:?}"
                                ));
                                // Sacred-error: previously the error
                                // only landed in the sim log. Route
                                // through the typed Error pipeline
                                // so a wasm-UI surface sees it.
                                crate::error::emit_region(
                                    crate::error::Severity::Error,
                                    "engine",
                                    "activate-failed",
                                    format!("activation rejected for {iid}[{ability_index}]"),
                                    format!("turn {turn} ({active:?}): {e:?}"),
                                );
                            }
                            continue;
                        }
                    }
                }
            };
            let pick_wall_us = pick_t0.elapsed().as_micros();
            // Record the wall-clock of THIS pick (and the chosen card
            // id, or "pass" if the AI declined) so the GAME TIMEOUT
            // dump can attribute the 30s budget. Done before the
            // filter so the time measured is what the AI actually
            // spent deciding, not what the engine spent validating.
            let pick_card_id = pick
                .as_ref()
                .and_then(|iid| state.card_pool.get(iid).map(|c| c.card().id.clone()))
                .unwrap_or_else(|| "pass".to_string());
            pick_timing.record(PickTimingEntry {
                turn,
                active,
                site: "pattern_b",
                card_id: pick_card_id,
                wall_us: pick_wall_us,
            });
            // See parallel comment in sim/step/main_phases.rs: when the
            // pick came from an inner search (UCT/MCTS) that mutated
            // state and rolled back imperfectly, the chosen iid may
            // no longer be affordable in the current state. Re-validate
            // before committing to build/play_card.
            let pick = pick.filter(|iid| {
                state.player(active).hand.contains(iid)
                    && super::ai::can_pay_instant_cost(state, active, iid)
            });
            let Some(picked) = pick else {
                break;
            };
            last_picked = Some(picked.clone());
            let picked_is_creature = state
                .card_pool
                .get(&picked)
                .map(|c| c.card().kind == CardType::Creature)
                .unwrap_or(false);
            {
                let kind = state
                    .card_pool
                    .get(&picked)
                    .map(|c| c.card().kind)
                    .unwrap_or(CardType::Unspecified);
                // Build PlayChoices via the shared choice-builder (same
                // function MCTS rollouts use). Sacrifice-stats bumping
                // stays here (Pattern B's concern, not the builder's).
                let build_result = build_pattern_b_choices(
                    state,
                    active,
                    &picked,
                    &mut oracle,
                );
                let choices = match build_result {
                    BuildChoiceResult::Choices(c) => c,
                    BuildChoiceResult::UnaffordableX { picked_is_creature: pic } => {
                        if pic {
                            played_creature = true;
                        } else {
                            break;
                        }
                        continue;
                    }
                    BuildChoiceResult::Pending(p) => {
                        // run_game_continue uses HumanAwareOracle which
                        // resolves human answers synchronously via the
                        // channel API — it can't return Pending. This
                        // branch is reachable only if a future caller
                        // wires a yielding oracle here.
                        panic!(
                            "run_game_continue: unexpected ChoicePending from HumanAwareOracle: {p:?}"
                        );
                    }
                };
                // Update sacrifice telemetry from the picked ids.
                for sac_iid in &choices.sacrifice_ids {
                    if let Some(card_id) = state.card_pool.get(sac_iid).map(|c| c.card().id.clone()) {
                        *stats.card_sacrificed_count.entry(card_id).or_insert(0) += 1;
                    }
                }
                oracle.clear();
                state.journal = Some(crate::game::Journal::new());
                let opponent_of_active = active.opponent();
                // Per-cast wall-clock tripwire: a single cast resolving for >1s
                // suggests a Lua handler in a tight loop (no Rust-side watchdog
                // covers handler-internal time).
                let cast_start = Instant::now();
                let result = state.play_card(
                    active,
                    &picked,
                    choices,
                    Some(&mut EventContext::new(lua, &mut oracle)),
                );
                let cast_elapsed = cast_start.elapsed();
                if cast_elapsed > Duration::from_secs(1) {
                    let card_id = state
                        .card_pool
                        .get(&picked)
                        .map(|c| c.card().id.clone())
                        .unwrap_or_else(|| format!("?{picked}"));
                    eprintln!(
                        "\x1b[38;5;{}m[SLOW CAST] turn={} active={:?} card={} elapsed={:.2?} result={:?}\x1b[0m",
                        game_color, state.turn, active, card_id, cast_elapsed, result,
                    );
                    state.bump_action("slow_cast", active);
                }
                let suicide = state.winner == Some(opponent_of_active);
                let preview_size = state.journal.as_ref().map(|j| j.len()).unwrap_or(0) as u64;

                bump_preview_attempt(&mut stats, active, preview_size);

                if result.is_ok() && !suicide {
                    if let Some(mut preview) = state.journal.take() {
                        if let Some(replay) = state.replay_journal.as_mut() {
                            replay.extend_from(&mut preview);
                        }
                    }
                    bump_played(&mut stats, active);
                    if let Some(card_id) =
                        state.card_pool.get(&picked).map(|c| c.card().id.clone())
                    {
                        match active {
                            PlayerId::A => {
                                stats.a_played_card_ids.insert(card_id.clone());
                            }
                            PlayerId::B => {
                                stats.b_played_card_ids.insert(card_id.clone());
                            }
                        }
                        let turn_now = state.turn;
                        stats
                            .card_play_turns
                            .entry(card_id.clone())
                            .and_modify(|(min_t, max_t)| {
                                if turn_now < *min_t {
                                    *min_t = turn_now;
                                }
                                if turn_now > *max_t {
                                    *max_t = turn_now;
                                }
                            })
                            .or_insert((turn_now, turn_now));
                        // Full distribution (vs the (min, max) summary
                        // above) — feeds the turn-curve aggregation in
                        // `tsot curve-sample` → `cards-report.py`.
                        // Player kept so a future per-deck analysis
                        // can group; today's consumer ignores it.
                        stats
                            .card_play_turn_events
                            .push((card_id, turn_now, active));
                    }
                    let timing = state.card_pool.get(&picked).and_then(|c| c.card().timing);
                    let label = match kind {
                        CardType::Spell => match timing {
                            Some(crate::Timing::Instant) => format!("instant {}", short(&picked)),
                            Some(crate::Timing::Sorcery) => format!("sorcery {}", short(&picked)),
                            None => format!("spell {}", short(&picked)),
                        },
                        _ => {
                            let (x, y) = state.effective_stats(&picked);
                            format!("{} ({x}/{y})", short(&picked))
                        }
                    };
                    events.push(format!("played {label}"));
                    if picked_is_creature {
                        played_creature = true;
                    }
                } else {
                    if let Some(journal) = state.journal.take() {
                        journal.rollback(state);
                    }
                    bump_preview_rollback(&mut stats, active);
                    if suicide {
                        state.bump_action("preview_skip_suicide", active);
                    }
                    // Surface the failure reason so the UI / log isn't
                    // blind to silent rollbacks. AI rollbacks happen
                    // often (preview-and-skip is the design); human-side
                    // rollbacks should be rare and worth flagging.
                    if let Err(err) = result {
                        let card_id = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card().id.clone())
                            .unwrap_or_else(|| picked.clone());
                        let active_is_human = matches!(ais[active.index()], super::AiKind::Human(_));
                        crate::sim::instrument::tee_log(log, format!(
                            "turn {turn} ({active:?}): play_card({card_id}) failed: {err:?}{}",
                            if active_is_human { " [HUMAN — visible failure]" } else { "" }
                        ));
                    } else if suicide {
                        let card_id = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card().id.clone())
                            .unwrap_or_else(|| picked.clone());
                        crate::sim::instrument::tee_log(log, format!(
                            "turn {turn} ({active:?}): {card_id} rolled back (would have lost the game)"
                        ));
                    }
                    if picked_is_creature {
                        played_creature = true;
                    } else {
                        break;
                    }
                }
            }
        }

        // Pre-combat activation pass: fire activated abilities on
        // non-creature board cards (artifacts, mostly jewels). Drawing
        // before combat lets the AI know its hand for the rest of the
        // turn. Creatures hold their activations until post-combat so
        // tapping for an ability doesn't pre-empt an attack.
        let pre_acts =
            run_activation_pass(state, active, lua, &mut oracle, true, &mut last_activated, ais);
        if pre_acts > 0 {
            events.push(format!("{pre_acts} pre-combat activation(s)"));
        }

        while state.phase != Phase::Combat && state.winner.is_none() {
            let mut oracle = RandomOracle::new(StdRng::seed_from_u64(rng.gen()));
            state
                .next_phase(Some(&mut EventContext::new(lua, &mut oracle)))
                .expect("autonomous EA uses RandomOracle which never yields ChoicePending");
        }
        if state.winner.is_some() {
            if !events.is_empty() {
                crate::sim::instrument::tee_log(log, format!("turn {turn} ({active:?}): {}", events.join("; ")));
            }
            break;
        }

        let defender = active.opponent();
        let attackers: Vec<InstanceId> = match &ais[active.index()] {
            super::AiKind::Game
            | super::AiKind::Fast
            | super::AiKind::Stress
            | super::AiKind::Mcts(_)
            | super::AiKind::Uct(_) => select_attackers(state, active),
            super::AiKind::Human(iface) => {
                let eligible = super::ai::eligible_attackers(state, active);
                iface.pick_attackers(state, active, eligible)
            }
        };
        let mut declared_atk_count = 0u32;
        for atk in &attackers {
            if state
                .declare_attacker(atk, Some(&mut EventContext::new(lua, &mut oracle)))
                .is_ok()
            {
                declared_atk_count += 1;
            }
        }

        if declared_atk_count > 0 {
            state.confirm_attacks().unwrap();
            let assignments = match &ais[defender.index()] {
                super::AiKind::Game
                | super::AiKind::Fast
                | super::AiKind::Stress
                | super::AiKind::Mcts(_)
                | super::AiKind::Uct(_) => pick_blocks(state, defender),
                super::AiKind::Human(iface) => {
                    use crate::game::CombatState;
                    let declared: Vec<InstanceId> = match &state.combat {
                        Some(CombatState::AwaitingBlockers { attacks }) => {
                            attacks.iter().map(|a| a.attacker.clone()).collect()
                        }
                        _ => Vec::new(),
                    };
                    let eligible = super::ai::eligible_blockers(state, defender);
                    iface.pick_blocks(state, defender, declared, eligible)
                }
            };
            let mut block_count = 0u32;
            for (blk, atk) in &assignments {
                if state
                    .declare_blocker(blk, atk, Some(&mut EventContext::new(lua, &mut oracle)))
                    .is_ok()
                {
                    block_count += 1;
                }
            }
            let outcome = state
                .confirm_blocks(Some(&mut EventContext::new(lua, &mut oracle)))
                .unwrap();
            bump_attacks(&mut stats, active, declared_atk_count);
            bump_milled(&mut stats, defender, outcome.defender_milled_to_exile as u32);
            for death in &outcome.deaths {
                if state.card_pool.get(death).map(|i| i.owner) == Some(PlayerId::A) {
                    stats.a_deaths += 1;
                } else {
                    stats.b_deaths += 1;
                }
            }
            events.push(format!(
                "{declared_atk_count} attackers, {block_count} blockers → mill {}, {} deaths",
                outcome.defender_milled_to_exile,
                outcome.deaths.len()
            ));
        } else if events.is_empty() {
            events.push("no play, no attack".to_string());
        }

        // Post-combat activation pass: anything still untapped that can
        // activate, fires now. Vigilance creatures that swung this turn
        // are still untapped here — this is where vigilant-human draws.
        // Pre-combat non-creature activations already tapped those, so
        // they're naturally excluded by `can_activate`.
        let post_acts =
            run_activation_pass(state, active, lua, &mut oracle, false, &mut last_activated, ais);
        if post_acts > 0 {
            events.push(format!("{post_acts} post-combat activation(s)"));
        }

        // Human-side Main2 prompt loop. Engine has a Main2 phase but
        // the turn-progression code skips through it without a prompt
        // — fine for AI (which auto-activated post-combat), wrong for
        // human. Run a Pattern-B-style loop here: plays + activations
        // until the human passes.
        if let super::AiKind::Human(iface) = &ais[active.index()] {
            let iface = iface.clone();
            // Advance into Main2 explicitly so play_card timing checks
            // (sorcery-speed) accept the cast.
            while state.phase != Phase::Main2 && state.winner.is_none() {
                let mut oracle = RandomOracle::new(StdRng::seed_from_u64(rng.gen()));
                state
                .next_phase(Some(&mut EventContext::new(lua, &mut oracle)))
                .expect("autonomous EA uses RandomOracle which never yields ChoicePending");
                if matches!(state.phase, Phase::Untap | Phase::Draw) {
                    // We've already wrapped past End — bail.
                    break;
                }
            }
            let mut m2_played_creature = false;
            loop {
                if state.winner.is_some() {
                    break;
                }
                let kind_filter = if m2_played_creature {
                    PickKindFilter::NonCreatureOnly
                } else {
                    PickKindFilter::Any
                };
                let candidates =
                    super::ai::enumerate_playable_in_hand(state, active, kind_filter);
                let activations = enumerate_human_activations(state, active);
                if candidates.is_empty() && activations.is_empty() {
                    break;
                }
                match iface.main_phase_choice(
                    state,
                    active,
                    candidates,
                    kind_filter,
                    activations,
                ) {
                    super::human::MainPhaseChoice::Pass => break,
                    super::human::MainPhaseChoice::Activate { iid, ability_index, x } => {
                        last_activated = Some((iid.clone(), ability_index));
                        if let Err(e) = state.activate_ability(
                            &iid,
                            ability_index,
                            x,
                            // Slice #4: see Main1 call site above for the
                            // SACRIFICE-from-human-side caveat. Same gap
                            // applies on the Main2 side.
                            crate::game::ActivateChoices::default(),
                            Some(&mut EventContext::new(lua, &mut oracle)),
                        ) {
                            crate::sim::instrument::tee_log(log, format!(
                                "turn {turn} ({active:?}): main2 activation {iid}[{ability_index}] failed: {e:?}"
                            ));
                            crate::error::emit_region(
                                crate::error::Severity::Error,
                                "engine",
                                "activate-failed",
                                format!("Main2 activation rejected for {iid}[{ability_index}]"),
                                format!("turn {turn} ({active:?}): {e:?}"),
                            );
                        }
                    }
                    super::human::MainPhaseChoice::Play(picked) => {
                        let picked_is_creature = state
                            .card_pool
                            .get(&picked)
                            .map(|c| c.card().kind == CardType::Creature)
                            .unwrap_or(false);
                        let build_result = build_pattern_b_choices(
                            state, active, &picked, &mut oracle,
                        );
                        let choices = match build_result {
                            BuildChoiceResult::Choices(c) => c,
                            BuildChoiceResult::UnaffordableX { .. } => continue,
                            BuildChoiceResult::Pending(p) => panic!(
                                "run_game_continue Main2: unexpected ChoicePending: {p:?}"
                            ),
                        };
                        oracle.clear();
                        let result = state.play_card(
                            active,
                            &picked,
                            choices,
                            Some(&mut EventContext::new(lua, &mut oracle)),
                        );
                        if let Err(err) = result {
                            let card_id = state
                                .card_pool
                                .get(&picked)
                                .map(|c| c.card().id.clone())
                                .unwrap_or_else(|| picked.clone());
                            crate::sim::instrument::tee_log(log, format!(
                                "turn {turn} ({active:?}): main2 play_card({card_id}) failed: {err:?}"
                            ));
                        } else if picked_is_creature {
                            m2_played_creature = true;
                        }
                    }
                }
            }
        }

        crate::sim::instrument::tee_log(log, format!("turn {turn} ({active:?}): {}", events.join("; ")));

        let starting_turn = state.turn;
        while state.turn == starting_turn && state.winner.is_none() {
            let mut oracle = RandomOracle::new(StdRng::seed_from_u64(rng.gen()));
            state
                .next_phase(Some(&mut EventContext::new(lua, &mut oracle)))
                .expect("autonomous EA uses RandomOracle which never yields ChoicePending");
        }
    }

    stats.turns = state.turn;
    stats.winner = state.winner.unwrap_or(PlayerId::A);
    stats.a_final_board = state.a.board.len() as u32;
    stats.b_final_board = state.b.board.len() as u32;
    stats.a_final_gy = state.a.graveyard.len() as u32;
    stats.b_final_gy = state.b.graveyard.len() as u32;
    stats.event_fires = state.event_fires.clone();
    stats.action_counts = state.action_counts.clone();
    for (key, counts) in &state.action_counts {
        if let Some(cid) = key.strip_prefix("discarded:") {
            let total = counts[0] + counts[1];
            *stats
                .card_discarded_count
                .entry(cid.to_string())
                .or_insert(0) += total;
        }
    }
    // `replay_journal` lifecycle is the caller's responsibility — the
    // wrapper [`run_game`] takes + sets `stats.replay_journal_entries`;
    // MCTS rollouts take and rollback instead. We just return stats.
    stats
}

/// Build the activatable-abilities list a human main-phase prompt
/// surfaces. Walks every zone where the player's cards could declare
/// an activation (board, hand, graveyard, exile, deck, attached-of-
/// any-host). Each card's activations are filtered by `can_activate`,
/// which honors the ability's declared `from_zones`.
pub(crate) fn enumerate_human_activations(
    state: &GameState,
    player: PlayerId,
) -> Vec<super::human::ActivationOption> {
    let mut out = Vec::new();
    // Candidate iids: every card the player controls across every zone,
    // plus every attached card whose host is on either player's board
    // (an attached card's controller can fire its from-attached ability
    // regardless of which side the host is on).
    let p = state.player(player);
    let mut candidates: Vec<InstanceId> = Vec::new();
    candidates.extend(p.board.iter().cloned());
    candidates.extend(p.hand.iter().cloned());
    candidates.extend(p.graveyard.iter().cloned());
    candidates.extend(p.exile.iter().cloned());
    candidates.extend(p.deck.iter().cloned());
    for (_iid, inst) in state.card_pool.iter() {
        for aid in &inst.attached {
            if let Some(ainst) = state.card_pool.get(aid) {
                if ainst.controller == player && !candidates.contains(aid) {
                    candidates.push(aid.clone());
                }
            }
        }
    }
    for iid in &candidates {
        let n = state.activation_count(iid);
        if n == 0 {
            continue;
        }
        let card_name = state
            .card_pool
            .get(iid)
            .map(|i| i.card().name.clone())
            .unwrap_or_else(|| iid.clone());
        for idx in 0..n {
            if !state.can_activate(iid, idx) {
                continue;
            }
            let (text, needs_x) = state
                .activation_at(iid, idx)
                .map(|a| (a.text.clone(), a.cost_components.iter().any(|c| c.is_x)))
                .unwrap_or_else(|| (format!("ability {idx}"), false));
            out.push(super::human::ActivationOption {
                iid: iid.clone(),
                card_name: card_name.clone(),
                ability_index: idx,
                text,
                needs_x,
            });
        }
    }
    out
}

/// Fire activated abilities the player can currently afford. Walks
/// their board, considers each card's first activatable ability, and
/// activates it if eligible. `non_creatures_only = true` restricts the
/// pass to non-creature cards (used pre-combat, so creatures stay free
/// for attack decisions). Returns the number of activations fired.
pub(crate) fn run_activation_pass(
    state: &mut GameState,
    player: PlayerId,
    lua: &mlua::Lua,
    oracle: &mut dyn ChoiceOracle,
    non_creatures_only: bool,
    last_activated: &mut Option<(InstanceId, usize)>,
    ais: &[super::AiKind; 2],
) -> u32 {
    // Activated abilities require the controller's opt-in. AI sides
    // keep auto-firing (preserves heuristic behavior + test
    // determinism). Human side never enters this pass — `Activate`
    // is a main-phase action the player drives explicitly.
    let active_is_human = matches!(ais[player.index()], super::AiKind::Human(_));
    if active_is_human {
        return 0;
    }
    let mut count = 0u32;
    // Snapshot board ids up front. Activation handlers can mutate the
    // board (move cards in/out), so we re-validate membership and
    // re-fetch instance data on each iteration.
    let ids: Vec<InstanceId> = state.player(player).board.clone();
    for iid in &ids {
        let is_creature = match state.card_pool.get(iid) {
            Some(inst) => inst.card().kind == CardType::Creature,
            None => continue,
        };
        // RULES A.5+: total activations = printed (`card.activated`)
        // plus static-granted (`StaticDef.granted_activated`).
        let abilities_n = state.activation_count(iid);
        if abilities_n == 0 {
            continue;
        }
        if non_creatures_only && is_creature {
            continue;
        }
        // Activate the first eligible ability. Tap is consumed by the
        // first activation so additional abilities on the same card
        // won't pass `can_activate` until next untap.
        for idx in 0..abilities_n {
            if !state.can_activate(iid, idx) {
                continue;
            }
            // AI sides keep the auto-fire behavior. Human side never
            // auto-fires — the human drives activations explicitly via
            // the main-phase `Activate` action. Skip the entire loop
            // for human (we exit via `return 0` below).
            let _ = active_is_human;
            // For X-cost activations, the AI commits to a concrete X
            // before calling. Simple heuristic: spend ~half the hand
            // (rounded up) so we keep some cards in hand. Bigger X
            // when hand is large; minimum X=1.
            let needs_x = state
                .activation_at(iid, idx)
                .map(|a| a.cost_components.iter().any(|c| c.is_x))
                .unwrap_or(false);
            let x_value = if needs_x {
                let hand_size = state.player(player).hand.len() as i32;
                // Heuristic: spend up to half the hand, capped at 5,
                // minimum 1. Refined later — this is a v1 stake.
                Some(((hand_size + 1) / 2).clamp(1, 5))
            } else {
                None
            };
            *last_activated = Some((iid.clone(), idx));
            if state
                .activate_ability(
                    iid,
                    idx,
                    x_value,
                    // Slice #4: AI auto-fire passes empty sacrifice_ids.
                    // Abilities with SACRIFICE cost return
                    // WrongSacrificeCount and the AI skips them. Wiring
                    // the AI to pick a sacrifice target (lowest-value
                    // creature heuristic) is a follow-up; today
                    // Reincubator + 156's SACRIFICE-cost activations
                    // simply aren't exercised by the EA / probe loops.
                    crate::game::ActivateChoices::default(),
                    Some(&mut EventContext::new(lua, oracle)),
                )
                .is_ok()
            {
                count += 1;
                break;
            }
        }
    }
    count
}

/// Dump game state to stderr when the wall-clock watchdog or inner-loop
/// safety cap trips. Reports active player, turn, zone contents (by
/// `card.id`, not InstanceId — readable correlation to `cards/*.lua`),
/// most recent card picked, and most recent activation. Operator greps
/// stderr for `[GAME TIMEOUT]` to identify the suspect card.
fn report_game_timeout(
    state: &GameState,
    site: &str,
    game_seed: u64,
    last_picked: Option<&InstanceId>,
    last_activated: Option<&(InstanceId, usize)>,
    pick_timing: Option<&PickTiming>,
    game_color: u8,
) {
    let _ = crate::game::bump_timeout_and_maybe_halt(site);
    let ids = |iids: &[InstanceId]| -> Vec<String> {
        iids.iter()
            .filter_map(|i| state.card_pool.get(i).map(|c| c.card().id.clone()))
            .collect()
    };
    let card_id_of = |iid: &InstanceId| -> String {
        state
            .card_pool
            .get(iid)
            .map(|c| c.card().id.clone())
            .unwrap_or_else(|| format!("?{iid}"))
    };
    // Per-game color prefix/suffix so every dump line wears the
    // same color as the heartbeats from this game — operator can
    // visually demux interleaved parallel-EA output.
    let c = format!("\x1b[38;5;{game_color}m");
    let z = "\x1b[0m";
    eprintln!(
        "{c}[GAME TIMEOUT] site={site} game_seed=0x{game_seed:016x} \
         turn={} active={:?} winner={:?}{z}",
        state.turn, state.active_player, state.winner,
    );
    eprintln!(
        "{c}  reproduce: run_game(state, &mut StdRng::seed_from_u64(0x{game_seed:016x}), \
         &mut log, registry, 0x{game_seed:016x}){z}",
    );
    if let Some(p) = last_picked {
        eprintln!("{c}  last_picked: {} ({}){z}", p, card_id_of(p));
    }
    if let Some((iid, idx)) = last_activated {
        eprintln!("{c}  last_activated: {} ({}) ability_idx={idx}{z}", iid, card_id_of(iid));
    }
    eprintln!("{c}  A hand: {:?}{z}", ids(&state.a.hand));
    eprintln!("{c}  A board: {:?}{z}", ids(&state.a.board));
    eprintln!("{c}  A graveyard: {:?}{z}", ids(&state.a.graveyard));
    eprintln!("{c}  B hand: {:?}{z}", ids(&state.b.hand));
    eprintln!("{c}  B board: {:?}{z}", ids(&state.b.board));
    eprintln!("{c}  B graveyard: {:?}{z}", ids(&state.b.graveyard));
    // Attached observability: P.6 hand-cost pitches and P.26 mutations
    // live under their host. Without these in the dump, "card came
    // out as X/Y but the cost was 3-hand" left attached payments
    // invisible. Print one line per host-with-attached so the dump
    // shows the full per-zone state, not just card.id lists.
    let attached_lines = |label: &str, board: &[InstanceId]| {
        for host_iid in board {
            let Some(host) = state.card_pool.get(host_iid) else { continue };
            if host.attached.is_empty() {
                continue;
            }
            let host_id = host.card().id.clone();
            let att: Vec<String> = host.attached.iter().map(card_id_of).collect();
            eprintln!("{c}  {label} attached: {host_id} <- {att:?}{z}");
        }
    };
    attached_lines("A", &state.a.board);
    attached_lines("B", &state.b.board);
    eprintln!(
        "{c}  decks: A={} B={} | priority_chain={}{z}",
        state.a.deck.len(),
        state.b.deck.len(),
        state.priority.as_ref().map(|p| p.chain.len()).unwrap_or(0),
    );
    // Per-pick wall-clock breakdown so the operator can see WHERE
    // the 30s went. Many small picks (~tens of ms each) summing to
    // 30s implies UCT cost is just at the limit on this state; a
    // handful of giant picks (~seconds each) implies a specific
    // state/cast is exploding the search. The top-10 slowest
    // entries pin which (turn, side, card) ate the budget.
    if let Some(t) = pick_timing {
        if t.total_picks > 0 {
            let total_ms = (t.total_wall_us / 1000) as u64;
            let avg_ms = (t.total_wall_us / t.total_picks as u128 / 1000) as u64;
            let slowest_ms = t
                .top_slowest
                .first()
                .map(|e| (e.wall_us / 1000) as u64)
                .unwrap_or(0);
            eprintln!(
                "{c}  picks: {} total, {} ms cumulative, {} ms avg, slowest {} ms{z}",
                t.total_picks, total_ms, avg_ms, slowest_ms,
            );
            for (i, e) in t.top_slowest.iter().take(10).enumerate() {
                eprintln!(
                    "{c}    #{:>2} turn {:>2} {:?} {} card={} {:>6} ms{z}",
                    i + 1,
                    e.turn,
                    e.active,
                    e.site,
                    e.card_id,
                    (e.wall_us / 1000) as u64,
                );
            }
        }
    }
}

pub fn short(iid: &InstanceId) -> String {
    let parts: Vec<&str> = iid.splitn(3, ':').collect();
    if parts.len() == 3 {
        format!("{}:{}", parts[0], parts[2])
    } else {
        iid.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardRegistry;

    /// A vanilla 50-card deck of the simplest creature in `cards/`.
    /// Used by seed/determinism tests that need a runnable game without
    /// caring about specific card effects.
    fn vanilla_deck(registry: &CardRegistry) -> Vec<crate::card::Card> {
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.len() == 1
                    && !c.cost[0].is_x
            })
            .expect("a vanilla creature should exist in cards/")
            .clone();
        (0..50).map(|_| template.clone()).collect()
    }

    /// `Game` / `Fast` / `Stress` are three intent-named views of one
    /// shared no-search picker — the split is metadata, not behaviour.
    /// Same seed + same decks must produce a byte-identical game across
    /// all three, or "behaviour-preserving" is a lie and a call site
    /// silently got a different opponent.
    #[test]
    fn game_fast_stress_are_behaviourally_identical() {
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let deck = vanilla_deck(&registry);
        let seed: u64 = 0x1DEA_5EED_0000_0007;

        let run = |kind: super::super::AiKind| {
            let mut state = GameState::new(deck.clone(), deck.clone());
            state.replay_journal = Some(crate::game::Journal::new());
            let mut rng = StdRng::seed_from_u64(seed);
            let mut log: Vec<String> = Vec::new();
            let ais = [kind.clone(), kind];
            let stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, seed);
            (format!("{:?}", stats.winner), stats.turns, log)
        };

        let g = run(super::super::AiKind::Game);
        let f = run(super::super::AiKind::Fast);
        let s = run(super::super::AiKind::Stress);
        assert_eq!(g, f, "Game and Fast diverged — the split is not behaviour-preserving");
        assert_eq!(g, s, "Game and Stress diverged — the split is not behaviour-preserving");
    }

    /// The [GAME TIMEOUT] dump can only reproduce a hung game if the
    /// game's seed is faithfully surfaced. Guard: the seed `run_game`
    /// records must be the seed that actually drove the game — reseeding
    /// a fresh run from `stats.game_seed` must reproduce the outcome. A
    /// caller that threads a `game_seed` not matching its rng fails here.
    #[test]
    fn game_seed_is_recorded_and_reproduces_the_game() {
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let deck = vanilla_deck(&registry);
        let seed: u64 = 0x5EED_1234_ABCD_0001;

        let state1 = GameState::new(deck.clone(), deck.clone());
        let mut rng1 = StdRng::seed_from_u64(seed);
        let mut log1: Vec<String> = Vec::new();
        let (stats1, _) = run_game(state1, &mut rng1, &mut log1, &registry, seed);

        assert_eq!(
            stats1.game_seed, seed,
            "recorded game_seed must equal the seed that drove the game",
        );

        // Reproduce strictly from the reported seed.
        let state2 = GameState::new(deck.clone(), deck.clone());
        let mut rng2 = StdRng::seed_from_u64(stats1.game_seed);
        let mut log2: Vec<String> = Vec::new();
        let (stats2, _) = run_game(state2, &mut rng2, &mut log2, &registry, stats1.game_seed);

        assert_eq!(
            format!("{:?}", stats1.winner),
            format!("{:?}", stats2.winner),
            "reported game_seed did not reproduce the winner",
        );
        assert_eq!(
            stats1.turns, stats2.turns,
            "reported game_seed did not reproduce the turn count",
        );
    }

    /// EA prerequisite: fitness(genome) is meaningful only if
    /// `run_game(state, &mut rng, &mut log, lua)` produces byte-identical
    /// outputs for byte-identical inputs. If this ever fails, the EA's
    /// generation-to-generation signal is noise and everything downstream
    /// is chasing it.
    #[test]
    fn sim_pays_attached_cost_with_real_baseline_decks() {
        // Play N games using the project's curated baseline decks
        // against themselves. These are EA-evolved/tournament-vetted
        // 50-card decks with realistic cost variety. Assert at least
        // one P.31 attached-payment fires across the matchup.
        use crate::sim::evolved_deck::EvolvedDeck;
        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let baseline_dir = std::path::Path::new("baselines");
        let mut decks: Vec<Vec<crate::card::Card>> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(baseline_dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match EvolvedDeck::load(&p) {
                Ok(saved) => match saved.to_cards(&registry) {
                    Ok(cards) => {
                        labels.push(p.file_name().unwrap().to_string_lossy().into_owned());
                        decks.push(cards);
                    }
                    Err(e) => eprintln!("baseline {} to_cards err: {e}", p.display()),
                },
                Err(e) => eprintln!("baseline {} load err: {e}", p.display()),
            }
        }
        assert!(!decks.is_empty(), "expected at least one baseline deck");
        eprintln!("loaded baselines: {:?}", labels);
        // Play each baseline against each other (including mirror) for
        // a few seeds. One mirror match per deck minimum.
        let mut total_transfer: u32 = 0;
        let mut total_exile: u32 = 0;
        for (i, deck_a) in decks.iter().enumerate() {
            for (j, deck_b) in decks.iter().enumerate() {
                let state = GameState::new(deck_a.clone(), deck_b.clone());
                let seed = 0xBA5E0000_u64 + (i as u64) * 100 + (j as u64);
                let mut rng = StdRng::seed_from_u64(seed);
                let mut log: Vec<String> = Vec::new();
                let (stats, _journal) =
                    run_game(state, &mut rng, &mut log, &registry, seed);
                total_transfer += stats
                    .action_counts
                    .get("attached_payment_transfer")
                    .map(|v| v[0] + v[1])
                    .unwrap_or(0);
                total_exile += stats
                    .action_counts
                    .get("attached_payment_exile")
                    .map(|v| v[0] + v[1])
                    .unwrap_or(0);
            }
        }
        eprintln!(
            "P.31 baseline matchups: {} games, transfer={} exile={}",
            decks.len() * decks.len(),
            total_transfer,
            total_exile,
        );
        assert!(
            total_transfer + total_exile > 0,
            "expected ≥1 P.31 attached-payment across {} baseline matchups. transfer={total_transfer} exile={total_exile}",
            decks.len() * decks.len()
        );
    }

    // X-cost cap must credit Symbol-tap (P.24e) alongside jewel-tap.
    // hydra-shaped scenario: X-hand cost cast in hand, no identity-
    // matching hand cards, no gy_anchor possible, but a Symbol on
    // BOARD untapped. The picker credits Symbol-tap for affordability
    // at X=1; the builder's X-branch cap (sim/run.rs ~line 233) used
    // to only credit jewel_coverage, so max_x came out 0 and build
    // returned UnaffordableX — picker/build asymmetry surfaced in
    // make evolve UCT rollouts as the turn-2 hydra timeout.
    #[test]
    fn build_pattern_b_choices_x_cap_credits_symbol_tap_substitution() {
        use crate::card::{CardType, CostComponent, CostSource};
        use crate::choice::RandomOracle;
        use rand::SeedableRng;
        let registry = std::sync::Arc::new(
            CardRegistry::load(std::path::Path::new("cards")).unwrap(),
        );
        let _ = registry; // load just to ensure registry path exists
        // Minimal hand: just the cast card so the X-branch's hand_size
        // logic doesn't accidentally credit identity. Use the helper
        // pattern from game::test_helpers::deck_of.
        let card_fn = |id: &str| -> crate::card::Card {
            crate::card::Card {
                id: id.to_string(),
                name: String::new(),
                colors: Vec::new(),
                kind: CardType::Creature,
                timing: None,
                subtypes: Vec::new(),
                cannot_block_subtypes: Vec::new(),
                can_block_subtypes: Vec::new(),
                symbols: Vec::new(),
                frame: None,
                holes: Vec::new(),
                symbol_slots: std::collections::BTreeMap::new(),
                color_slots: std::collections::BTreeMap::new(),
                face: Vec::new(),
                cost: Vec::new(),
                abilities: Vec::new(),
                flavor: String::new(),
                stats: Some(crate::card::Stats { x: 1.0, y: 1.0 }),
                static_def: None,
                handlers: std::collections::BTreeMap::new(),
                activated: Vec::new(),
                gy_hand_substitute: false,
                allow_x_zero: false,
                same_sleeve: false,
                target: None,
                is_variant: false,
                variant_of: None,
            }
        };
        let deck_a: Vec<crate::card::Card> = (0..60).map(|i| card_fn(&format!("a-{i}"))).collect();
        let deck_b: Vec<crate::card::Card> = (0..60).map(|i| card_fn(&format!("b-{i}"))).collect();
        let mut s = GameState::new(deck_a, deck_b);
        let active = PlayerId::A;
        // Re-shape A's hand[0] to a hydra-like X-cost green creature
        // (any color works — picker rejects with empty identity hand).
        let cast = s.player(active).hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&cast).unwrap();
            inst.card_mut().colors = vec!["green".to_string()];
            inst.card_mut().cost = vec![CostComponent {
                amount: 0,
                source: CostSource::Hand,
                is_x: true,
                kind: None,
            }];
        }
        // Promote A's hand[1] to a Symbol on BOARD untapped — covers
        // exactly one HAND component via P.24e.
        let symbol = s.player(active).hand[1].clone();
        {
            let inst = s.card_pool.get_mut(&symbol).unwrap();
            inst.card_mut().kind = CardType::Symbol;
            inst.card_mut().colors = vec!["blue".to_string()];
        }
        s.player_mut(active).hand.retain(|x| x != &symbol);
        s.player_mut(active).board.push(symbol.clone());
        s.card_pool.get_mut(&symbol).unwrap().tapped = false;
        // Run the build path. With symbol_coverage missing the
        // X-branch returns UnaffordableX; with it credited, max_x≥1
        // and the oracle's choose_int produces Choices.
        let mut rng = rand::rngs::StdRng::seed_from_u64(0xC0DE);
        let mut oracle = RandomOracle::new(&mut rng);
        let result = build_pattern_b_choices(&mut s, active, &cast, &mut oracle);
        let accepted = matches!(result, BuildChoiceResult::Choices(_));
        assert!(
            accepted,
            "build_pattern_b_choices must accept hydra-shape X-hand with a Symbol on board (P.24e)",
        );
    }

    // P.12a anchor-feasibility cap on X. read-the-embers shape:
    // X-hand + X-graveyard red spell. With a same-color jewel on
    // board and NO red card in the player's graveyard (no anchor),
    // build's X-cap used to pick X higher than the jewel could
    // fully drain — at X=2 the jewel saturates on hand and gy_need
    // stays at 2, engine fires NoGraveyardPaymentForColor. The
    // anchor-feasibility cap bounds max_x to the largest X where
    // post-coverage gy_need = 0 (X=1 with a jewel on this shape;
    // jewel covers 1 hand + 1 gy).
    #[test]
    fn build_pattern_b_choices_x_cap_respects_p12a_anchor_when_no_anchor_in_gy() {
        use crate::card::{CardType, CostComponent, CostSource};
        use crate::choice::RandomOracle;
        use rand::SeedableRng;
        let registry = std::sync::Arc::new(
            CardRegistry::load(std::path::Path::new("cards")).unwrap(),
        );
        let _ = registry;
        let card_fn = |id: &str| -> crate::card::Card {
            crate::card::Card {
                id: id.to_string(),
                name: String::new(),
                colors: Vec::new(),
                kind: CardType::Creature,
                timing: None,
                subtypes: Vec::new(),
                cannot_block_subtypes: Vec::new(),
                can_block_subtypes: Vec::new(),
                symbols: Vec::new(),
                frame: None,
                holes: Vec::new(),
                symbol_slots: std::collections::BTreeMap::new(),
                color_slots: std::collections::BTreeMap::new(),
                face: Vec::new(),
                cost: Vec::new(),
                abilities: Vec::new(),
                flavor: String::new(),
                stats: Some(crate::card::Stats { x: 1.0, y: 1.0 }),
                static_def: None,
                handlers: std::collections::BTreeMap::new(),
                activated: Vec::new(),
                gy_hand_substitute: false,
                allow_x_zero: false,
                same_sleeve: false,
                target: None,
                is_variant: false,
                variant_of: None,
            }
        };
        let deck_a: Vec<_> = (0..60).map(|i| card_fn(&format!("a-{i}"))).collect();
        let deck_b: Vec<_> = (0..60).map(|i| card_fn(&format!("b-{i}"))).collect();
        let mut s = GameState::new(deck_a, deck_b);
        let active = PlayerId::A;
        // Cast: X-hand + X-graveyard red spell (read-the-embers shape).
        let cast = s.player(active).hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&cast).unwrap();
            inst.card_mut().kind = CardType::Spell;
            inst.card_mut().colors = vec!["red".to_string()];
            inst.card_mut().cost = vec![
                CostComponent {
                    amount: 0,
                    source: CostSource::Hand,
                    is_x: true,
                    kind: None,
                },
                CostComponent {
                    amount: 0,
                    source: CostSource::Graveyard,
                    is_x: true,
                    kind: None,
                },
            ];
        }
        // Grab all iids BEFORE moving anything so indexes are stable.
        let jewel = s.player(active).hand[1].clone();
        let red_hand_a = s.player(active).hand[2].clone();
        let red_hand_b = s.player(active).hand[3].clone();
        let gy_seed_a = s.player(active).hand[4].clone();
        // Red jewel on A's board → substitution_coverage = 2.
        {
            let j = s.card_pool.get_mut(&jewel).unwrap();
            j.card_mut().kind = CardType::Artifact;
            j.card_mut().subtypes = vec!["jewel".to_string()];
            j.card_mut().colors = vec!["red".to_string()];
        }
        s.player_mut(active).hand.retain(|x| x != &jewel);
        s.player_mut(active).board.push(jewel.clone());
        s.card_pool.get_mut(&jewel).unwrap().tapped = false;
        // Identity-matching hand cards so the picker doesn't refuse at
        // the hand-identity gate (we're testing the X-cap, not identity).
        s.card_pool.get_mut(&red_hand_a).unwrap().card_mut().colors = vec!["red".to_string()];
        s.card_pool.get_mut(&red_hand_b).unwrap().card_mut().colors = vec!["red".to_string()];
        // Seed graveyard with a non-red card (no anchor possible).
        s.card_pool.get_mut(&gy_seed_a).unwrap().card_mut().colors = vec!["blue".to_string()];
        s.player_mut(active).hand.retain(|x| x != &gy_seed_a);
        s.player_mut(active).graveyard.push(gy_seed_a);
        let mut rng = rand::rngs::StdRng::seed_from_u64(0xC0DE);
        let mut oracle = RandomOracle::new(&mut rng);
        let result = build_pattern_b_choices(&mut s, active, &cast, &mut oracle);
        match result {
            BuildChoiceResult::Choices(choices) => {
                let x = choices.x_value.unwrap_or(0);
                assert!(
                    x <= 1,
                    "build must bound X to at most 1 (jewel covers 1 hand + 1 gy) when no anchor exists in graveyard; got X={x}",
                );
            }
            BuildChoiceResult::UnaffordableX { .. } => {
                // Also acceptable — if build determines no X works.
            }
            _ => panic!("unexpected build result variant"),
        }
    }

    /// Strongest journal-rollback invariant: open the replay journal at
    /// game start, run a FULL random game to completion via
    /// `run_game_continue`, then rollback — final state must equal
    /// initial state byte-for-byte.
    ///
    /// `tests/journal_full_game_rollback.rs` exercises the scripted
    /// 3-turn version using only the public engine API. This in-crate
    /// test uses `run_game_continue` directly to cover the full
    /// Pattern B / X-cost / response window flow that MCTS rollouts
    /// actually trip into.
    ///
    /// If this test fails, some mutation site is not journaled. MCTS
    /// rollout safety depends on this rollback being byte-identical;
    /// the test diff identifies which field (and thus mutation site)
    /// is the gap.
    #[test]
    fn full_random_game_rollback_restores_initial_state() {
        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        // Pick a vanilla creature template so the deck contains
        // handler-free cards — the test exercises journaling for the
        // hot-path Pattern B / combat / turn machinery, not for one
        // specific card's Lua side effects.
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.len() == 1
                    && !c.cost[0].is_x
            })
            .expect("a vanilla creature should exist in cards/")
            .clone();
        let deck_a: Vec<crate::card::Card> = (0..50).map(|_| template.clone()).collect();
        let deck_b: Vec<crate::card::Card> = (0..50).map(|_| template.clone()).collect();

        let mut state = GameState::new(deck_a, deck_b);

        // Clone the initial state for per-field comparison after rollback.
        // Cloning a 100-card pool is expensive but acceptable for a
        // one-shot diagnostic test.
        let initial = state.clone();

        state.replay_journal = Some(crate::game::Journal::new());

        let mut rng = StdRng::seed_from_u64(0xC0FFEE);
        let mut log: Vec<String> = Vec::new();
        let ais = [super::super::AiKind::Fast, super::super::AiKind::Fast];
        let stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, 0xC0FFEE);

        assert!(state.winner.is_some(), "game should have a winner");
        assert!(stats.turns > 0, "stats should record turns");

        let journal = state
            .replay_journal
            .take()
            .expect("replay_journal still open");
        let entry_count = journal.len();
        assert!(entry_count > 0, "a full game should produce many journal entries");
        journal.rollback(&mut state);

        // Compare each top-level field; on a card_pool mismatch, walk
        // per-instance and emit the first divergent Sleeve field.
        assert_eq!(format!("{:?}", initial.active_player), format!("{:?}", state.active_player), "active_player not rolled back");
        assert_eq!(initial.turn, state.turn, "turn not rolled back");
        assert_eq!(format!("{:?}", initial.phase), format!("{:?}", state.phase), "phase not rolled back");
        assert_eq!(format!("{:?}", initial.winner), format!("{:?}", state.winner), "winner not rolled back");
        assert_eq!(format!("{:?}", initial.combat), format!("{:?}", state.combat), "combat not rolled back");
        assert_eq!(format!("{:?}", initial.event_fires), format!("{:?}", state.event_fires), "event_fires not rolled back");
        assert_eq!(format!("{:?}", initial.action_counts), format!("{:?}", state.action_counts), "action_counts not rolled back");
        assert_eq!(format!("{:?}", initial.priority), format!("{:?}", state.priority), "priority not rolled back");
        assert_eq!(format!("{:?}", initial.a), format!("{:?}", state.a), "player A's zones not rolled back");
        assert_eq!(format!("{:?}", initial.b), format!("{:?}", state.b), "player B's zones not rolled back");

        // card_pool: narrow to which Sleeve + which field.
        for (iid, post_inst) in &state.card_pool {
            let init_inst = initial.card_pool.get(iid)
                .unwrap_or_else(|| panic!("instance {iid} appeared post-rollback (not in initial pool)"));
            assert_eq!(init_inst.tapped, post_inst.tapped, "{iid}.tapped not rolled back");
            assert_eq!(init_inst.damage, post_inst.damage, "{iid}.damage not rolled back");
            assert_eq!(init_inst.face_down, post_inst.face_down, "{iid}.face_down not rolled back");
            assert_eq!(init_inst.summoning_sick, post_inst.summoning_sick, "{iid}.summoning_sick not rolled back");
            assert_eq!(init_inst.attacked_this_turn, post_inst.attacked_this_turn, "{iid}.attacked_this_turn not rolled back");
            assert_eq!(format!("{:?}", init_inst.controller), format!("{:?}", post_inst.controller), "{iid}.controller not rolled back");
            assert_eq!(format!("{:?}", init_inst.attached), format!("{:?}", post_inst.attached), "{iid}.attached not rolled back");
            assert_eq!(format!("{:?}", init_inst.modifiers), format!("{:?}", post_inst.modifiers), "{iid}.modifiers not rolled back");
            assert_eq!(format!("{:?}", init_inst.status_effects), format!("{:?}", post_inst.status_effects), "{iid}.status_effects not rolled back");
        }
        for iid in initial.card_pool.keys() {
            assert!(state.card_pool.contains_key(iid), "instance {iid} removed but rollback didn't restore it");
        }
        // Per-instance whole-Debug compare to surface the divergent one.
        for (iid, post_inst) in &state.card_pool {
            let init_inst = initial.card_pool.get(iid).unwrap();
            let init_dbg = format!("{:?}", init_inst);
            let post_dbg = format!("{:?}", post_inst);
            if init_dbg != post_dbg {
                panic!(
                    "instance {iid} differs after rollback. \n\
                     INITIAL: {init_dbg}\n\
                     POST:    {post_dbg}"
                );
            }
        }
        assert_eq!(format!("{:?}", initial.card_pool), format!("{:?}", state.card_pool), "card_pool differs in some field not covered by per-field checks ({entry_count} journal entries)");

        // JOURNALING CONTRACT backstop: compare the ENTIRE GameState, not
        // a hand-picked subset. The per-field checks above give nice
        // messages; THIS one is unfalsifiable — any field a mutation
        // touched without journaling (the delayed-trigger class of bug)
        // fails here regardless of what anyone remembered to assert.
        assert_eq!(
            format!("{:?}", initial),
            format!("{:?}", state),
            "rollback did not restore the ENTIRE state — a field mutated without being journaled",
        );
    }

    #[test]
    fn run_game_is_deterministic_per_seed_and_decks() {
        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, crate::card::CardType::Creature))
            .unwrap()
            .clone();
        let deck_a: Vec<crate::card::Card> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();

        let state_1 = GameState::new(deck_a.clone(), deck_b.clone());
        let state_2 = GameState::new(deck_a, deck_b);
        let mut rng_1 = StdRng::seed_from_u64(0xEA_C8);
        let mut rng_2 = StdRng::seed_from_u64(0xEA_C8);
        let mut log_1: Vec<String> = Vec::new();
        let mut log_2: Vec<String> = Vec::new();

        let (stats_1, journal_1) = run_game(state_1, &mut rng_1, &mut log_1, &registry, 0xEA_C8);
        let (stats_2, journal_2) = run_game(state_2, &mut rng_2, &mut log_2, &registry, 0xEA_C8);

        assert_eq!(log_1, log_2, "logs diverged across identical runs");
        assert_eq!(
            format!("{stats_1:?}"),
            format!("{stats_2:?}"),
            "GameStats diverged across identical runs"
        );
        assert_eq!(
            format!("{journal_1:?}"),
            format!("{journal_2:?}"),
            "journals diverged across identical runs"
        );
    }

    /// Slice 8.3 acceptance: a deck built from `DeckUnit`s including cardless
    /// sleeves (S.4) plays a full AI game — exercising the Z.8b free draw —
    /// and the full-game replay journal rolls back to the exact initial
    /// state (cardless sleeves included).
    #[test]
    fn full_game_with_cardless_sleeves_runs_and_rolls_back() {
        use crate::game::DeckUnit;
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, crate::card::CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.len() == 1
                    && !c.cost[0].is_x
            })
            .expect("a vanilla creature should exist in cards/")
            .clone();
        let make_units = || -> Vec<DeckUnit> {
            (0..50)
                .map(|i| {
                    if i >= 5 && i % 6 == 5 {
                        DeckUnit::Cardless
                    } else {
                        DeckUnit::Card(template.clone())
                    }
                })
                .collect()
        };

        let mut state = GameState::from_units(make_units(), make_units());
        let initial_cardless =
            state.card_pool.values().filter(|s| s.is_cardless()).count();
        assert!(initial_cardless > 0, "test deck must contain cardless sleeves");

        let initial = state.clone();
        state.replay_journal = Some(crate::game::Journal::new());
        let mut rng = StdRng::seed_from_u64(0x5133_1E55);
        let mut log: Vec<String> = Vec::new();
        let ais = [super::super::AiKind::Fast, super::super::AiKind::Fast];
        let stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, 0x5133_1E55);

        assert!(state.winner.is_some(), "game should have a winner");
        assert!(stats.turns > 0);
        // Z.8b actually fired: at least one cardless sleeve was drawn out of
        // a deck (collected into a hand / moved on) during the game.
        let cardless_off_deck = state
            .card_pool
            .iter()
            .filter(|(iid, s)| {
                s.is_cardless()
                    && !state.a.deck.contains(*iid)
                    && !state.b.deck.contains(*iid)
            })
            .count();
        assert!(
            cardless_off_deck > 0,
            "the free draw should have pulled a cardless sleeve off a deck"
        );

        let journal = state.replay_journal.take().expect("replay_journal open");
        assert!(!journal.is_empty(), "a full game produces journal entries");
        journal.rollback(&mut state);

        assert_eq!(
            format!("{:?}", initial.a),
            format!("{:?}", state.a),
            "A zones not rolled back"
        );
        assert_eq!(
            format!("{:?}", initial.b),
            format!("{:?}", state.b),
            "B zones not rolled back"
        );
        assert_eq!(
            format!("{:?}", initial.card_pool),
            format!("{:?}", state.card_pool),
            "card_pool (incl. cardless sleeves) not rolled back"
        );
        assert_eq!(
            state.card_pool.values().filter(|s| s.is_cardless()).count(),
            initial_cardless,
            "all cardless sleeves restored by rollback"
        );
    }

    #[test]
    fn full_game_with_cardless_sleeves_is_deterministic() {
        use crate::game::DeckUnit;
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, crate::card::CardType::Creature))
            .unwrap()
            .clone();
        let make_units = || -> Vec<DeckUnit> {
            (0..50)
                .map(|i| {
                    if i >= 5 && i % 6 == 5 {
                        DeckUnit::Cardless
                    } else {
                        DeckUnit::Card(template.clone())
                    }
                })
                .collect()
        };

        let s1 = GameState::from_units(make_units(), make_units());
        let s2 = GameState::from_units(make_units(), make_units());
        let mut rng1 = StdRng::seed_from_u64(0xC0FFEE);
        let mut rng2 = StdRng::seed_from_u64(0xC0FFEE);
        let mut log1: Vec<String> = Vec::new();
        let mut log2: Vec<String> = Vec::new();
        let (stats1, j1) = run_game(s1, &mut rng1, &mut log1, &registry, 0xC0FFEE);
        let (stats2, j2) = run_game(s2, &mut rng2, &mut log2, &registry, 0xC0FFEE);

        assert_eq!(log1, log2, "logs diverged with cardless sleeves present");
        assert_eq!(format!("{stats1:?}"), format!("{stats2:?}"));
        assert_eq!(format!("{j1:?}"), format!("{j2:?}"), "journals diverged");
    }

    /// Slice 9.4 — the end-to-end test deck.
    ///
    /// Not a hand-authored fixture: take the shipped blue starter deck,
    /// copy it, and swap a slice of its clears for the azure cardless
    /// stack — Window Cleaners, `clear-azure`, an azure symbol — plus a
    /// few loose cardless sleeves for Window Cleaner's ETB to find. The
    /// real cards, in a real deck, must play a full game to a winner
    /// with the rollback + determinism invariants still holding.
    fn window_cleaner_deck(registry: &CardRegistry) -> Vec<crate::game::DeckUnit> {
        use crate::game::DeckUnit;
        use crate::replay::CARDLESS_SLEEVE_ID;

        fn replace_first_n(ids: &mut [String], from: &str, to: &str, n: usize) {
            let mut done = 0;
            for id in ids.iter_mut() {
                if done >= n {
                    break;
                }
                if id == from {
                    *id = to.to_string();
                    done += 1;
                }
            }
        }

        let lookup = |id: &str| -> crate::card::Card {
            registry
                .cards()
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("card {id} missing from registry"))
                .clone()
        };

        // Copy the shipped blue starter, then convert its 12 `clear-blue`
        // slots into the azure cardless stack: 4 Window Cleaners, 4
        // `clear-azure`, 4 loose cardless sleeves. Swap the blue ix
        // symbols for azure ones so the deck can pay an azure identity.
        let mut ids: Vec<String> = crate::sim::deck_presets::STARTER_DECK_IDS
            .iter()
            .map(|s| s.to_string())
            .collect();
        replace_first_n(&mut ids, "clear-blue", "window-cleaner", 4);
        replace_first_n(&mut ids, "clear-blue", "clear-azure", 4);
        replace_first_n(&mut ids, "clear-blue", CARDLESS_SLEEVE_ID, 4);
        replace_first_n(&mut ids, "blue-ix-symbol", "azure-ix-symbol", 2);

        ids.iter()
            .map(|id| {
                if id == CARDLESS_SLEEVE_ID {
                    DeckUnit::Cardless
                } else {
                    DeckUnit::Card(lookup(id))
                }
            })
            .collect()
    }

    #[test]
    fn full_game_on_a_window_cleaner_deck_runs_and_rolls_back() {
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());

        let mut state = GameState::from_units(
            window_cleaner_deck(&registry),
            window_cleaner_deck(&registry),
        );

        // Build sanity: the copied-and-swapped deck really carries the
        // cardless stack, not just the blue starter.
        let has_window_cleaner = state
            .card_pool
            .values()
            .any(|s| !s.is_cardless() && s.card().id == "window-cleaner");
        let cardless = state.card_pool.values().filter(|s| s.is_cardless()).count();
        assert!(has_window_cleaner, "deck must contain Window Cleaners");
        assert!(cardless > 0, "deck must contain loose cardless sleeves");

        let initial = state.clone();
        state.replay_journal = Some(crate::game::Journal::new());
        let mut rng = StdRng::seed_from_u64(0x9A4_5EED);
        let mut log: Vec<String> = Vec::new();
        let ais = [super::super::AiKind::Fast, super::super::AiKind::Fast];
        let stats =
            run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, 0x9A4_5EED);

        assert!(state.winner.is_some(), "the deck should play to a winner");
        assert!(stats.turns > 0, "the game should take turns");

        let journal = state.replay_journal.take().expect("replay_journal open");
        assert!(!journal.is_empty(), "a full game produces journal entries");
        journal.rollback(&mut state);

        assert_eq!(
            format!("{:?}", initial.a),
            format!("{:?}", state.a),
            "A zones not rolled back after a Window Cleaner game"
        );
        assert_eq!(
            format!("{:?}", initial.b),
            format!("{:?}", state.b),
            "B zones not rolled back after a Window Cleaner game"
        );
        for (iid, post) in &state.card_pool {
            let init = initial
                .card_pool
                .get(iid)
                .unwrap_or_else(|| panic!("instance {iid} appeared post-rollback"));
            assert_eq!(
                format!("{:?}", init.attached),
                format!("{:?}", post.attached),
                "{iid}.attached not rolled back"
            );
            assert_eq!(init.is_cardless(), post.is_cardless(), "{iid} cardless-ness changed");
        }
    }

    #[test]
    fn full_game_on_a_window_cleaner_deck_is_deterministic() {
        let registry =
            std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());

        let s1 = GameState::from_units(
            window_cleaner_deck(&registry),
            window_cleaner_deck(&registry),
        );
        let s2 = GameState::from_units(
            window_cleaner_deck(&registry),
            window_cleaner_deck(&registry),
        );
        let mut rng1 = StdRng::seed_from_u64(0x9A4_C0FFEE);
        let mut rng2 = StdRng::seed_from_u64(0x9A4_C0FFEE);
        let mut log1: Vec<String> = Vec::new();
        let mut log2: Vec<String> = Vec::new();
        let (stats1, j1) = run_game(s1, &mut rng1, &mut log1, &registry, 0x9A4_C0FFEE);
        let (stats2, j2) = run_game(s2, &mut rng2, &mut log2, &registry, 0x9A4_C0FFEE);

        assert_eq!(log1, log2, "logs diverged on the Window Cleaner deck");
        assert_eq!(format!("{stats1:?}"), format!("{stats2:?}"), "stats diverged");
        assert_eq!(format!("{j1:?}"), format!("{j2:?}"), "journals diverged");
    }
}
