//! Choice oracles — the "thing that answers choice questions" from Lua handlers.
//!
//! When a card handler asks `game.choose_card(pool, opts)`, the engine
//! delegates the decision to an oracle. Different oracle implementations
//! plug in for different contexts (sim, tests, future UI).

use crate::game::{GameState, InstanceId, PlayChoices, PlayerId, StackItem};
use rand::Rng;
use std::collections::VecDeque;

/// Decision returned from `ChoiceOracle::respond_or_pass`. Either pass
/// priority, or play a card from hand as a response (currently zero-cost
/// instants only — the policy is intentionally narrow).
///
/// TODO(stack-phase-2-driver): the right architecture is Option B from the
/// design discussion — `play_card` just announces, the caller (sim main
/// loop or UI) drives the priority loop and feeds decisions in. That
/// matches how the UI will work (human is the outer driver) and keeps
/// engine policy-free. Option A (this trait method) is Phase 1 expedience:
/// it gets counterspell firing in the sim without restructuring `play_card`
/// and the existing call sites. Revisit before the UX work starts.
#[derive(Debug, Clone)]
pub enum ResponseAction {
    Pass,
    Respond {
        card: InstanceId,
        choices: PlayChoices,
    },
}

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

    /// Inside an open response window, `player` has priority — should they
    /// cast something from hand as a response, or pass? Default impl: Pass.
    /// Every oracle keeps its existing behavior (NoopOracle, ScriptedOracle
    /// in old tests) unless it explicitly overrides.
    ///
    /// See `ResponseAction` for the architectural caveat (Option B is the
    /// long-term path).
    fn respond_or_pass(&mut self, _state: &GameState, _player: PlayerId) -> ResponseAction {
        ResponseAction::Pass
    }
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

    /// Phase 2 response policy: respond when an opposing threat is present —
    /// either an opposing cast on top of the chain (R.1.a) or an opposing
    /// declared attacker (R.1.b empty-chain window). Picks any
    /// `playable_responses` candidate; pays HAND cost by taking the first
    /// non-self cards in hand (deterministic — smart payment ordering is a
    /// future refinement). Probability is threat-aware: high if fast death
    /// looms, lower otherwise.
    fn respond_or_pass(&mut self, state: &GameState, player: PlayerId) -> ResponseAction {
        let Some(p) = &state.priority else {
            return ResponseAction::Pass;
        };

        // Threat present? Either chain-top is an opposing cast, or there
        // are opposing declared attackers (R.1.b context with empty chain).
        let chain_threat = p.chain.last().is_some_and(|top| {
            let StackItem::PlayedCard { controller, .. } = top;
            *controller != player
        });
        let combat_threat = matches!(
            &state.combat,
            Some(crate::game::CombatState::AwaitingBlockers { attacks })
                if attacks.iter().any(|a| {
                    state.card_pool.get(&a.attacker)
                        .map(|i| i.controller != player)
                        .unwrap_or(false)
                })
        );
        if !chain_threat && !combat_threat {
            return ResponseAction::Pass;
        }

        let candidates = state.playable_responses(player);
        if candidates.is_empty() {
            return ResponseAction::Pass;
        }
        let prob = if would_die_soon(state, player) {
            0.95
        } else {
            0.25
        };
        if !self.rng.gen_bool(prob) {
            return ResponseAction::Pass;
        }
        let pick = candidates[self.rng.gen_range(0..candidates.len())].clone();

        // Hand-cost payment: deterministic first-N non-self hand cards.
        // Smart payment selection is a follow-up; here we just need a
        // legal payment so play_card validates.
        let Some(inst) = state.card_pool.get(&pick) else {
            return ResponseAction::Pass;
        };
        let mut hand_need: usize = 0;
        for c in &inst.card.cost {
            if let crate::card::CostSource::Hand = c.source {
                hand_need += c.amount.max(0) as usize;
            }
        }
        let hand_payment_ids: Vec<InstanceId> = if hand_need > 0 {
            state
                .player(player)
                .hand
                .iter()
                .filter(|iid| **iid != pick)
                .take(hand_need)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        ResponseAction::Respond {
            card: pick,
            choices: PlayChoices {
                hand_payment_ids,
                x_value: None,
            },
        }
    }
}

