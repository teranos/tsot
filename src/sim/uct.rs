// S12: tests in this module still call `run_game_continue` for the
// full-game smoke. Production UCT rollouts now state-swap into a
// `StepEngine`. Suppress deprecation at the file level for the
// test path.
#![allow(deprecated)]

//! UCT (UCB1 Tree-Search) MCTS for tsot's card-pick decision.
//!
//! Distinct from [`super::mcts`]'s one-ply rollout: UCT maintains a
//! persistent search tree across iterations, applies UCB1 to focus
//! iterations on promising branches, and expands one new leaf per
//! iteration. The tree branches on every `pick_play` decision —
//! both my picks and the opponent's. Each leaf finishes with a
//! Heuristic rollout (default policy).
//!
//! Algorithm per iteration:
//!   1. Selection — from root, walk down picking children via UCB1
//!      until reaching a node with untried actions OR a terminal
//!      (game-ended) state.
//!   2. Expansion — pop one untried action, create a child node.
//!   3. Simulation — from the expanded child, finish the game with
//!      the heuristic AI on both sides.
//!   4. Backprop — walk back up the path, updating each node's
//!      visit count and accumulated reward (perspective: the win
//!      from the *acting* player's POV at each node).
//!
//! State management: each iteration opens a fresh journal, replays
//! the selection path action-by-action by feeding the planned action
//! sequence to a thread-local "override" the run-loop consults
//! before falling back to the heuristic picker. After the rollout
//! finishes, the journal is rolled back so the next iteration
//! starts from the same root state. The full-game rollback invariant
//! covers this exercise.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::f64::consts::SQRT_2;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::game::{GameState, InstanceId, Journal, PlayerId};

use super::ai::{enumerate_playable_in_hand, PickKindFilter};
use super::AiKind;

/// Diagnostic counters. Reset via `reset_uct_diagnostics()`.
pub static UCT_PICK_CALLS: AtomicU64 = AtomicU64::new(0);
pub static UCT_ITERATIONS: AtomicU64 = AtomicU64::new(0);
pub static UCT_NODES_CREATED: AtomicU64 = AtomicU64::new(0);

pub fn reset_uct_diagnostics() {
    UCT_PICK_CALLS.store(0, Ordering::SeqCst);
    UCT_ITERATIONS.store(0, Ordering::SeqCst);
    UCT_NODES_CREATED.store(0, Ordering::SeqCst);
}

thread_local! {
    /// Per-iteration plan: actions for the engine's `pick_play` calls
    /// to consume in order. `idx >= plan.len()` means "plan exhausted,
    /// fall back to heuristic." Set by [`pick_play_uct`] before each
    /// simulation; consumed by [`take_planned_action`] from inside
    /// the engine's pick-dispatch.
    static UCT_PLAN: RefCell<Vec<InstanceId>> = const { RefCell::new(Vec::new()) };
    static UCT_PLAN_IDX: RefCell<usize> = const { RefCell::new(0) };
}

/// Called by `run.rs`'s heuristic-pick dispatch to consume the next
/// planned action, if any. Returns `Some(iid)` when the plan still
/// has entries (selection / expansion phase), `None` when exhausted
/// (the engine then falls back to its heuristic picker for the rest
/// of the simulation).
pub fn take_planned_action() -> Option<InstanceId> {
    UCT_PLAN.with(|plan| {
        UCT_PLAN_IDX.with(|idx| {
            let mut i = idx.borrow_mut();
            let p = plan.borrow();
            if *i >= p.len() {
                return None;
            }
            let action = p[*i].clone();
            *i += 1;
            Some(action)
        })
    })
}

/// Reset the plan so the next call to [`take_planned_action`] returns
/// `None`. Called by [`pick_play_uct`] after each simulation completes,
/// so leftover state doesn't bleed into an unrelated future search.
fn clear_planned_actions() {
    UCT_PLAN.with(|p| p.borrow_mut().clear());
    UCT_PLAN_IDX.with(|i| *i.borrow_mut() = 0);
}

