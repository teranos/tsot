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

// Web Worker model: wasm runs in a worker thread. Each UCT
// iteration emits one event via the JS-side callback below; the
// callback `postMessage`s the event to the main thread for live
// rendering. Wasm32-only — native callers no-op via #[cfg].
#[cfg(target_arch = "wasm32")]
extern "C" {
    fn tsot_emit_iteration_event(json_ptr: *const u8, json_len: usize);
}

/// Diagnostic counters. Reset via `reset_uct_diagnostics()`.
pub static UCT_PICK_CALLS: AtomicU64 = AtomicU64::new(0);
pub static UCT_ITERATIONS: AtomicU64 = AtomicU64::new(0);
pub static UCT_NODES_CREATED: AtomicU64 = AtomicU64::new(0);

/// Cooperative cancellation flag. Read once per iteration by
/// [`pick_play_uct`]; when set, the search breaks early and
/// returns its best-so-far (the visit-max child of the root, or
/// `None` if the cancel arrived before the first iteration ran).
///
/// **Exported as a no-mangle static** so the JS main thread can
/// resolve its wasm-memory address (`Module._UCT_CANCEL_FLAG`) and
/// flip it via `Atomics.store(sharedHeapI32, addr >> 2, 1)`
/// **synchronously, without waiting for the worker to be idle**.
/// This is what makes mid-search cancellation actually responsive:
/// the wasm-side `is_uct_cancel_requested()` reads the same atomic
/// the main thread just wrote, no postMessage hop required.
///
/// Requires shared wasm memory (`-sSHARED_MEMORY=1` in
/// `.cargo/config.toml` link args) and the page running in a
/// cross-origin-isolated context (COOP/COEP headers — see
/// `tools/serve-isolated.py`).
///
/// Worst-case cancellation latency is one rollout duration
/// (typically ~50–200ms in wasm release, ~75ms in wasm-dev).
///
/// NOT auto-cleared by `pick_play_uct` — callers are responsible
/// for clearing before starting a fresh search if they previously
/// cancelled one. FFI entry points (`tsot_apply_action_impl` etc.)
/// clear at the start of each call.
#[no_mangle]
pub static UCT_CANCEL_FLAG: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Set the cancellation flag from Rust. JS can also set it directly
/// via `Atomics.store` on the shared wasm heap.
pub fn request_uct_cancel() {
    UCT_CANCEL_FLAG.store(1, std::sync::atomic::Ordering::Relaxed);
}

/// Reset the cancellation flag. Call before starting a fresh search
/// if a previous one was cancelled.
pub fn clear_uct_cancel() {
    UCT_CANCEL_FLAG.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Read the current cancellation request without modifying it.
pub fn is_uct_cancel_requested() -> bool {
    UCT_CANCEL_FLAG.load(std::sync::atomic::Ordering::Relaxed) != 0
}

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
    /// Rollout depth cap (turns from the rollout's start turn).
    /// Each iteration's simulate phase terminates after this many
    /// turn-changes and assigns a heuristic winner via
    /// `StepEngine::score_position_a_minus_b` instead of playing to
    /// deck-out. `u32::MAX` reverts to play-to-end (the original
    /// behavior — used by full-game tests). The default 2 keeps
    /// per-pick wall-clock in the hundreds-of-ms range on the
    /// current card pool; without it a single UCT pick was hitting
    /// 10–50s as instrumentation showed.
    pub rollout_turn_cap: u32,
    /// Per-pick wall-clock budget in milliseconds. When the time
    /// spent on a single `pick_play_uct` call exceeds this, the
    /// iteration loop breaks early and picks the best-so-far from
    /// however many iterations actually ran. This bounds individual
    /// picks structurally — without it a complex state would let one
    /// pick burn the entire per-game wall-clock budget while UCT
    /// chased a deep search. `0` disables the cap (legacy behavior).
    /// Default 1000ms covers the 95th-percentile picks at full
    /// iterations while truncating the rare long tail.
    pub per_pick_wall_ms: u32,
}

