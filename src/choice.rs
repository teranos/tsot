//! Choice oracles — the "thing that answers choice questions" from Lua handlers.
//!
//! When a card handler asks `game.choose_card(pool, opts)`, the engine
//! delegates the decision to an oracle. Different oracle implementations
//! plug in for different contexts (sim, tests, future UI).

use crate::game::{GameState, InstanceId, PlayChoices, PlayerId, StackItem};
use rand::Rng;
use std::collections::VecDeque;

/// Side-channel hint declaring what the NEXT `choose_card` call is for.
/// Set via `ChoiceOracle::set_next_intent`, consumed (cleared) on the next
/// `choose_card`. Lets the oracle pick intent-specific scoring without
/// changing the `ChooseCardRequest` signature (which would touch every
/// existing handler and ScriptedOracle fixture).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetIntent {
    /// "Pick an opp's loaded host I want to take attached from."
    /// Opp-controller bias plus attached-count bias (jewels, statics).
    /// Used by shift's source pick and falter's target.
    Steal,
    /// "Pick a card on my side I want to enrich."
    /// Asker-controlled bias; bonus for creature kind (so granted
    /// activations land on a body) and for existing attached count
    /// (consolidation play for Phase-3 grant-stacks). Used by shift's
    /// destination pick.
    Donate,
    /// "Pick the highest-value attached card."
    /// No controller bias — caller has already gated by host. Prefers
    /// `OnAttachedAsCost` handlers (jewels), then statics, then cost-heavy.
    /// Used by shift's per-attached pick.
    HighValueAttached,
    /// "Pick the opponent's biggest threat to remove."
    /// Opp-controller bias plus body-aware scoring (effective stats,
    /// cost, handler density, statics). Distinct from `Steal`: body
    /// matters here, attached count doesn't. Used by silent-murder,
    /// beguile, condemn, bring-down, jellyfish, this-for-that's "take".
    RemoveThreat,
    /// "Pick the most-valuable card in my graveyard to bring back."
    /// No controller bias (own GY by definition). Prefers cost-heavy
    /// (recursion saves a re-payment), handler-bearing, and statics.
    /// Used by mesopelagic-fish, resurrect, wake-dead, philosopher.
    Recur,
    /// "Pick a card on my side I'm willing to give away."
    /// Asker-controlled bias plus INVERTED body-aware scoring — prefer
    /// LOW stats, LOW cost, NO handlers. Jewels and pitch-payoff cards
    /// take a big penalty (don't gift them). Used by this-for-that's
    /// "give" pool.
    LowValueOwn,
}

impl TargetIntent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "steal" => Some(Self::Steal),
            "donate" => Some(Self::Donate),
            "high_value_attached" => Some(Self::HighValueAttached),
            "remove_threat" => Some(Self::RemoveThreat),
            "recur" => Some(Self::Recur),
            "low_value_own" => Some(Self::LowValueOwn),
            _ => None,
        }
    }
}

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

/// What an oracle returns when it can't answer locally — STAGE_MACHINE
/// S7. Carries the full request payload back up the call stack so the
/// `StepEngine` can lift it into a `HumanPrompt::Choose*` yield. Most
/// oracles never produce this (they answer locally); only
/// `HumanReplayOracle` does, and only when its pre-loaded answer queue
/// is exhausted while the asker is the human side.
#[derive(Debug, Clone)]
pub enum ChoicePending {
    Card(ChooseCardRequest),
    Confirm { asker: PlayerId, prompt: String },
    Player(ChoosePlayerRequest),
    Int(ChooseIntRequest),
}

/// Oracle trait — implementors answer choice questions on behalf of a
/// player. All choice methods receive `&GameState` so the oracle can
/// introspect: controllers, stats, handlers, zones, etc.
///
/// Methods return `Result<_, ChoicePending>` so a human-driving oracle
/// (e.g. `HumanReplayOracle` under `StepEngine`) can surface "needs the
/// human's answer" up through `?`-propagation rather than blocking on a
/// channel or panicking. Oracles that always answer locally
/// (`RandomOracle`, `NoopOracle`, `ScriptedOracle`) always return `Ok`.
pub trait ChoiceOracle {
    /// Pick one card from a pool, or None if optional and skipped.
    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Result<Option<InstanceId>, ChoicePending>;