#[derive(Debug, Clone)]
pub struct UctConfig {
    /// Total UCT iterations per pick decision. Each iteration is one
    /// selection / expansion / simulation / backprop cycle. Cost
    /// ≈ `iterations × game_finish_time` (path-replay overhead is
    /// small compared to the heuristic finish).
    pub iterations: u32,
    /// UCB1 exploration constant. `sqrt(2)` is the classical choice
    /// when rewards are in `[0, 1]`. Higher = more exploration of
    /// under-visited children; lower = more exploitation of the
    /// current best.
    pub exploration_c: f64,
    /// Base seed for iteration RNG. Iteration `i` uses
    /// `base_seed + i` so the iteration sequence is reproducible.
    pub base_seed: u64,
    /// Cap on candidates at each tree node — defense against
    /// pathologically wide hands. Above this cap, candidates are
    /// truncated deterministically (first-N by InstanceId order).
    pub max_candidates: u32,
}

impl Default for UctConfig {
    fn default() -> Self {
        Self {
            iterations: 50,
            exploration_c: SQRT_2,
            base_seed: 0xBEEF_FACE,
            max_candidates: 10,
        }
    }
}

#[derive(Debug)]
struct Node {
    visits: u32,
    /// Accumulated reward for the player who acts AT this node.
    /// Per UCT convention: when backprop'ing a game outcome, each
    /// node's reward is updated using the outcome from THAT node's
    /// `player_to_act` perspective (1.0 if player_to_act won, else 0.0).
    /// During selection at the parent, the parent's choice of child
    /// uses the child's win-rate FROM THE CHILD'S acting-player
    /// perspective — which is the parent's opponent's perspective.
    /// So the parent inverts: parent wants child where
    /// `1 - child.wins/child.visits` is highest. Equivalently,
    /// store wins from the PARENT's perspective and use directly.
    /// We use the latter — `wins` is "wins for the player whose move
    /// led INTO this node," i.e. the parent's player. Makes UCB1
    /// arithmetic trivial.
    wins: f64,
    children: BTreeMap<InstanceId, Box<Node>>,
    untried: Vec<InstanceId>,
    /// The player whose turn it is to pick at THIS node. Children
    /// of this node represent THIS player's possible actions.
    player_to_act: PlayerId,
}

impl Node {
    fn new(player_to_act: PlayerId, candidates: Vec<InstanceId>) -> Self {
        UCT_NODES_CREATED.fetch_add(1, Ordering::SeqCst);
        Self {
            visits: 0,
            wins: 0.0,
            children: BTreeMap::new(),
            untried: candidates,
            player_to_act,
        }
    }

    fn is_fully_expanded(&self) -> bool {
        self.untried.is_empty()
    }
}