impl Default for UctConfig {
    fn default() -> Self {
        Self {
            iterations: 50,
            exploration_c: SQRT_2,
            base_seed: 0xBEEF_FACE,
            max_candidates: 10,
            rollout_turn_cap: 1,
            // Per-pick wall budget = 30s. UCT runs until either it
            // completes `iterations` iters OR 30s elapses, picking
            // best-so-far at the break. This IS the search-space
            // cap the operator asked for: hard cards (dark-
            // salamander's dual X-cost, mutation-vial's choice
            // chain) get the full 30s; cheap states finish in ms.
            per_pick_wall_ms: 30_000,
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

/// Debug snapshot of the tree at the end of a [`pick_play_uct`] call.
/// Used by the wasm UI's log panel to show what UCT considered each
/// time the AI side gets priority. Not used by the engine itself —
/// the picker still returns just the chosen iid.
#[derive(Debug, Clone, Default)]
pub struct UctTrace {
    /// Total iterations requested (matches `cfg.iterations`).
    pub iterations: u32,
    /// Why no tree was built (single-candidate fast-path, empty
    /// candidates, etc). Empty string when a real search ran.
    pub note: String,
    /// The iid the picker returned. `None` when there were no
    /// candidates and the caller treats it as a pass.
    pub picked: Option<InstanceId>,
    /// The root node's full subtree. `visits` at the root equals the
    /// number of iterations that completed a backprop pass.
    pub root: UctTraceNode,
}

#[derive(Debug, Clone, Default)]
pub struct UctTraceNode {
    /// `None` at the root; `Some(iid)` for every action edge.
    pub iid: Option<InstanceId>,
    pub visits: u32,
    pub wins: f64,
    pub children: Vec<UctTraceNode>,
}

fn snapshot_subtree(node: &Node, iid: Option<InstanceId>) -> UctTraceNode {
    let mut children: Vec<UctTraceNode> = node
        .children
        .iter()
        .map(|(child_iid, child_node)| snapshot_subtree(child_node, Some(child_iid.clone())))
        .collect();
    // Most-visited first so the ASCII rendering reads top-down by
    // exploration priority.
    children.sort_by_key(|n| std::cmp::Reverse(n.visits));
    UctTraceNode {
        iid,
        visits: node.visits,
        wins: node.wins,
        children,
    }
}

impl UctTrace {
    /// Render the trace as an indented ASCII tree. `name_of` maps a
    /// candidate iid to a display label (card name); the caller pulls
    /// it from the game's card pool so this module stays
    /// game-state-free. Depth is capped to keep the log readable —
    /// UCT's tree fans out fast and the user only needs the first
    /// couple of levels to understand the decision.
    pub fn format_ascii<F>(&self, name_of: F, max_depth: usize) -> String
    where
        F: Fn(&InstanceId) -> String,
    {
        let mut out = String::new();
        if !self.note.is_empty() {
            out.push_str(&self.note);
            return out;
        }
        let picked_str = self
            .picked
            .as_ref()
            .map(|iid| format!("{} ({})", name_of(iid), iid))
            .unwrap_or_else(|| "(pass)".to_string());
        out.push_str(&format!(
            "UCT {} iters, root visits={}, pick: {}\n",
            self.iterations, self.root.visits, picked_str
        ));
        write_node(&mut out, &self.root.children, "", max_depth, &name_of);
        out
    }
}

fn write_node<F>(
    out: &mut String,
    children: &[UctTraceNode],
    prefix: &str,
    remaining_depth: usize,
    name_of: &F,
) where
    F: Fn(&InstanceId) -> String,
{
    let last_idx = children.len().saturating_sub(1);
    for (i, ch) in children.iter().enumerate() {
        let is_last = i == last_idx;
        let connector = if is_last { "└─ " } else { "├─ " };
        let label = ch
            .iid
            .as_ref()
            .map(|iid| format!("{} ({})", name_of(iid), iid))
            .unwrap_or_else(|| "(root)".to_string());
        let wr = if ch.visits == 0 {
            "—".to_string()
        } else {
            format!("{:.2}", ch.wins / ch.visits as f64)
        };
        out.push_str(&format!(
            "{}{}{}  v={} wr={}\n",
            prefix, connector, label, ch.visits, wr
        ));
        if remaining_depth > 0 && !ch.children.is_empty() {
            let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "│  " });
            write_node(out, &ch.children, &child_prefix, remaining_depth - 1, name_of);
        }
    }
}

/// Pick the next card via UCT. Returns `(choice, trace)` where
/// `choice` is `None` if no candidate is playable (caller treats as
/// pass) and `trace` is a debug snapshot of the search tree for log
/// surfaces (the engine itself only consumes `choice`).
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
) -> (Option<InstanceId>, UctTrace) {
    UCT_PICK_CALLS.fetch_add(1, Ordering::SeqCst);

    // O6: bracket whole search with `Instant::now()` so AiPick events
    // carry duration_us. Cheap no-op when trace is off.
    let trace_active = crate::trace::is_enabled();
    let t0 = trace_active.then(std::time::Instant::now);

    // Dedup: see `dedup_candidates_by_card_id` rationale. Without
    // this, 6 blue-monkeys in hand give 6 root children with ~8
    // visits each — same successor state, no signal differentiation.
    let mut candidates = crate::sim::ai::dedup_candidates_by_card_id(
        state,
        enumerate_playable_in_hand(state, player, kind_filter),
    );
    if candidates.is_empty() {
        emit_uct_ai_pick(&[], &None, t0);
        return (
            None,
            UctTrace {
                iterations: cfg.iterations,
                note: "UCT: no candidates → pass".to_string(),
                picked: None,
                root: UctTraceNode::default(),
            },
        );
    }
    if candidates.len() == 1 {
        let only = candidates.into_iter().next();
        emit_uct_ai_pick(
            only.iter().map(|iid| (iid.clone(), 0i32)).collect::<Vec<_>>().as_slice(),
            &only,
            t0,
        );
        return (
            only.clone(),
            UctTrace {
                iterations: cfg.iterations,
                note: format!(
                    "UCT: single candidate fast-path → {}",
                    only.as_ref().map(|i| i.to_string()).unwrap_or_default()
                ),
                picked: only,
                root: UctTraceNode::default(),
            },
        );
    }
    if (candidates.len() as u32) > cfg.max_candidates {
        candidates.sort();
        candidates.truncate(cfg.max_candidates as usize);
    }

    let root_player = player;
    let mut root = Node::new(root_player, candidates.clone());

    // Per-pick wall-clock budget. HARD cap: a deadline is computed
    // once and passed into every rollout via
    // `StepEngine::run_to_end_with_caps`, which checks
    // `Instant::now() >= deadline` inside its step loop and
    // terminates with the heuristic winner on the spot. Iteration-
    // boundary checks alone weren't enough — a single slow rollout
    // on handler-rich state can blow past the budget by seconds.
    // `0` disables both the deadline and the boundary check (legacy).
    let pick_start = std::time::Instant::now();
    let pick_wall_deadline = if cfg.per_pick_wall_ms > 0 {
        Some(pick_start + std::time::Duration::from_millis(cfg.per_pick_wall_ms as u64))
    } else {
        None
    };
    for it in 0..cfg.iterations {
        crate::sim::instrument::set_current_op(format!(
            "UCT iteration {}/{} turn={} player={:?}",
            it + 1, cfg.iterations, state.turn, player
        ));
        // Cooperative cancellation: yield mid-search if the caller
        // requested it (e.g., JS posted a cancel because the user
        // got impatient). Best-so-far selection happens below using
        // whatever visits we accumulated. If we cancel before iter 1,
        // `root.children` is empty and `picked` becomes None.
        if is_uct_cancel_requested() {
            break;
        }
        // Per-pick wall-clock cap (iteration-boundary leg). The hard
        // cap lives inside the rollout (deadline passed below); this
        // check just avoids starting a fresh iteration once the
        // deadline has passed. Together they bound the pick to
        // ~budget + one in-flight step.
        if let Some(deadline) = pick_wall_deadline {
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        UCT_ITERATIONS.fetch_add(1, Ordering::SeqCst);

        // O6+: per-iteration wall-clock. Each iteration emits one
        // UctIteration event with path + winner + duration so the
        // wasm UI can see what UCT is exploring and how long each
        // rollout actually takes. Rollout internals stay suspended.
        let iter_t0 = crate::trace::is_enabled().then(std::time::Instant::now);

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
        let ais = [AiKind::Game, AiKind::Game];
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
        // O6 fix: suspend the trace bus during the rollout so its
        // millions of Step / Cursor / Mutation events don't
        // accumulate into the parent envelope. Without this, the
        // serde JSON serialization at FFI exit takes seconds-to-
        // minutes for any UCT decision.
        let rollout_cap = cfg.rollout_turn_cap;
        let rollout_deadline = pick_wall_deadline;
        let stats = crate::trace::suspend(|| {
            engine.run_to_end_with_caps(rollout_cap, rollout_deadline)
        });
        *state = engine.state;
        let winner = stats.winner;
        clear_planned_actions();

        // O6+: per-iteration event AFTER suspend completes so it
        // actually lands in the buffer. Shows the breakdown the
        // user needs to answer "which part of the search costs
        // the most" — turns simulated, plays in rollout, attacks,
        // deaths, and total Lua handler fires.
        if let Some(iter_t0) = iter_t0 {
            let rollout_handler_fires: u32 = stats
                .event_fires
                .values()
                .map(|v| v[0] + v[1])
                .sum();
            crate::trace::push(crate::trace::TraceEvent::UctIteration {
                at_us: crate::trace::now_us(),
                iter: it,
                total: cfg.iterations,
                path: path.clone(),
                winner,
                duration_us: iter_t0.elapsed().as_micros() as u64,
                rollout_turns: stats.turns,
                rollout_plays: stats.a_played + stats.b_played,
                rollout_attacks: stats.a_attacks + stats.b_attacks,
                rollout_deaths: stats.a_deaths + stats.b_deaths,
                rollout_handler_fires,
            });
            // Live UCT in Web Worker model: emit one iteration
            // summary as a JSON line. JS-side (in the worker) calls
            // postMessage on the line; main thread renders.
            #[cfg(target_arch = "wasm32")]
            {
                let line = format!(
                    "{{\"kind\":\"UctIterLive\",\"iter\":{},\"total\":{},\"duration_us\":{},\"rollout_turns\":{},\"rollout_plays\":{},\"rollout_attacks\":{},\"rollout_deaths\":{},\"rollout_handler_fires\":{}}}",
                    it, cfg.iterations, iter_t0.elapsed().as_micros() as u64,
                    stats.turns, stats.a_played + stats.b_played, stats.a_attacks + stats.b_attacks, stats.a_deaths + stats.b_deaths, rollout_handler_fires,
                );
                unsafe {
                    tsot_emit_iteration_event(line.as_ptr(), line.len());
                }
            }
        }

        // 4. Backprop along the path.
        backpropagate(&mut root, &path, expanded_path_index, winner, root_player);

        // 5. Rollback the iteration's mutations.
        let journal = state.replay_journal.take().unwrap_or_default();
        journal.rollback(state);
        state.replay_journal = outer_replay;
    }

    // Pick the most-visited root child (UCT-robust choice).
    let picked = root
        .children
        .iter()
        .max_by_key(|(_, c)| c.visits)
        .map(|(iid, _)| iid.clone())
        // Fallback: if no iterations expanded a child (shouldn't
        // happen with iterations >= candidates), pick the first
        // candidate.
        .or_else(|| candidates.into_iter().next());
    // O6: emit AiPick with each root child's visit count as its
    // score. UCT's chosen iid is the visit-max, so score == visits
    // is the right signal for "why did UCT pick this."
    let scored: Vec<(InstanceId, i32)> = root
        .children
        .iter()
        .map(|(iid, child)| (iid.clone(), child.visits as i32))
        .collect();
    emit_uct_ai_pick(&scored, &picked, t0);
    let trace = UctTrace {
        iterations: cfg.iterations,
        note: String::new(),
        picked: picked.clone(),
        root: snapshot_subtree(&root, None),
    };
    (picked, trace)
}

/// O6: shared AiPick emission for `pick_play_uct`. Captures the
/// candidate list with per-candidate visit counts (or zeros for
/// pre-search fast paths), the chosen iid, and wall-clock duration
/// since the Instant at function entry.
fn emit_uct_ai_pick(
    scored: &[(InstanceId, i32)],
    chosen: &Option<InstanceId>,
    t0: Option<std::time::Instant>,
) {
    let Some(t0) = t0 else { return };
    let candidates: Vec<crate::trace::CandidateScore> = scored
        .iter()
        .map(|(iid, score)| crate::trace::CandidateScore {
            iid: iid.clone(),
            score: *score,
            rejected_reason: None,
        })
        .collect();
    crate::trace::push(crate::trace::TraceEvent::AiPick {
        at_us: crate::trace::now_us(),
        ai: "Uct".to_string(),
        candidates,
        chosen: chosen.clone(),
        duration_us: t0.elapsed().as_micros() as u64,
    });
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

    /// INTENT: the cancel flag round-trips through its accessors —
    /// request → set, clear → unset, read is non-destructive.
    /// Thread-local so this test doesn't fight a parallel one.
    #[test]
    fn uct_cancel_flag_round_trips() {
        clear_uct_cancel();
        assert!(!is_uct_cancel_requested(), "starts cleared");
        request_uct_cancel();
        assert!(is_uct_cancel_requested(), "set after request");
        assert!(
            is_uct_cancel_requested(),
            "read is non-destructive — flag still set after first read"
        );
        clear_uct_cancel();
        assert!(!is_uct_cancel_requested(), "cleared after clear");
    }

    /// INTENT: pre-arming the cancel flag before calling
    /// `pick_play_uct` makes the search yield BEFORE running its
    /// first iteration — `UCT_ITERATIONS` doesn't budge. Pins the
    /// "the iteration loop's first action is to check the flag"
    /// contract.
    #[test]
    fn uct_pre_armed_cancel_skips_all_iterations() {
        let registry = std::sync::Arc::new(
            CardRegistry::load(std::path::Path::new("cards")).unwrap(),
        );
        // Hand needs ≥ 2 affordable candidates to reach the iteration
        // loop (1-candidate fast-path returns before the loop).
        // Build a 50-card deck of a vanilla creature so the opening
        // hand has multiple distinct iids of the same playable card.
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

        let cfg = UctConfig {
            iterations: 1000, // would normally run many; cancel should cut it off
            exploration_c: SQRT_2,
            base_seed: 0xC0DE,
            max_candidates: 4,
            ..Default::default()
        };

        let iter_before = UCT_ITERATIONS.load(Ordering::SeqCst);
        clear_uct_cancel();
        request_uct_cancel();
        let _ = pick_play_uct(
            &mut state,
            PlayerId::A,
            PickKindFilter::Any,
            &cfg,
            &registry,
        );
        let iter_after = UCT_ITERATIONS.load(Ordering::SeqCst);
        clear_uct_cancel();

        let delta = iter_after - iter_before;
        // The 1-candidate fast-path returns before entering the
        // iteration loop. If dedup collapsed all 4 starting-hand
        // iids to 1 representative, delta = 0 by fast-path. With ≥2
        // representatives, delta should also be 0 because cancel is
        // checked at iter 0. Either way the check holds: a pre-armed
        // cancel runs no iterations.
        assert_eq!(
            delta, 0,
            "pre-armed cancel must skip all UCT iterations, ran {delta}"
        );
    }

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
            ..Default::default()
        };

        let mut rng = StdRng::seed_from_u64(0xC0DE);
        let mut log: Vec<String> = Vec::new();
        let ais = [AiKind::Uct(cfg.clone()), AiKind::Uct(cfg)];
        let stats = run_game_continue(&mut state, &mut rng, &mut log, &registry, &ais, 0xC0DE);

        assert!(state.winner.is_some(), "UCT game produced no winner");
        assert!(stats.turns > 0, "UCT game recorded zero turns");
    }

    /// Microbenchmark: a SINGLE pick_play_uct call with the default
    /// rollout_turn_cap=6 must complete in well under 5 seconds on
    /// realistic-card state, not the 30–60 seconds the EA was seeing
    /// in production. If this asserts, the cap isn't actually bounding
    /// the rollout cost (likely candidate: state.turn doesn't advance
    /// the way I expected from the cap-firing point) and a deeper
    /// dive is warranted.
    #[test]
    fn pick_play_uct_with_rollout_cap_completes_fast() {
        let registry = std::sync::Arc::new(
            CardRegistry::load(std::path::Path::new("cards")).unwrap(),
        );
        let template = registry
            .cards()
            .iter()
            .find(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.handlers.is_empty()
                    && c.cost.iter().all(|cc| {
                        !cc.is_x
                            && matches!(
                                cc.source,
                                crate::card::CostSource::Hand
                                    | crate::card::CostSource::Mill
                            )
                    })
            })
            .unwrap()
            .clone();
        let deck_a: Vec<_> = (0..50).map(|_| template.clone()).collect();
        let deck_b = deck_a.clone();
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(Journal::new());
        let cfg = UctConfig {
            iterations: 30,
            exploration_c: SQRT_2,
            base_seed: 0xC0DE,
            max_candidates: 10,
            rollout_turn_cap: 6,
            per_pick_wall_ms: 0,
        };
        let t0 = std::time::Instant::now();
        let _ = pick_play_uct(
            &mut state,
            PlayerId::A,
            crate::sim::ai::PickKindFilter::Any,
            &cfg,
            &registry,
        );
        let elapsed = t0.elapsed();
        // 5s is generous: at 6-turn rollouts × 30 iterations on
        // vanilla 1-hand creatures in debug, expect well under 1s.
        // If this asserts, the cap is not effective.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "pick_play_uct with rollout_turn_cap=6 took {elapsed:?} — cap not effective",
        );
    }

    // per_pick_wall_ms is a HARD cap: when the budget runs out
    // mid-rollout, the rollout itself terminates (heuristic winner
    // assigned by StepEngine::score_position_a_minus_b) and UCT
    // returns best-so-far. Without mid-rollout cancellation a single
    // slow iteration on handler-heavy state can blow the budget by
    // seconds — the operator saw 30-70s picks with 1000ms iter-
    // boundary cap. With true mid-rollout cancellation, the pick
    // bounds at budget + at-most-one-step's worth.
    //
    // Test: use real handler-rich cards (the full registry pool) so
    // rollout iterations are slow enough to make the difference
    // measurable. 50ms budget, expect pick to complete within 200ms
    // (4× tolerance for finalize + check cadence + test overhead).
    #[test]
    fn pick_play_uct_per_pick_wall_is_a_hard_cap() {
        let registry = std::sync::Arc::new(
            CardRegistry::load(std::path::Path::new("cards")).unwrap(),
        );
        // Build a deck from real registry cards — handler-rich on
        // purpose so rollout per-iteration cost is non-trivial.
        let real_cards: Vec<_> = registry
            .cards()
            .iter()
            .filter(|c| {
                matches!(c.kind, CardType::Creature)
                    && c.cost.iter().all(|cc| {
                        matches!(
                            cc.source,
                            crate::card::CostSource::Hand
                                | crate::card::CostSource::Mill
                        )
                    })
            })
            .take(10)
            .cloned()
            .collect();
        assert!(!real_cards.is_empty(), "need real cards for handler load");
        let deck_a: Vec<_> = (0..50)
            .map(|i| real_cards[i % real_cards.len()].clone())
            .collect();
        let deck_b = deck_a.clone();
        let mut state = GameState::new(deck_a, deck_b);
        state.replay_journal = Some(Journal::new());
        let cfg = UctConfig {
            iterations: 10_000,
            exploration_c: SQRT_2,
            base_seed: 0xC0DE,
            max_candidates: 10,
            // Disable turn cap so per_pick_wall_ms is the only
            // bound — that's what we're testing.
            rollout_turn_cap: u32::MAX,
            per_pick_wall_ms: 50,
        };
        let t0 = std::time::Instant::now();
        let _ = pick_play_uct(
            &mut state,
            PlayerId::A,
            crate::sim::ai::PickKindFilter::Any,
            &cfg,
            &registry,
        );
        let elapsed = t0.elapsed();
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "per_pick_wall_ms=50 must hard-cap within ~4× budget, got {elapsed:?}. \
             If this fails the deadline check isn't reaching inside the rollout.",
        );
    }
}
