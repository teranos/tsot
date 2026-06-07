//! Card-kind routing properties for the cast resolution path. Single
//! source of truth for "where does a card of this kind go when played,
//! and what events fire around it." Both the EA's `playable_pool`
//! filter (`main.rs`) and `play_card`'s type guard / resolution body
//! (`game::play`) consult this trait.
//!
//! Adding a new `CardType` variant: implement its booleans here, then
//! the rest of the engine picks it up automatically. `is_castable`
//! must return `true` for any variant the resolver can handle; if it
//! returns `false`, `play_card` rejects the cast with `UnsupportedType`.

use crate::card::CardType;

pub trait CastRouting {
    /// True when the resolver has a routing path for this kind.
    /// `play_card` returns `PlayError::UnsupportedType` when this is
    /// `false`. The EA's `playable_pool` filter uses the same gate so
    /// the gene pool stays aligned with what the engine can actually
    /// cast.
    fn is_castable(&self) -> bool;

    /// True when the card resolves onto the BOARD (per P.2 / P.19 /
    /// P.21). Implies HAND payments attach (P.6), ATTACHED payments
    /// re-attach (P.31 BOARD branch), `on_attached_as_cost` and
    /// `on_enter_board` events fire.
    fn is_board_placed(&self) -> bool;

    /// True when the card resolves by attaching to another card on
    /// resolution (P.26: mutation). The cast card never enters the
    /// BOARD as a separate slot; HAND payments follow the spell
    /// convention (→ GRAVEYARD).
    fn attaches_to_target(&self) -> bool;

    /// True when B.3 summoning sickness applies on entry. Currently
    /// `Creature` only; artifacts/environments skip it.
    fn applies_summoning_sickness(&self) -> bool;
}

impl CastRouting for CardType {
    fn is_castable(&self) -> bool {
        matches!(
            self,
            CardType::Creature
                | CardType::Spell
                | CardType::Artifact
                | CardType::Mutation
                | CardType::Symbol
                | CardType::Unspecified
        )
    }

    fn is_board_placed(&self) -> bool {
        // P.37: Symbol joins Creature + Artifact on the BOARD-placed
        // path. Environment intentionally stays out — `is_castable`
        // rejects it ahead of routing.
        matches!(
            self,
            CardType::Creature | CardType::Artifact | CardType::Symbol
        )
    }

    fn attaches_to_target(&self) -> bool {
        matches!(self, CardType::Mutation)
    }

    fn applies_summoning_sickness(&self) -> bool {
        // C.17a: Symbol skips B.3, parallel to Artifact's exemption.
        matches!(self, CardType::Creature)
    }
}