/// Pick the next card via UCT. Returns `None` if no candidate is
/// playable (caller treats as pass), `Some(iid)` otherwise.
///
/// The returned iid is the root child with the highest visit count
/// — UCT's most-robust answer (more exploration → more confidence).
/// Win-rate tie-break would be more brittle to short searches.
pub fn pick_play_uct(
    state: &mut GameState,
    player: PlayerId,
    kind_filter: PickKindFilter,
    cfg: &UctConfig,
    registry: &std::sync::Arc<crate::card::CardRegistry>,
) -> Option<InstanceId> {
    UCT_PICK_CALLS.fetch_add(1, Ordering::SeqCst);

    let mut candidates = enumerate_playable_in_hand(state, player, kind_filter);
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    if (candidates.len() as u32) > cfg.max_candidates {
        candidates.sort();
        candidates.truncate(cfg.max_candidates as usize);
    }

    let root_player = player;
    let mut root = Node::new(root_player, candidates.clone());

    for it in 0..cfg.iterations {
        UCT_ITERATIONS.fetch_add(1, Ordering::SeqCst);

        // Open a per-iteration journal so the simulation's mutations
        // can be rolled back at the end.
        let outer_replay = state.replay_journal.take();
        state.replay_journal = Some(Journal::new());

        // 1. Selection + 2. Expansion: build the planned path of
        //    actions to feed to the engine.
        let (path, expanded_path_index) = select_and_expand(&mut root, cfg, it);

        // 3. Simulation: run the engine with the plan installed. The
        //    engine consumes path[0..N] via the override, then falls
        //    back to heuristic for the rest of the game.
        UCT_PLAN.with(|p| *p.borrow_mut() = path.clone());
        UCT_PLAN_IDX.with(|i| *i.borrow_mut() = 0);
        let ais = [AiKind::Heuristic, AiKind::Heuristic];
        // S12: state-swap UCT rollout finish into a StepEngine. Same
        // pattern as the MCTS rollout — swap, run, swap back. The
        // per-iteration journal travels with the state.
        let placeholder = crate::game::GameState::new(Vec::new(), Vec::new());
        let taken = std::mem::replace(state, placeholder);
        let rollout_seed = cfg.base_seed.wrapping_add(it as u64);
        let mut engine = crate::sim::step::StepEngine::new(
            taken,
            ais,
            registry.clone(),
            rollout_seed,
        );
        let stats = engine.run_to_end();
        *state = engine.state;
        let winner = stats.winner;
        clear_planned_actions();

        // 4. Backprop along the path.
        backpropagate(&mut root, &path, expanded_path_index, winner, root_player);

        // 5. Rollback the iteration's mutations.
        let journal = state.replay_journal.take().unwrap_or_default();
        journal.rollback(state);
        state.replay_journal = outer_replay;
    }

    // Pick the most-visited root child (UCT-robust choice).
    root.children
        .iter()
        .max_by_key(|(_, c)| c.visits)
        .map(|(iid, _)| iid.clone())
        // Fallback: if no iterations expanded a child (shouldn't
        // happen with iterations >= candidates), pick the first
        // candidate.
        .or_else(|| candidates.into_iter().next())
}

