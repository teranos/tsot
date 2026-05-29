//! Choice oracles — the "thing that answers choice questions" from Lua handlers.
//!
//! When a card handler asks `game.choose_card(pool, opts)`, the engine
//! delegates the decision to an oracle. Different oracle implementations
//! plug in for different contexts (sim, tests, future UI).

use crate::game::{InstanceId, PlayerId};
use rand::Rng;
use std::collections::VecDeque;

/// A choose-card prompt with the pool and options.
#[derive(Debug, Clone)]
pub struct ChooseCardRequest {
    pub pool: Vec<InstanceId>,
    /// If true, the oracle may return None (skip the choice).
    pub optional: bool,
    /// Free-form prompt for UI; ignored by random/scripted oracles.
    pub prompt: String,
}

/// A choose-player prompt. For 1v1 this is usually trivial (the active
/// player vs. the opponent), but the surface exists for cards that
/// explicitly say "target player" and for future multi-player.
#[derive(Debug, Clone)]
pub struct ChoosePlayerRequest {
    pub exclude: Vec<PlayerId>,
    pub optional: bool,
    pub prompt: String,
}

/// A choose-int prompt. Used for variable-X costs and X-value handler
/// choices (e.g., "deal X damage; choose X").
#[derive(Debug, Clone)]
pub struct ChooseIntRequest {
    pub min: i32,
    pub max: i32,
    pub prompt: String,
}

/// Oracle trait — implementors answer choice questions on behalf of a player.
pub trait ChoiceOracle {
    /// Pick one card from a pool, or None if optional and skipped.
    fn choose_card(&mut self, req: ChooseCardRequest) -> Option<InstanceId>;

    /// Yes/no decision. Used by `game.confirm`.
    fn confirm(&mut self, prompt: &str) -> bool;

    /// Pick a player from `{A, B} - exclude`. Returns None if the candidate
    /// pool is empty, or if optional and the oracle declines.
    fn choose_player(&mut self, req: ChoosePlayerRequest) -> Option<PlayerId>;

    /// Pick an integer in `[min, max]`. Mandatory — no opt-out.
    fn choose_int(&mut self, req: ChooseIntRequest) -> i32;
}

/// Random oracle — sim default. Picks uniformly random from the pool,
/// biases toward "activity" on optional choices (70% pick, 30% skip).
/// Filter logic is the card-author's responsibility — the oracle just
/// picks from whatever pool is passed.
pub struct RandomOracle<R: Rng> {
    rng: R,
}

impl<R: Rng> RandomOracle<R> {
    pub fn new(rng: R) -> Self {
        Self { rng }
    }
}

impl<R: Rng> ChoiceOracle for RandomOracle<R> {
    fn choose_card(&mut self, req: ChooseCardRequest) -> Option<InstanceId> {
        if req.pool.is_empty() {
            return None;
        }
        if req.optional && self.rng.gen_bool(0.3) {
            return None;
        }
        let idx = self.rng.gen_range(0..req.pool.len());
        Some(req.pool[idx].clone())
    }

    fn confirm(&mut self, _prompt: &str) -> bool {
        self.rng.gen_bool(0.7)
    }

    fn choose_player(&mut self, req: ChoosePlayerRequest) -> Option<PlayerId> {
        let candidates: Vec<PlayerId> = [PlayerId::A, PlayerId::B]
            .into_iter()
            .filter(|p| !req.exclude.contains(p))
            .collect();
        if candidates.is_empty() {
            return None;
        }
        if req.optional && self.rng.gen_bool(0.3) {
            return None;
        }
        let idx = self.rng.gen_range(0..candidates.len());
        Some(candidates[idx])
    }

    fn choose_int(&mut self, req: ChooseIntRequest) -> i32 {
        let lo = req.min.min(req.max);
        let hi = req.min.max(req.max);
        self.rng.gen_range(lo..=hi)
    }
}

/// Noop oracle — convenience for tests that exercise a handler but don't
/// actually invoke any choice prompts. Returns None / false.
pub struct NoopOracle;

impl ChoiceOracle for NoopOracle {
    fn choose_card(&mut self, _req: ChooseCardRequest) -> Option<InstanceId> {
        None
    }
    fn confirm(&mut self, _prompt: &str) -> bool {
        false
    }
    fn choose_player(&mut self, _req: ChoosePlayerRequest) -> Option<PlayerId> {
        None
    }
    fn choose_int(&mut self, req: ChooseIntRequest) -> i32 {
        req.min
    }
}

/// Scripted oracle — for tests. Answers are pre-loaded; each call consumes one.
/// Panics if asked the wrong kind of question or runs out of answers.
pub struct ScriptedOracle {
    answers: VecDeque<ScriptedAnswer>,
}

#[derive(Debug, Clone)]
pub enum ScriptedAnswer {
    Card(Option<InstanceId>),
    Confirm(bool),
    Player(Option<PlayerId>),
    Int(i32),
}

impl ScriptedOracle {
    pub fn new(answers: Vec<ScriptedAnswer>) -> Self {
        Self {
            answers: answers.into(),
        }
    }
}

impl ChoiceOracle for ScriptedOracle {
    fn choose_card(&mut self, _req: ChooseCardRequest) -> Option<InstanceId> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Card(c)) => c,
            Some(other) => panic!(
                "ScriptedOracle: expected Card answer, got {other:?}"
            ),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn confirm(&mut self, _prompt: &str) -> bool {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Confirm(b)) => b,
            Some(other) => panic!(
                "ScriptedOracle: expected Confirm answer, got {other:?}"
            ),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_player(&mut self, _req: ChoosePlayerRequest) -> Option<PlayerId> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Player(p)) => p,
            Some(other) => panic!(
                "ScriptedOracle: expected Player answer, got {other:?}"
            ),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_int(&mut self, _req: ChooseIntRequest) -> i32 {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Int(n)) => n,
            Some(other) => panic!(
                "ScriptedOracle: expected Int answer, got {other:?}"
            ),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }
}