/// "Will this player be decked within ~2 turns at the opponent's current
/// pace?" Sums opponent's on-board creature power (tapped or not — tapped
/// creatures will untap and attack again, or are CURRENTLY attacking in
/// the case of declared attackers in R.1.b) + any opposing creature on
/// the chain. Cheap heuristic used by `RandomOracle` to decide whether a
/// response is worth burning.
fn would_die_soon(state: &GameState, victim: PlayerId) -> bool {
    let opponent = victim.opponent();
    let board_power: i32 = state
        .player(opponent)
        .board
        .iter()
        .map(|iid| state.effective_stats(iid).0)
        .sum();
    let chain_power: i32 = state
        .priority
        .as_ref()
        .map(|p| {
            p.chain
                .iter()
                .map(|item| {
                    let StackItem::PlayedCard {
                        card, controller, ..
                    } = item;
                    if *controller != opponent {
                        return 0;
                    }
                    let Some(inst) = state.card_pool.get(card) else {
                        return 0;
                    };
                    if inst.card.kind == crate::card::CardType::Creature {
                        state.effective_stats(card).0
                    } else {
                        0
                    }
                })
                .sum()
        })
        .unwrap_or(0);
    let incoming = board_power + chain_power;
    let deck = state.player(victim).deck.len() as i32;
    incoming > 0 && deck < incoming * 2
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

impl ScriptedOracle {
    /// Build a new sequence with the first `Player` answer swapped to the
    /// alternative candidate from `req`. Returns `None` if no `Player` answer
    /// is present, or no alternative exists. Used by the sim to retry a
    /// suicidal play with a different "target player" pick.
    pub fn flip_first_player(answers: &[ScriptedAnswer]) -> Option<Vec<ScriptedAnswer>> {
        let pos = answers
            .iter()
            .position(|a| matches!(a, ScriptedAnswer::Player(_)))?;
        let original = match &answers[pos] {
            ScriptedAnswer::Player(p) => *p,
            _ => unreachable!(),
        };
        // 1v1: flip A↔B. If the original was None, force A as a fallback —
        // None means the oracle declined an optional pick, and retrying with
        // an actual pick is the whole point.
        let alt = match original {
            Some(PlayerId::A) => Some(PlayerId::B),
            Some(PlayerId::B) => Some(PlayerId::A),
            None => Some(PlayerId::A),
        };
        let mut out: Vec<ScriptedAnswer> = answers.to_vec();
        out[pos] = ScriptedAnswer::Player(alt);
        Some(out)
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

/// Recording wrapper around any inner oracle. Captures the exact answer
/// sequence so the sim can replay a play with a modified answer set when the
/// first attempt suicides. The recording lives outside `GameState`, so journal
/// rollback doesn't clear it.
pub struct RecordingOracle<O: ChoiceOracle> {
    inner: O,
    recording: Vec<ScriptedAnswer>,
}

impl<O: ChoiceOracle> RecordingOracle<O> {
    pub fn new(inner: O) -> Self {
        Self {
            inner,
            recording: Vec::new(),
        }
    }

    pub fn recording(&self) -> &[ScriptedAnswer] {
        &self.recording
    }

    pub fn clear(&mut self) {
        self.recording.clear();
    }

    pub fn inner_mut(&mut self) -> &mut O {
        &mut self.inner
    }
}

impl<O: ChoiceOracle> ChoiceOracle for RecordingOracle<O> {
    fn choose_card(&mut self, req: ChooseCardRequest) -> Option<InstanceId> {
        let ans = self.inner.choose_card(req);
        self.recording.push(ScriptedAnswer::Card(ans.clone()));
        ans
    }

    fn confirm(&mut self, prompt: &str) -> bool {
        let ans = self.inner.confirm(prompt);
        self.recording.push(ScriptedAnswer::Confirm(ans));
        ans
    }

    fn choose_player(&mut self, req: ChoosePlayerRequest) -> Option<PlayerId> {
        let ans = self.inner.choose_player(req);
        self.recording.push(ScriptedAnswer::Player(ans));
        ans
    }

    fn choose_int(&mut self, req: ChooseIntRequest) -> i32 {
        let ans = self.inner.choose_int(req);
        self.recording.push(ScriptedAnswer::Int(ans));
        ans
    }

    /// Forwards to inner. Not added to the recording — `ResponseAction`
    /// isn't a `ScriptedAnswer` variant and the suicide-retry replay only
    /// flips the first `choose_player`, not response decisions. Re-running
    /// the same handler with the same answer sequence will produce the
    /// same response decisions because RandomOracle's rng is deterministic.
    fn respond_or_pass(&mut self, state: &GameState, player: PlayerId) -> ResponseAction {
        self.inner.respond_or_pass(state, player)
    }
}
