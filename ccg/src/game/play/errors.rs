//! Error types and the player-supplied `PlayChoices` struct for the
//! cast / activate paths. Extracted from `play.rs` so the cast loop
//! itself reads in one screen without scrolling past 200 lines of
//! enum variants.

use super::super::state::InstanceId;
use crate::card::{CardType, CostSource};

/// Outcomes for `activate_ability`. The sim AI is expected to call only
/// when validation will pass (cheap pre-checks), but the engine still
/// enforces each rule so manual call sites and replays stay honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateError {
    /// A Lua handler fired during the activation's `effect` called
    /// `game.choose_card` / `game.confirm` / `game.choose_player` /
    /// `game.choose_int` with an oracle that needs the human to answer.
    /// Mirrors `PlayError::ChoicePending` — wrapper raises
    /// `Error::external(ChoicePending)`, `fire_activated` downcasts,
    /// `activate_ability` lifts via `?`. The StepEngine catches this
    /// variant, rolls back the preview journal, surfaces a
    /// `HumanPrompt::Choose*`, and re-fires after the user's answer
    /// is appended to `HumanReplayOracle.replay`.
    ChoicePending(crate::choice::ChoicePending),
    /// Source iid not in the card pool.
    SourceMissing,
    /// `ability_idx` out of range for this card's `activated` array.
    NoSuchAbility,
    /// Source is not in the controller's BOARD zone. Activations from
    /// other zones (hand, graveyard, attached) are a v2 extension.
    NotOnBoard,
    /// Tap cost: source is already tapped.
    AlreadyTapped,
    /// Tap cost: source is a creature with B.3 summoning sickness and
    /// no haste.
    SummoningSick,
    /// One of the cost components cannot be paid from controller state
    /// (insufficient hand size, deck depth, graveyard size, etc.) or
    /// the cost source isn't supported by this v1 activation path
    /// (Sacrifice / SelfExile pending).
    CannotPayComponents,
    /// The ability's optional `validate` hook returned false (or
    /// errored). No cost is paid in this case — the hook's purpose is
    /// to refuse activation when no legal target exists, so the AI
    /// doesn't burn cards on a no-op.
    NoLegalTarget,
    /// RULES P.30: X < 1 on an X-cost activation that doesn't opt
    /// into X = 0 (`Card.allow_x_zero = false`).
    XBelowMinimum,
    /// SACRIFICE cost: caller's `ActivateChoices.sacrifice_ids` count
    /// does not match the activation's SACRIFICE component total.
    /// Mirrors `PlayError::WrongSacrificeCount` for cast paths.
    WrongSacrificeCount { expected: usize, got: usize },
    /// SACRIFICE cost: a chosen sacrifice id is not on the controller's
    /// BOARD, or not controlled by the controller, or fails the cost
    /// component's `kind` filter. Mirrors
    /// `PlayError::SacrificePaymentInvalid`.
    SacrificePaymentInvalid(InstanceId),
    /// SACRIFICE cost: the same id appears more than once in
    /// `sacrifice_ids`. Mirrors `PlayError::DuplicateSacrifice`.
    DuplicateSacrifice(InstanceId),
}

/// Player-supplied choices for an activation. Parallel to `PlayChoices`
/// on the cast side. Today carries only the SACRIFICE target list; the
/// struct shape is the future home for any other choices an activation
/// needs (e.g., per-activation jewel-tap substitution, ATTACHED
/// payment lists for activations from ATTACHED zone, etc.).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActivateChoices {
    /// One InstanceId per SACRIFICE cost component on the activated
    /// ability. Each id must be on the controller's BOARD, controlled
    /// by the controller, and match any `kind` filter declared on the
    /// SACRIFICE component (same shape as `PlayChoices.sacrifice_ids`).
    pub sacrifice_ids: Vec<InstanceId>,
}

