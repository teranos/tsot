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

/// A choose-card prompt with the pool and options. All call sites now pass
/// `state` to the oracle separately, so the oracle reads controllers /
/// stats / handlers itself — no parallel metadata vecs on the request.
#[derive(Debug, Clone)]
pub struct ChooseCardRequest {
    pub pool: Vec<InstanceId>,
    /// The owner of the handler issuing the choice. None when the caller
    /// doesn't have a player context (tests, headless eval).
    pub asker: Option<PlayerId>,
    /// When this `choose_card` is for a hand-payment slot, `host` is the
    /// InstanceId of the card being paid for. The oracle uses it to score
    /// candidates against pitch-payoff effects (jewels / zebra / mantis-shrimp
    /// matching the host's color). None for non-payment choices (target
    /// pickers, recur pickers, etc.).
    pub host: Option<InstanceId>,
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

/// Oracle trait — implementors answer choice questions on behalf of a
/// player. All choice methods receive `&GameState` so the oracle can
/// introspect: controllers, stats, handlers, zones, etc.
pub trait ChoiceOracle {
    /// Pick one card from a pool, or None if optional and skipped.
    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Option<InstanceId>;

    /// Yes/no decision. Used by `game.confirm`.
    fn confirm(&mut self, state: &GameState, asker: PlayerId, prompt: &str) -> bool;

    /// Pick a player from `{A, B} - exclude`. Returns None if the candidate
    /// pool is empty, or if optional and the oracle declines.
    fn choose_player(
        &mut self,
        state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Option<PlayerId>;

    /// Pick an integer in `[min, max]`. Mandatory — no opt-out.
    fn choose_int(&mut self, state: &GameState, req: ChooseIntRequest) -> i32;

    /// Inside an open response window, `player` has priority — should they
    /// cast something from hand as a response, or pass? Default impl: Pass.
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
    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Option<InstanceId> {
        if req.pool.is_empty() {
            return None;
        }
        if req.optional && self.rng.gen_bool(0.3) {
            return None;
        }

        // Score each candidate. Higher = preferred.
        let scores: Vec<i32> = req
            .pool
            .iter()
            .map(|iid| {
                if let Some(host_iid) = &req.host {
                    pitch_score(state, iid, host_iid)
                } else if let Some(asker) = req.asker {
                    target_score(state, iid, asker)
                } else {
                    0
                }
            })
            .collect();

        let max_score = *scores.iter().max().unwrap_or(&0);
        let top: Vec<usize> = scores
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == max_score)
            .map(|(i, _)| i)
            .collect();
        let pick = top[self.rng.gen_range(0..top.len())];
        Some(req.pool[pick].clone())
    }

    fn confirm(&mut self, _state: &GameState, _asker: PlayerId, _prompt: &str) -> bool {
        self.rng.gen_bool(0.7)
    }

    fn choose_player(
        &mut self,
        _state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Option<PlayerId> {
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

    fn choose_int(&mut self, _state: &GameState, req: ChooseIntRequest) -> i32 {
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
                jewel_tap: None,
            },
        }
    }
}

/// Heuristic: "should I pitch this candidate to pay for `host`?" Higher =
/// more preferable. Used by RandomOracle when `req.host` is set. Signals:
/// pitch-payoff handler + color match (big bonus), artifact / uncastable
/// (bonus), cast-value handlers (penalty), effective stats (penalty).
fn pitch_score(state: &GameState, candidate_iid: &InstanceId, host_iid: &InstanceId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let Some(host) = state.card_pool.get(host_iid) else {
        return 0;
    };
    let mut score = 0i32;

    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        let host_is_creature = host.card.kind == crate::card::CardType::Creature;
        let color_overlap = cand
            .card
            .colors
            .iter()
            .any(|cc| host.card.colors.iter().any(|hc| hc.eq_ignore_ascii_case(cc)));
        if host_is_creature && color_overlap {
            score += 100;
        } else if host_is_creature {
            score += 30;
        } else {
            score -= 50;
        }
    }

    if matches!(cand.card.kind, crate::card::CardType::Artifact) {
        score += 50;
    }

    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnPlay)
    {
        score -= 20;
    }
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnEnterBoard)
    {
        score -= 10;
    }
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttack)
    {
        score -= 10;
    }

    let (x, y) = state.effective_stats(candidate_iid);
    score -= x + y / 2;

    score
}

/// Heuristic: "should I target this candidate?" Used by RandomOracle when
/// `req.host` is None but `req.asker` is set. Today: prefer opponent-controlled
/// (the "don't grief yourself" default) and, within the filtered set, prefer
/// candidates with higher effective X (= more threatening). Returns 0 when
/// state lookups fail or there's no useful signal.
fn target_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = 0i32;
    if cand.controller != asker {
        score += 100;
    }
    let (x, y) = state.effective_stats(candidate_iid);
    score += x * 4 + y;
    score
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
    fn choose_card(
        &mut self,
        _state: &GameState,
        _req: ChooseCardRequest,
    ) -> Option<InstanceId> {
        None
    }
    fn confirm(&mut self, _state: &GameState, _asker: PlayerId, _prompt: &str) -> bool {
        false
    }
    fn choose_player(
        &mut self,
        _state: &GameState,
        _req: ChoosePlayerRequest,
    ) -> Option<PlayerId> {
        None
    }
    fn choose_int(&mut self, _state: &GameState, req: ChooseIntRequest) -> i32 {
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
    fn choose_card(
        &mut self,
        _state: &GameState,
        _req: ChooseCardRequest,
    ) -> Option<InstanceId> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Card(c)) => c,
            Some(other) => panic!("ScriptedOracle: expected Card answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn confirm(&mut self, _state: &GameState, _asker: PlayerId, _prompt: &str) -> bool {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Confirm(b)) => b,
            Some(other) => panic!("ScriptedOracle: expected Confirm answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_player(
        &mut self,
        _state: &GameState,
        _req: ChoosePlayerRequest,
    ) -> Option<PlayerId> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Player(p)) => p,
            Some(other) => panic!("ScriptedOracle: expected Player answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_int(&mut self, _state: &GameState, _req: ChooseIntRequest) -> i32 {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Int(n)) => n,
            Some(other) => panic!("ScriptedOracle: expected Int answer, got {other:?}"),
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
    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Option<InstanceId> {
        let ans = self.inner.choose_card(state, req);
        self.recording.push(ScriptedAnswer::Card(ans.clone()));
        ans
    }

    fn confirm(&mut self, state: &GameState, asker: PlayerId, prompt: &str) -> bool {
        let ans = self.inner.confirm(state, asker, prompt);
        self.recording.push(ScriptedAnswer::Confirm(ans));
        ans
    }

    fn choose_player(
        &mut self,
        state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Option<PlayerId> {
        let ans = self.inner.choose_player(state, req);
        self.recording.push(ScriptedAnswer::Player(ans));
        ans
    }

    fn choose_int(&mut self, state: &GameState, req: ChooseIntRequest) -> i32 {
        let ans = self.inner.choose_int(state, req);
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
