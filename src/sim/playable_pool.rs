//! Shared filter for the set of cards humans, the EA, and the wasm
//! deckbuilder all consider "playable from a deck."
//!
//! Extracted from `main.rs` so the wasm FFI's `tsot_list_card_pool`
//! and the deckbuilder UI see the same set the CLI tools see. Filter
//! mirrors the original main.rs inline computation byte-for-byte:
//!
//! 1. `c.kind.is_castable()` — has a routing path in `play_card`.
//! 2. `!c.is_variant` — balance-probe variants don't belong in the
//!    main pool; `tsot balance-probe` picks them up separately.
//! 3. No `"test"` subtype.
//! 4. Every cost component's source is one the engine knows how to
//!    pay (HAND / MILL / GRAVEYARD / SACRIFICE / ATTACHED / SELFEXILE).
//! 5. X-cost spells without an `on_play` handler are filtered out —
//!    casting them pays the cost but resolves to a no-op, which the
//!    EA / search would waste budget exploring.

use crate::card::{Card, CardType, CostSource, EventName};
use crate::cast_routing::CastRouting;

/// Filter the registry's full card list down to the set that can
/// appear in a constructed deck. Stable ordering: matches the input
/// slice's order.
pub fn playable_pool(cards: &[Card]) -> Vec<Card> {
    cards
        .iter()
        .filter(|c| c.kind.is_castable())
        .filter(|c| !c.is_variant)
        .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("test")))
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
        .filter(|c| {
            let has_x = c.cost.iter().any(|cc| cc.is_x);
            let is_spell = matches!(c.kind, CardType::Spell);
            let has_play_handler = c
                .handlers
                .keys()
                .any(|e| matches!(e, EventName::OnPlay));
            !(has_x && is_spell && !has_play_handler)
        })
        .cloned()
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
