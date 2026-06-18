//! Two pools.
//!
//! `playable_pool` is the EA / genetic-algorithm candidate set —
//! cards that the search is allowed to pick as gene values. It's
//! INTENTIONALLY narrow: it excludes anything the EA would waste
//! budget exploring (variants, test-subtype probes, X-cost spells
//! without an `on_play` handler, debug-subtype cards, etc.). The
//! filter matches the original main.rs inline computation byte-for-
//! byte plus a `debug` subtype exclusion added 2026-06-18 so the
//! `faal` chaos-engineering card stays out of EA gauntlets.
//!
//! `deckbuilder_pool` is the human-deckbuilder candidate set — every
//! card the engine can physically route through `play_card`. It's
//! strictly larger than `playable_pool`: debug-subtype cards, X-cost
//! spells without `on_play` handlers, etc. are all KEPT here because
//! a human author may deliberately include them in a custom or
//! preset deck (e.g. the Faal Test preset which is itself a chaos-
//! engineering verification deck).
//!
//! Why two pools: the EA-pool limitations are about the SEARCH's
//! sanity (don't burn rollouts on no-op casts), NOT about whether
//! a human is allowed to play the card. Conflating the two meant
//! any deck the human built was secretly gated by EA-search heuristics.
//!
//! Filter rules:
//!
//! `deckbuilder_pool`:
//!   1. `c.kind.is_castable()` — has a routing path in `play_card`.
//!   2. `!c.is_variant` — variants are a balance-probe schema thing.
//!   3. Cost source is one the engine can pay.
//!
//! `playable_pool` (EA-only, adds to deckbuilder_pool):
//!   4. No `"test"` subtype.
//!   5. No `"debug"` subtype (chaos-engineering cards aren't EA genes).
//!   6. X-cost spells without `on_play` are filtered (search-burn guard).

use crate::card::{Card, CardType, CostSource, EventName};
use crate::cast_routing::CastRouting;

/// Engine-routable, human-deckbuilder-eligible. Strictly larger than
/// [`playable_pool`]. Use this for the deckbuilder card pool, preset
/// validation, and anywhere a human can deliberately include a card
/// (even one the EA wouldn't pick).
pub fn deckbuilder_pool(cards: &[Card]) -> Vec<Card> {
    cards
        .iter()
        .filter(|c| c.kind.is_castable())
        .filter(|c| !c.is_variant)
        .filter(|c| {
            c.cost.iter().all(|cc| {
                matches!(
                    cc.source,
                    CostSource::Hand
                        | CostSource::Mill
                        | CostSource::Graveyard
                        | CostSource::Sacrifice
                        | CostSource::Attached
                        | CostSource::SelfExile
                )
            })
        })
        .cloned()
        .collect()
}

/// EA / genetic-algorithm candidate set. Subset of
/// [`deckbuilder_pool`] with additional sanity-of-search filters
/// applied. Use this for the EA mainline (`tsot evolve`, the search
/// tree's expand-step, gauntlet building). NOT for deckbuilder UIs
/// or preset validation — see [`deckbuilder_pool`].
pub fn playable_pool(cards: &[Card]) -> Vec<Card> {
    deckbuilder_pool(cards)
        .into_iter()
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("debug")))
        .filter(|c| {
            let has_x = c.cost.iter().any(|cc| cc.is_x);
            let is_spell = matches!(c.kind, CardType::Spell);
            let has_play_handler = c
                .handlers
                .keys()
                .any(|e| matches!(e, EventName::OnPlay));
            !(has_x && is_spell && !has_play_handler)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardRegistry;

    fn registry() -> CardRegistry {
        CardRegistry::load(std::path::Path::new("cards")).expect("registry loads")
    }

    #[test]
    fn playable_pool_is_non_empty() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        assert!(!pool.is_empty(), "playable pool should not be empty");
    }

    #[test]
    fn playable_pool_excludes_variants() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        let n_variants = pool.iter().filter(|c| c.is_variant).count();
        assert_eq!(n_variants, 0, "playable pool must exclude is_variant cards");
    }

    #[test]
    fn playable_pool_excludes_test_subtype() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        let n_tests = pool
            .iter()
            .filter(|c| c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
            .count();
        assert_eq!(n_tests, 0, "playable pool must exclude test-subtype cards");
    }

    #[test]
    fn playable_pool_excludes_uncastable_kinds() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        let n_uncastable = pool.iter().filter(|c| !c.kind.is_castable()).count();
        assert_eq!(n_uncastable, 0, "every card in the pool must be is_castable");
    }

    #[test]
    fn playable_pool_excludes_unsupported_cost_sources() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        for c in &pool {
            for cc in &c.cost {
                assert!(
                    matches!(
                        cc.source,
                        CostSource::Hand
                            | CostSource::Mill
                            | CostSource::Graveyard
                            | CostSource::Sacrifice
                            | CostSource::Attached
                            | CostSource::SelfExile
                    ),
                    "card {} has unsupported cost source {:?}",
                    c.id,
                    cc.source
                );
            }
        }
    }

    #[test]
    fn playable_pool_excludes_x_spell_without_on_play() {
        let reg = registry();
        let pool = playable_pool(reg.cards());
        for c in &pool {
            let has_x = c.cost.iter().any(|cc| cc.is_x);
            let is_spell = matches!(c.kind, CardType::Spell);
            let has_play_handler =
                c.handlers.keys().any(|e| matches!(e, EventName::OnPlay));
            assert!(
                !(has_x && is_spell && !has_play_handler),
                "card {} is an X-cost spell with no on_play handler — should be filtered",
                c.id
            );
        }
    }

    #[test]
    fn playable_pool_includes_blue_monkey() {
        // Smoke check: a well-known card every test deck relies on
        // should survive every filter.
        let reg = registry();
        let pool = playable_pool(reg.cards());
        assert!(
            pool.iter().any(|c| c.id == "blue-monkey"),
            "blue-monkey should be in the playable pool"
        );
    }
}