    /// Yes/no decision. Used by `game.confirm`.
    fn confirm(
        &mut self,
        state: &GameState,
        asker: PlayerId,
        prompt: &str,
    ) -> Result<bool, ChoicePending>;

    /// Pick a player from `{A, B} - exclude`. Returns None if the candidate
    /// pool is empty, or if optional and the oracle declines.
    fn choose_player(
        &mut self,
        state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Result<Option<PlayerId>, ChoicePending>;

    /// Pick an integer in `[min, max]`. Mandatory — no opt-out.
    fn choose_int(
        &mut self,
        state: &GameState,
        req: ChooseIntRequest,
    ) -> Result<i32, ChoicePending>;

    /// Inside an open response window, `player` has priority — should they
    /// cast something from hand as a response, or pass? Default impl: Pass.
    fn respond_or_pass(&mut self, _state: &GameState, _player: PlayerId) -> ResponseAction {
        ResponseAction::Pass
    }

    /// Side-channel hint: declare the purpose of the NEXT `choose_card`
    /// call. Consumed (cleared) by the next `choose_card`. No-op default
    /// for oracles that don't score (Scripted, Noop).
    fn set_next_intent(&mut self, _intent: Option<TargetIntent>) {}
}

/// Random oracle — sim default. Picks uniformly random from the pool,
/// biases toward "activity" on optional choices (70% pick, 30% skip).
/// Filter logic is the card-author's responsibility — the oracle just
/// picks from whatever pool is passed.
pub struct RandomOracle<R: Rng> {
    rng: R,
    next_intent: Option<TargetIntent>,
}

impl<R: Rng> RandomOracle<R> {
    pub fn new(rng: R) -> Self {
        Self {
            rng,
            next_intent: None,
        }
    }
}

impl<R: Rng> ChoiceOracle for RandomOracle<R> {
    fn set_next_intent(&mut self, intent: Option<TargetIntent>) {
        self.next_intent = intent;
    }

