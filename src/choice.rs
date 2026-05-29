//! Choice oracles — the "thing that answers choice questions" from Lua handlers.
//!
//! When a card handler asks `game.choose_card(pool, opts)`, the engine
//! delegates the decision to an oracle. Different oracle implementations
//! plug in for different contexts (sim, tests, future UI).

use crate::game::InstanceId;
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

/// Oracle trait — implementors answer choice questions on behalf of a player.
pub trait ChoiceOracle {
    /// Pick one card from a pool, or None if optional and skipped.
    fn choose_card(&mut self, req: ChooseCardRequest) -> Option<InstanceId>;

    /// Yes/no decision. Used by `game.confirm`.
    fn confirm(&mut self, prompt: &str) -> bool;
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
}