/// Player-supplied choices when playing a card.
/// In this slice, only HAND payments require choice (which cards to spend).
/// MILL payments are deterministic (top N of DECK).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayChoices {
    /// One InstanceId per HAND cost-card the player chooses to spend.
    pub hand_payment_ids: Vec<InstanceId>,
    /// The value of X for variable-X cost components. Required if any cost
    /// component has `is_x: true`; the same X applies to every variable
    /// component on the card (per recast's `X hand + X graveyard` pattern).
    pub x_value: Option<i32>,
    /// P.24: optionally tap one untapped jewel on the player's BOARD whose
    /// colors share at least one with the cast card, to substitute for one
    /// HAND-source cost component. Max one per cast. The substituted HAND
    /// count is reduced by 1 (so `hand_payment_ids.len()` should be the
    /// already-reduced count).
    pub jewel_tap: Option<InstanceId>,
    /// P.16: one InstanceId per SACRIFICE cost component. Each ID must be
    /// on the player's BOARD and they control it. Moves BOARD → GRAVEYARD
    /// as part of cost payment; on_die fires per sacrificed card.
    pub sacrifice_ids: Vec<InstanceId>,
    /// MUTATION target: required when the cast card has `kind = Mutation`.
    /// Names the on-board creature the mutation will attach to. Any
    /// creature is a legal target (friendly or opposing).
    pub mutation_target: Option<InstanceId>,
    /// Clear View-style HAND-payment substitutes drawn from the
    /// controller's GRAVEYARD. Each iid must be in the controller's
    /// graveyard and have `Card.gy_hand_substitute = true`. Each one
    /// fills one HAND-source slot of the cast and moves GY → EXILE
    /// during cost payment. Does NOT satisfy P.7a identity for the
    /// cast — only the `hand_payment_ids` slots are identity-checked,
    /// so casts of identity-bearing spells still need at least one
    /// matching card in hand for each non-substituted slot.
    #[serde(default)]
    pub gy_hand_payment_ids: Vec<InstanceId>,
    /// P.31: one InstanceId per ATTACHED-source cost slot. Each id must
    /// currently be attached to a card the player controls on the BOARD.
    /// On resolution the cards detach and either re-attach to the played
    /// card (if BOARD-placed) or move to EXILE (non-BOARD).
    #[serde(default)]
    pub attached_payment_ids: Vec<InstanceId>,
    /// P.12 + P.12a: explicit choice of which GY cards to exile to pay
    /// `N graveyard` cost components. When non-empty, must contain
    /// exactly `graveyard_needed` ids and each must be in the player's
    /// GRAVEYARD; the engine exiles them in the provided order. When
    /// empty (the legacy path), the engine falls back to exiling the
    /// most-recent N cards from the back of the GY. The empty fallback
    /// keeps the slice's existing behavior byte-identical for callers
    /// that haven't migrated yet; P.12a's color-anchor rule (added in a
    /// follow-up slice) needs explicit ids to enforce.
    #[serde(default)]
    pub graveyard_payment_ids: Vec<InstanceId>,
    /// P.42: one InstanceId per `tap` cost slot. Each id must be an
    /// untapped permanent the player controls on the BOARD; the engine
    /// taps them as part of the cost (non-consumptive — they untap at
    /// U.2). Distinct ids; count must equal the card's total `tap` cost.
    #[serde(default)]
    pub tap_payment_ids: Vec<InstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayError {
    /// A Lua handler fired during play (e.g. `on_play`, `on_enter_board`,
    /// or a death-triggered `on_die` cascading from this cast) called
    /// `game.choose_card` / `game.confirm` / `game.choose_player` /
    /// `game.choose_int` with an oracle that needs the human to answer.
    /// The wrapper raises `Error::external(ChoicePending)`; `fire_*`
    /// downcasts; play_card lifts via `?` and propagates here. The
    /// StepEngine catches this variant, rolls back the preview journal,
    /// surfaces a `HumanPrompt::Choose*`, and re-fires play_card after
    /// the user's answer is appended to `HumanReplayOracle.replay`.
    ChoicePending(crate::choice::ChoicePending),
    GameOver,
    NotInHand,
    /// RULES P.41b: the card is not castable from the zone it currently
    /// occupies. Today the only case: a graveyard-only card (its declared
    /// `cast_zones` omit HAND) sitting in HAND — inert there until it
    /// reaches the GRAVEYARD. Refused before any cost is paid; the card
    /// stays where it is.
    NotCastableFromZone,
    /// Card type not currently routable by `play_card`. Today: Creature,
    /// Spell, Artifact. Environment still unsupported.
    UnsupportedType(CardType),
    /// Spell (sorcery timing) cast while a response window is open. Per
    /// R.1 + sorcery convention: sorceries are main-phase only.
    SorceryAtInstantSpeed,
    /// This slice supports HAND, MILL, and GRAVEYARD cost sources.
    UnsupportedCostSource(CostSource),
    /// GRAVEYARD doesn't have enough cards to pay the GRAVEYARD cost.
    InsufficientGraveyardForCost { needed: usize, have: usize },
    /// P.12: explicit `graveyard_payment_ids` was non-empty but its
    /// length doesn't match the card's total GRAVEYARD cost. The empty
    /// case falls back to the legacy "back of GY" behavior and is not
    /// a count error.
    WrongGraveyardPaymentCount { expected: usize, got: usize },
    /// P.12: a chosen GRAVEYARD-payment id isn't in the player's
    /// GRAVEYARD (or doesn't exist in the card pool).
    GraveyardPaymentInvalid(InstanceId),
    /// P.12: a GRAVEYARD-payment id appears more than once.
    DuplicateGraveyardPayment(InstanceId),
    /// P.12a: cast has non-empty colors and a GRAVEYARD-source cost
    /// component, but none of the cards being exiled (either the
    /// explicit `graveyard_payment_ids` or the legacy back-of-GY) share
    /// a printed color with the cast. The color-anchor requirement is
    /// lenient: a single color-matching pitch anywhere in the bundle
    /// satisfies it.
    NoGraveyardPaymentForColor,
    /// P.42: `tap` payment count doesn't equal the card's total `tap` cost.
    WrongTapPaymentCount { expected: usize, got: usize },
    /// P.42: a chosen `tap` payment isn't an untapped permanent the player
    /// controls on the BOARD (not on board, not controlled, already tapped,
    /// or not in the pool).
    InvalidTapPayment(InstanceId),
    /// P.42: a `tap` payment id appears more than once.
    DuplicateTapPayment(InstanceId),
    /// P.42a: cast has a `tap` component but no color anchor — no payment
    /// (tap, HAND, or GRAVEYARD) shares a printed color with the cast, and
    /// a colorless cast can never anchor.
    NoTapPaymentForColor,
    /// Card has a variable-X cost component but choices.x_value is None.
    VariableXValueMissing,
    /// RULES P.30: X < 1 on a card that doesn't opt into X = 0
    /// (`Card.allow_x_zero = false`).
    XBelowMinimum,
    /// HAND payment count must equal the card's total HAND cost.
    WrongHandPaymentCount { expected: usize, got: usize },
    /// A chosen HAND payment isn't in the player's hand, or is the card being played itself.
    HandPaymentInvalid(InstanceId),
    /// A HAND payment ID appears more than once in the choices.
    DuplicateHandPayment(InstanceId),
    /// DECK doesn't have enough cards to pay the MILL cost.
    InsufficientDeckForMill { needed: usize, have: usize },
    /// P.24: jewel-tap substitution declared, but the chosen card isn't a
    /// valid jewel for this cast (not on player's BOARD, not untapped, not
    /// a jewel subtype, or color mismatch with cast card).
    InvalidJewelTap(InstanceId),
    /// P.24: jewel-tap declared on a card with no HAND-source cost component
    /// to substitute (would substitute nothing).
    JewelTapWithoutHandCost,
    /// Phase 3: a static restriction (e.g., flesh-eating-plant's
    /// `cannot_be_cost_paid`) forbids using this card as a HAND payment.
    HandPaymentForbidden(InstanceId),
    /// HAND payment doesn't share an identity element (color or
    /// symbol) with the casting card. Cards with no colors and no
    /// symbol act as wildcards on either side — this only fires when
    /// both have non-empty identity sets that don't intersect.
    HandPaymentIdentityMismatch(InstanceId),
    /// MUTATION cast missing a target creature.
    MutationTargetMissing,
    /// MUTATION target isn't a creature on either BOARD.
    MutationTargetInvalid(InstanceId),
    /// Z.7: the target sleeve is full — a sleeve holds at most 4 cards (a
    /// host and up to 3 same-sleeve mutations), so a 4th mutation is refused.
    SleeveFull(InstanceId),
    /// P.16: SACRIFICE payment count doesn't match the card's total
    /// SACRIFICE cost.
    WrongSacrificeCount { expected: usize, got: usize },
    /// P.16: a chosen sacrifice ID isn't on the player's BOARD, or the
    /// player doesn't control it.
    SacrificePaymentInvalid(InstanceId),
    /// P.16: a sacrifice ID appears more than once in the choices.
    DuplicateSacrifice(InstanceId),
    /// A GY-hand-substitute payment isn't in the player's graveyard.
    GyHandSubstituteNotInGraveyard(InstanceId),
    /// A GY-hand-substitute payment doesn't have
    /// `Card.gy_hand_substitute = true` — only Clear View-style cards
    /// qualify today.
    GyHandSubstituteNotEligible(InstanceId),
    /// Same iid appears twice in `gy_hand_payment_ids`.
    DuplicateGyHandSubstitute(InstanceId),
    /// `gy_hand_payment_ids` declared on a card with no HAND-source
    /// cost component to substitute (would substitute nothing).
    GyHandSubstituteWithoutHandCost,
    /// P.31: ATTACHED payment count doesn't match the card's total
    /// ATTACHED cost.
    WrongAttachedPaymentCount { expected: usize, got: usize },
    /// P.31: a chosen attached id isn't attached to a card the player
    /// controls on the BOARD.
    AttachedPaymentInvalid(InstanceId),
    /// P.31: an attached payment id appears more than once.
    DuplicateAttachedPayment(InstanceId),
    /// The card's optional `validate` hook returned false at cast time —
    /// typically "no legal target exists for this card." No cost is paid
    /// (the check runs before any state mutation). Parallel to
    /// `ActivateError::NoLegalTarget` but for cast.
    CastValidateFailed,
    /// All HAND slots were filled by GY substitutes on a cast that
    /// requires identity matching (cast has non-empty colors or
    /// symbols). Clear View doesn't carry identity, so at least one
    /// HAND payment from hand is required when the cast has any
    /// identity at all. A 1-hand blue cast can't be paid solely by
    /// Clear View — there's no hand-payment slot left to satisfy
    /// P.7a's identity check.
    NoHandPaymentForIdentity,
    /// P.35: a player may cast at most one Symbol card per turn. The
    /// cap is checked before any cost is paid; the second cast is
    /// refused with the card still in HAND.
    SymbolCastCapReached,
    /// P.36: a Symbol card with the same `id` is already on either
    /// player's BOARD. The cast is refused before any cost is paid.
    /// When the first Symbol leaves BOARD the id becomes castable
    /// again (no replacement effect — the second cast must be a fresh
    /// attempt).
    SymbolUniquenessViolated,
}