    fn choose_card(
        &mut self,
        state: &GameState,
        req: ChooseCardRequest,
    ) -> Result<Option<InstanceId>, ChoicePending> {
        if req.pool.is_empty() {
            return Ok(None);
        }

        // Invulnerability gate: an opponent-controlled candidate with
        // the `invulnerability` keyword can't be targeted. Filtered
        // pre-scoring so the option doesn't even reach the picker.
        // Own-controlled invulnerable cards remain pickable (a card
        // can target itself or its ally regardless of the keyword).
        // Mirrors how MtG-style hexproof gates the choose step.
        let pool: Vec<InstanceId> = if let Some(asker) = req.asker {
            req.pool
                .iter()
                .filter(|iid| {
                    state
                        .card_pool
                        .get(*iid)
                        .map(|c| c.controller == asker || !state.has_keyword(iid, "invulnerability"))
                        .unwrap_or(true)
                })
                .cloned()
                .collect()
        } else {
            req.pool.clone()
        };
        if pool.is_empty() {
            return Ok(None);
        }

        if req.optional && self.rng.gen_bool(0.3) {
            return Ok(None);
        }

        // Consume any intent set via set_next_intent; cleared even on
        // skip-by-optional path above so a deferred handler doesn't reuse
        // someone else's intent. Intent takes precedence over host /
        // asker-default scoring when present.
        let intent = self.next_intent.take();

        // Score each candidate. Higher = preferred.
        let scores: Vec<i32> = pool
            .iter()
            .map(|iid| {
                if let (Some(intent), Some(asker)) = (intent, req.asker) {
                    intent_score(state, iid, asker, intent)
                } else if let Some(host_iid) = &req.host {
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
        Ok(Some(req.pool[pick].clone()))
    }

    fn confirm(
        &mut self,
        _state: &GameState,
        _asker: PlayerId,
        _prompt: &str,
    ) -> Result<bool, ChoicePending> {
        Ok(self.rng.gen_bool(0.7))
    }

    fn choose_player(
        &mut self,
        _state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Result<Option<PlayerId>, ChoicePending> {
        let candidates: Vec<PlayerId> = [PlayerId::A, PlayerId::B]
            .into_iter()
            .filter(|p| !req.exclude.contains(p))
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }
        if req.optional && self.rng.gen_bool(0.3) {
            return Ok(None);
        }
        let idx = self.rng.gen_range(0..candidates.len());
        Ok(Some(candidates[idx]))
    }

    fn choose_int(
        &mut self,
        _state: &GameState,
        req: ChooseIntRequest,
    ) -> Result<i32, ChoicePending> {
        let lo = req.min.min(req.max);
        let hi = req.min.max(req.max);
        Ok(self.rng.gen_range(lo..=hi))
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
            1.0
        } else if chain_threat {
            // Cost-scaled response when not in die-soon mode: small spells
            // aren't worth countering; expensive ones must be.
            // < 2 cost → 0%, then 20%/step, 6+ → 100%.
            let target_cost: i32 = p
                .chain
                .last()
                .and_then(|item| {
                    let StackItem::PlayedCard { card, .. } = item;
                    state.card_pool.get(card)
                })
                .map(|inst| inst.card.cost.iter().map(|c| c.amount.max(0)).sum())
                .unwrap_or(0);
            if target_cost < 2 {
                0.0
            } else {
                ((target_cost - 1) as f64 * 0.20).min(1.0)
            }
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
        // Apply hand-cost reductions (modern-lcd-clock etc.). Without this,
        // when an on-board static reduces the cast's effective hand cost,
        // respond_or_pass keeps computing the unreduced count and submits
        // the wrong number of payments → play_card returns
        // WrongHandPaymentCount → spin.
        let hand_red = state
            .cost_reduction(&pick, crate::card::CostSource::Hand)
            .max(0) as usize;
        hand_need = hand_need.saturating_sub(hand_red);
        let hand_payment_ids: Vec<InstanceId> = if hand_need > 0 {
            // Identity-match filter so the picked payments match what
            // play_card will validate. Cast-side or pay-side empty
            // identity is a wildcard. Also exclude cards under a
            // `cannot_be_cost_paid` restriction (e.g., flesh-eating-plant
            // targeting insects); otherwise play_card refuses with
            // HandPaymentForbidden and the priority window spins.
            let cast_ident = state.card_identity(&pick);
            state
                .player(player)
                .hand
                .iter()
                .filter(|iid| **iid != pick)
                .filter(|iid| !state.has_restriction(iid, crate::card::Restriction::CannotBeCostPaid))
                .filter(|iid| {
                    if cast_ident.is_empty() {
                        return true;
                    }
                    let pay_ident = state.card_identity(iid);
                    !cast_ident.is_disjoint(&pay_ident)
                })
                .take(hand_need)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        // If the hand can't cover the cost (not enough identity-match
        // candidates), pass instead of submitting a doomed cast. Without
        // this, play_card refuses with WrongHandPaymentCount, the
        // priority window doesn't advance, and the oracle re-picks the
        // same card next iteration → spin until drive_window_to_close's
        // safety cap trips.
        if hand_payment_ids.len() < hand_need {
            return ResponseAction::Pass;
        }

        // P.12a: pick GY ids for the GRAVEYARD cost component, prioritizing
        // a color-anchor so the cast passes the new gate.
        let mut raw_gy_needed: usize = 0;
        for c in &inst.card.cost {
            if let crate::card::CostSource::Graveyard = c.source {
                raw_gy_needed += c.amount.max(0) as usize;
            }
        }
        let gy_red = state
            .cost_reduction(&pick, crate::card::CostSource::Graveyard)
            .max(0) as usize;
        let gy_needed = raw_gy_needed.saturating_sub(gy_red);
        let graveyard_payment_ids = if gy_needed > 0 {
            state.resolve_graveyard_payment(player, &pick, gy_needed)
        } else {
            Vec::new()
        };

        ResponseAction::Respond {
            card: pick,
            choices: PlayChoices {
                hand_payment_ids,
                x_value: None,
                jewel_tap: None,
                sacrifice_ids: vec![],
                mutation_target: None,
                gy_hand_payment_ids: vec![],
                attached_payment_ids: vec![],
                graveyard_payment_ids,
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
    score -= (x + y / 2.0).round() as i32;

    score
}

/// Heuristic: "should I target this candidate?" Used by RandomOracle when
/// `req.host` is None but `req.asker` is set. Higher score = more
/// preferable target. Signals:
/// - Controller bonus: prefer opponent-controlled candidates (the
///   "don't grief yourself" default) with a large +100 anchor so the
///   filter dominates the within-group ranking.
/// - Body weight: effective X is the primary threat axis on board;
///   X×4 + Y captures both damage and durability. Applies to hand
///   cards via printed stats too — a 4/4 in hand is still a threat.
/// - Cost weight: high-investment cards are tomorrow's threats.
///   Exiling them denies more value than removing cheap stuff. Sums
///   the printed cost amounts × 3.
/// - Handler density: each event handler is a payoff. Anthem statics,
///   pitch-payoff jewels, and on_attack triggers all add signal.
fn target_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = 0i32;
    if cand.controller != asker {
        score += 100;
    }
    let (x, y) = state.effective_stats(candidate_iid);
    score += (x * 4.0 + y).round() as i32;
    let cost_sum: i32 = cand.card.cost.iter().map(|c| c.amount.max(0)).sum();
    score += cost_sum * 3;
    // Each handler is a payoff; flat +5 per kind present so a card with
    // multiple triggers ranks above a vanilla body of similar size.
    score += (cand.card.handlers.len() as i32) * 5;
    // Anthem / restriction / keyword-grant statics are board-wide impact.
    if cand.card.static_def.is_some() {
        score += 15;
    }
    // Pitch-payoff (OnAttachedAsCost) cards — jewels, mantis-shrimp,
    // zebra — are recurring tools. Exiling them denies future leverage.
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        score += 10;
    }
    score
}

/// Intent-aware target scoring. Dispatched to per-intent scoring fns
/// when a handler has set the next-intent side-channel on the oracle.
/// Replaces the default `target_score` for that single call only.
fn intent_score(
    state: &GameState,
    candidate_iid: &InstanceId,
    asker: PlayerId,
    intent: TargetIntent,
) -> i32 {
    match intent {
        TargetIntent::Steal => steal_score(state, candidate_iid, asker),
        TargetIntent::Donate => donate_score(state, candidate_iid, asker),
        TargetIntent::HighValueAttached => attached_value_score(state, candidate_iid),
        TargetIntent::RemoveThreat => remove_threat_score(state, candidate_iid, asker),
        TargetIntent::Recur => recur_score(state, candidate_iid),
        TargetIntent::LowValueOwn => low_value_own_score(state, candidate_iid, asker),
    }
}

/// "Pick from opponent's side." Strong opp anchor (+1000) so any
/// opp-controlled candidate outranks any own-controlled candidate in a
/// mixed pool. Within opp candidates: bonus per attached card (more to
/// take), with extra weight for attached cards bearing pitch-payoff
/// handlers (jewels) or statics (anthems). Plain stat-value doesn't
/// matter for shift-style steals because the body stays put — only the
/// attached cards move.
fn steal_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = if cand.controller != asker { 1000 } else { -100 };
    for a_iid in &cand.attached {
        score += 10;
        let Some(att) = state.card_pool.get(a_iid) else {
            continue;
        };
        if att
            .card
            .handlers
            .contains_key(&crate::card::EventName::OnAttachedAsCost)
        {
            score += 30;
        }
        if att.card.static_def.is_some() {
            score += 15;
        }
    }
    score
}

/// "Pick from my side to enrich." Strong self anchor (+1000) so an own
/// card outranks an opp card in a mixed pool. Within own candidates:
/// creatures (+50) beat artifacts because granted activations want a
/// body that can attack; bigger creature beats smaller (x*4 + y);
/// already-loaded host beats naked (+10 per attached) because Phase-3
/// jewel grants stack on a single host.
fn donate_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = if cand.controller == asker { 1000 } else { -100 };
    if cand.card.kind == crate::card::CardType::Creature {
        score += 50;
    }
    let (x, y) = state.effective_stats(candidate_iid);
    score += (x * 4.0 + y).round() as i32;
    score += (cand.attached.len() as i32) * 10;
    score
}

/// "Pick the highest-value attached." Caller has already gated by host,
/// so no controller bias. Big bonus for pitch-payoff handlers (jewels
/// grant activations to the new host post-shift), moderate for statics
/// (anthems re-target), and a cost-weight tiebreaker.
fn attached_value_score(state: &GameState, candidate_iid: &InstanceId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = 0i32;
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        score += 100;
    }
    if cand.card.static_def.is_some() {
        score += 50;
    }
    let cost_sum: i32 = cand.card.cost.iter().map(|c| c.amount.max(0)).sum();
    score += cost_sum * 3;
    score
}

/// "Pick the biggest threat on opp's side to remove." Opp anchor +1000;
/// within opp candidates, body weight (X*4 + Y) dominates, with cost
/// and handler density as secondary signals (cost-heavy = bigger
/// committed investment; handler density = more payoffs). Used by
/// removal cards (silent-murder, beguile, condemn, bring-down,
/// jellyfish, this-for-that's "take"). Distinct from `Steal`: attached
/// count doesn't matter here — we're removing the body, not stealing
/// what's on it.
fn remove_threat_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = if cand.controller != asker { 1000 } else { -100 };
    let (x, y) = state.effective_stats(candidate_iid);
    score += (x * 4.0 + y).round() as i32;
    let cost_sum: i32 = cand.card.cost.iter().map(|c| c.amount.max(0)).sum();
    score += cost_sum * 3;
    score += (cand.card.handlers.len() as i32) * 5;
    if cand.card.static_def.is_some() {
        score += 15;
    }
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        score += 10;
    }
    score
}

/// "Pick the most-valuable card in (own) graveyard to bring back."
/// No controller bias — recursion is from own GY by convention; the
/// caller already filtered the pool. Prefers high-cost (saves a
/// re-payment on resolve), handler-bearing (silent-murder, surge,
/// draw-two), and static-bearing cards. Stat-light because most recur
/// targets are non-creatures (mesopelagic-fish filters to non-creature)
/// or creature stats already encoded in cost.
fn recur_score(state: &GameState, candidate_iid: &InstanceId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = 0i32;
    let cost_sum: i32 = cand.card.cost.iter().map(|c| c.amount.max(0)).sum();
    score += cost_sum * 5;
    score += (cand.card.handlers.len() as i32) * 10;
    if cand.card.static_def.is_some() {
        score += 30;
    }
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        score += 20;
    }
    score
}