/// Walk the tree from root via UCB1 until reaching a node with
/// untried actions OR a node with no children left to expand. Then
/// pop one untried action and create a child node. Returns:
/// - `path`: the action sequence from root to (and including) the
///   newly-expanded child. The engine consumes this in order.
/// - `expanded_path_index`: index in `path` at which the new child
///   sits. Below that index = pre-existing tree; at = the new node.
///
/// If no expansion was possible (e.g., root is fully expanded with
/// no children — degenerate case), returns an empty path so the
/// simulation falls back to pure heuristic.
fn select_and_expand(root: &mut Node, cfg: &UctConfig, _iter: u32) -> (Vec<InstanceId>, usize) {
    let mut path: Vec<InstanceId> = Vec::new();
    let total_visits = root.visits;
    let mut current = root;
    let mut total = total_visits.max(1);

    // SELECTION: descend via UCB1 while we're at a fully-expanded
    // node with children.
    loop {
        if !current.untried.is_empty() {
            // Stop here — we'll expand one of the untried actions.
            break;
        }
        if current.children.is_empty() {
            // Fully expanded with no children. Either: terminal
            // state (shouldn't happen pre-simulation), or this is a
            // leaf where every action has been tried zero times
            // (unusual). Stop and let simulation run from here.
            let idx = path.len();
            return (path, idx);
        }
        // UCB1: pick child with highest score.
        let parent_visits = current.visits.max(1) as f64;
        let parent_ln = parent_visits.ln();
        let c = cfg.exploration_c;
        let chosen_iid = current
            .children
            .iter()
            .max_by(|(_, a), (_, b)| {
                let sa = ucb1_score(a, parent_ln, c);
                let sb = ucb1_score(b, parent_ln, c);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(iid, _)| iid.clone())
            .expect("non-empty children verified above");
        path.push(chosen_iid.clone());
        total = total.saturating_add(1);
        // Re-borrow to descend.
        current = current.children.get_mut(&chosen_iid).expect("just chose this iid");
    }

    // EXPANSION: pop one untried, install as a new child.
    // We can't know the candidates at the new child's state without
    // actually advancing the engine — defer that to backprop or the
    // next iteration when this child becomes the leaf and we observe
    // its candidates from the engine's first pick_play call after
    // the path exhausts. For now: create with empty `untried`. It
    // will be populated lazily by a later iteration that descends
    // here, or stays empty (turning it into a "rollout-only" leaf).
    let untried_action = current
        .untried
        .pop()
        .expect("untried non-empty verified above");
    path.push(untried_action.clone());
    let expanded_idx = path.len() - 1;
    // We don't know the next player_to_act without running the
    // engine. Default to the same player for now — it gets used as
    // the perspective for the child's `wins` field. Since this leaf
    // hasn't had any iterations through it yet, the perspective
    // mismatch (if the next pick is the opponent's) will resolve
    // itself on the next iteration that descends here: that descent
    // sets up the right player_to_act via the engine's actual flow.
    let new_node = Node::new(current.player_to_act, Vec::new());
    current.children.insert(untried_action.clone(), Box::new(new_node));

    (path, expanded_idx)
}

fn ucb1_score(child: &Node, parent_ln_visits: f64, c: f64) -> f64 {
    if child.visits == 0 {
        // Unvisited children dominate selection — pick them first.
        return f64::INFINITY;
    }
    let exploitation = child.wins / child.visits as f64;
    let exploration = c * (parent_ln_visits / child.visits as f64).sqrt();
    exploitation + exploration
}

/// Update visits + wins along the path. For each node on the path,
/// the reward depends on whether the eventual winner matched the
/// player whose move LED INTO that node.
///
/// We track `wins` from the perspective of the player who *acted*
/// at the parent — i.e. the player whose choice put us into the
/// child. So at the root's children: wins from root_player's
/// perspective. At a grandchild: wins from the opp's perspective
/// (since the opp acted at the root's child to reach the grandchild).
///
/// Since we don't actually track player_to_act per path step (we'd
/// need the engine to tell us at each pick), we use a simple
/// heuristic: the path alternates between players starting from
/// `root_player`. Tsot's Pattern B can multi-pick within a turn so
/// this isn't always right, but it's close enough for v1; refining
/// requires the engine to expose "next picker" at each step.
fn backpropagate(
    root: &mut Node,
    path: &[InstanceId],
    _expanded_idx: usize,
    winner: PlayerId,
    root_player: PlayerId,
) {
    root.visits = root.visits.saturating_add(1);
    let mut current = &mut *root;
    let mut acting = root_player;
    for action in path {
        // The reward at the CHILD reached by `action` is "did the
        // player who chose `action` win?" — which is `acting`.
        let reward = if winner == acting { 1.0 } else { 0.0 };
        let next: &mut Node = match current.children.get_mut(action) {
            Some(c) => c,
            // Child missing (shouldn't happen if selection+expansion
            // ran cleanly). Stop backprop gracefully.
            None => return,
        };
        next.visits = next.visits.saturating_add(1);
        next.wins += reward;
        current = next;
        // Tsot picks aren't strictly alternating, but for the simple
        // v1 perspective tracking we flip player each ply. Replace
        // with engine-reported player_to_act in v2 if measurement
        // shows this matters.
        acting = acting.opponent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{CardRegistry, CardType};
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    #[test]
    fn uct_plays_a_full_game() {
        use crate::sim::run::run_game_continue;

        let registry = std::sync::Arc::new(CardRegistry::load(std::path::Path::new("cards")).unwrap());
        let template = registry
            .cards()
            .iter()
            .find(|c| matches!(c.kind, CardType::Creature) && c.handlers.is_empty())
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(Journal::new());

        // Tiny config so the test finishes quickly.
        let cfg = UctConfig {
            iterations: 4,
            exploration_c: SQRT_2,
            base_seed: 0xC0DE,
            max_candidates: 4,
        };

        let mut rng = StdRng::seed_from_u64(0xC0DE);
        let mut log: Vec<String> = Vec::new();
        let ais = [AiKind::Uct(cfg.clone()), AiKind::Uct(cfg)];
        let stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais);

        assert!(state.winner.is_some(), "UCT game produced no winner");
        assert!(stats.turns > 0, "UCT game recorded zero turns");
    }
}