/// "Pick the least valuable card on my side to give away." Asker
/// anchor +1000, then INVERSE body-aware scoring. Stats, cost, and
/// handler density all subtract. Jewels (`OnAttachedAsCost`) and
/// statics take an extra penalty — don't gift pitch-payoff engines or
/// anthems. Used by this-for-that's "give" pool (opp gets the worst
/// non-creature you control in exchange for their best creature).
fn low_value_own_score(state: &GameState, candidate_iid: &InstanceId, asker: PlayerId) -> i32 {
    let Some(cand) = state.card_pool.get(candidate_iid) else {
        return 0;
    };
    let mut score = if cand.controller == asker { 1000 } else { -100 };
    let (x, y) = state.effective_stats(candidate_iid);
    score -= (x * 4.0 + y).round() as i32;
    let cost_sum: i32 = cand.card.cost.iter().map(|c| c.amount.max(0)).sum();
    score -= cost_sum * 3;
    score -= (cand.card.handlers.len() as i32) * 5;
    if cand.card.static_def.is_some() {
        score -= 30;
    }
    if cand
        .card
        .handlers
        .contains_key(&crate::card::EventName::OnAttachedAsCost)
    {
        score -= 50;
    }
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
        .map(|iid| state.effective_stats(iid).0.floor() as i32)
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
                        state.effective_stats(card).0.floor() as i32
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
    ) -> Result<Option<InstanceId>, ChoicePending> {
        Ok(None)
    }
    fn confirm(
        &mut self,
        _state: &GameState,
        _asker: PlayerId,
        _prompt: &str,
    ) -> Result<bool, ChoicePending> {
        Ok(false)
    }
    fn choose_player(
        &mut self,
        _state: &GameState,
        _req: ChoosePlayerRequest,
    ) -> Result<Option<PlayerId>, ChoicePending> {
        Ok(None)
    }
    fn choose_int(
        &mut self,
        _state: &GameState,
        req: ChooseIntRequest,
    ) -> Result<i32, ChoicePending> {
        Ok(req.min)
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
    ) -> Result<Option<InstanceId>, ChoicePending> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Card(c)) => Ok(c),
            Some(other) => panic!("ScriptedOracle: expected Card answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn confirm(
        &mut self,
        _state: &GameState,
        _asker: PlayerId,
        _prompt: &str,
    ) -> Result<bool, ChoicePending> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Confirm(b)) => Ok(b),
            Some(other) => panic!("ScriptedOracle: expected Confirm answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_player(
        &mut self,
        _state: &GameState,
        _req: ChoosePlayerRequest,
    ) -> Result<Option<PlayerId>, ChoicePending> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Player(p)) => Ok(p),
            Some(other) => panic!("ScriptedOracle: expected Player answer, got {other:?}"),
            None => panic!("ScriptedOracle: out of answers"),
        }
    }

    fn choose_int(
        &mut self,
        _state: &GameState,
        _req: ChooseIntRequest,
    ) -> Result<i32, ChoicePending> {
        match self.answers.pop_front() {
            Some(ScriptedAnswer::Int(n)) => Ok(n),
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
    ) -> Result<Option<InstanceId>, ChoicePending> {
        let ans = self.inner.choose_card(state, req)?;
        self.recording.push(ScriptedAnswer::Card(ans.clone()));
        Ok(ans)
    }

    fn confirm(
        &mut self,
        state: &GameState,
        asker: PlayerId,
        prompt: &str,
    ) -> Result<bool, ChoicePending> {
        let ans = self.inner.confirm(state, asker, prompt)?;
        self.recording.push(ScriptedAnswer::Confirm(ans));
        Ok(ans)
    }

    fn choose_player(
        &mut self,
        state: &GameState,
        req: ChoosePlayerRequest,
    ) -> Result<Option<PlayerId>, ChoicePending> {
        let ans = self.inner.choose_player(state, req)?;
        self.recording.push(ScriptedAnswer::Player(ans));
        Ok(ans)
    }

    fn choose_int(
        &mut self,
        state: &GameState,
        req: ChooseIntRequest,
    ) -> Result<i32, ChoicePending> {
        let ans = self.inner.choose_int(state, req)?;
        self.recording.push(ScriptedAnswer::Int(ans));
        Ok(ans)
    }

    /// Forwards to inner. Not added to the recording — `ResponseAction`
    /// isn't a `ScriptedAnswer` variant and the suicide-retry replay only
    /// flips the first `choose_player`, not response decisions. Re-running
    /// the same handler with the same answer sequence will produce the
    /// same response decisions because RandomOracle's rng is deterministic.
    fn respond_or_pass(&mut self, state: &GameState, player: PlayerId) -> ResponseAction {
        self.inner.respond_or_pass(state, player)
    }

    /// Forwards to inner. Not added to the recording — intent is a
    /// side-channel hint, not a recordable answer. Suicide-retry replay
    /// is consistent because the same handler called the same way will
    /// re-set the same intent before each choose_card.
    fn set_next_intent(&mut self, intent: Option<TargetIntent>) {
        self.inner.set_next_intent(intent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::{ModifierValue, StaticAffects, StaticDef};
    use crate::game::test_helpers::deck_of;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn put_on_board(s: &mut GameState, side: PlayerId, iid: &InstanceId) {
        s.player_mut(side).hand.retain(|x| x != iid);
        s.player_mut(side).board.push(iid.clone());
    }

    fn give_static_def(s: &mut GameState, iid: &InstanceId) {
        s.card_pool.get_mut(iid).unwrap().card.static_def = Some(StaticDef {
            affects: StaticAffects::default(),
            modifier_x: ModifierValue::default(),
            modifier_y: ModifierValue::default(),
            modifier_keyword: None,
            condition: None,
            restrictions: vec![],
            cost_modifiers: vec![],
            granted_activated: None,
            granted_colors: vec![],
granted_face: Vec::new(),
        });
    }

    fn force_attach(s: &mut GameState, host: &InstanceId, attached: &InstanceId) {
        s.card_pool
            .get_mut(host)
            .unwrap()
            .attached
            .push(attached.clone());
    }

    fn req(pool: Vec<InstanceId>) -> ChooseCardRequest {
        ChooseCardRequest {
            pool,
            asker: Some(PlayerId::A),
            host: None,
            optional: false,
            prompt: String::new(),
        }
    }

    #[test]
    fn steal_intent_picks_opp_loaded_host_over_own_naked() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let own = s.a.hand[0].clone();
        let opp = s.b.hand[0].clone();
        let opp_jewel = s.b.hand[1].clone();
        put_on_board(&mut s, PlayerId::A, &own);
        put_on_board(&mut s, PlayerId::B, &opp);
        give_static_def(&mut s, &opp_jewel);
        force_attach(&mut s, &opp, &opp_jewel);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::Steal));
        let pick = oracle.choose_card(&s, req(vec![own.clone(), opp.clone()])).unwrap();
        assert_eq!(pick, Some(opp));
    }

    #[test]
    fn donate_intent_picks_own_creature_over_opp() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let own = s.a.hand[0].clone();
        let opp = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &own);
        put_on_board(&mut s, PlayerId::B, &opp);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::Donate));
        let pick = oracle.choose_card(&s, req(vec![own.clone(), opp.clone()])).unwrap();
        assert_eq!(pick, Some(own));
    }

    #[test]
    fn high_value_attached_prefers_static_bearing_card() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let vanilla = s.a.hand[0].clone();
        let jewel = s.a.hand[1].clone();
        give_static_def(&mut s, &jewel);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::HighValueAttached));
        let pick = oracle.choose_card(&s, req(vec![vanilla.clone(), jewel.clone()])).unwrap();
        assert_eq!(pick, Some(jewel));
    }

    fn boost_stats(s: &mut GameState, iid: &InstanceId, dx: f32, dy: f32) {
        s.add_modifier(iid, crate::game::Modifier::StatBoost { x: dx, y: dy });
    }

    #[test]
    fn remove_threat_prefers_bigger_opp_body() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let small = s.b.hand[0].clone();
        let big = s.b.hand[1].clone();
        put_on_board(&mut s, PlayerId::B, &small);
        put_on_board(&mut s, PlayerId::B, &big);
        // Stats from deck_of are 1/1; boost `big` to 5/5.
        boost_stats(&mut s, &big, 4.0, 4.0);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::RemoveThreat));
        let pick = oracle.choose_card(&s, req(vec![small.clone(), big.clone()])).unwrap();
        assert_eq!(pick, Some(big));
    }

    #[test]
    fn recur_prefers_cost_heavy() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let cheap = s.a.hand[0].clone();
        let expensive = s.a.hand[1].clone();
        // Mock cost: cheap has nothing, expensive has 5-graveyard component.
        s.card_pool.get_mut(&expensive).unwrap().card.cost = vec![crate::card::CostComponent {
            amount: 5,
            source: crate::card::CostSource::Graveyard,
            kind: None,
            is_x: false,
        }];

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::Recur));
        let pick = oracle.choose_card(&s, req(vec![cheap.clone(), expensive.clone()])).unwrap();
        assert_eq!(pick, Some(expensive));
    }

    #[test]
    fn low_value_own_prefers_smallest_throwaway() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let throwaway = s.a.hand[0].clone();
        let jewel_like = s.a.hand[1].clone();
        put_on_board(&mut s, PlayerId::A, &throwaway);
        put_on_board(&mut s, PlayerId::A, &jewel_like);
        // Mark jewel_like as static-bearing (-30 penalty for LowValueOwn).
        give_static_def(&mut s, &jewel_like);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::LowValueOwn));
        let pick = oracle.choose_card(&s, req(vec![throwaway.clone(), jewel_like.clone()])).unwrap();
        assert_eq!(pick, Some(throwaway));
    }

    #[test]
    fn intent_is_consumed_after_one_call() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let own = s.a.hand[0].clone();
        let opp = s.b.hand[0].clone();
        put_on_board(&mut s, PlayerId::A, &own);
        put_on_board(&mut s, PlayerId::B, &opp);

        let mut oracle = RandomOracle::new(StdRng::seed_from_u64(0));
        oracle.set_next_intent(Some(TargetIntent::Donate));
        let _ = oracle.choose_card(&s, req(vec![own.clone(), opp.clone()]));
        // Second call without re-setting: Donate doesn't apply. Default
        // target_score has a +100 opp bias, so opp wins.
        let pick = oracle.choose_card(&s, req(vec![own.clone(), opp.clone()])).unwrap();
        assert_eq!(pick, Some(opp));
    }
}
