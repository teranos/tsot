//! Deck-variant configuration: which cards make up each variant's pool,
//! which mandatory pre-fills land in every deck of that variant, and which
//! cards are exclusive to specific variants.

use rand::seq::SliceRandom;
use rand::Rng;
use tsot::card::Card;

/// Deck-build variants. Ra and Rb are full-pool baselines; the rest are
/// filtered pools meant to stress-test specific corpus interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeckVariant {
    /// Full pool, no filter.
    Ra,
    /// Full pool, no filter (identical to Ra; kept distinct so the matchup
    /// matrix shows the Ra↔Ra and Ra↔Rb baselines symmetrically).
    Rb,
    /// Humans tribe: no goblins. 2× modern-lcd-clock mandatory.
    Hu,
    /// Goblins tribe: filters out humans/fish/insects/beasts. Pre-fills
    /// 2× modern-lcd-clock plus 4 guaranteed goblins (eager-goblin +
    /// goblin-warlord). LCD Clock is exclusive to Hu and Go.
    Go,
    /// Colorless or blue only — heavy on draw / counter / interaction.
    Uu,
    /// Red or purple cards (must list at least one of those colors).
    Pr,
    /// Green or colorless only. Excludes purple, blue, red, black, white
    /// cards. 4× green-jewel mandatory.
    Gg,
}

pub const VARIANTS: [DeckVariant; 7] = [
    DeckVariant::Ra,
    DeckVariant::Rb,
    DeckVariant::Hu,
    DeckVariant::Go,
    DeckVariant::Uu,
    DeckVariant::Pr,
    DeckVariant::Gg,
];

pub fn variant_label(v: DeckVariant) -> &'static str {
    match v {
        DeckVariant::Ra => "ra",
        DeckVariant::Rb => "rb",
        DeckVariant::Hu => "hu",
        DeckVariant::Go => "go",
        DeckVariant::Uu => "uu",
        DeckVariant::Pr => "pr",
        DeckVariant::Gg => "gg",
    }
}

/// Cards that are exclusive to specific deck variants. Any card listed
/// here is filtered OUT of every variant NOT in its allow-list.
pub fn card_is_allowed_in(card_id: &str, v: DeckVariant) -> bool {
    match card_id {
        "modern-lcd-clock" => matches!(v, DeckVariant::Hu | DeckVariant::Go),
        "methylene-blue" => matches!(v, DeckVariant::Uu),
        _ => true,
    }
}

pub fn variant_pool(playable: &[Card], v: DeckVariant) -> Vec<Card> {
    let base: Vec<Card> = match v {
        DeckVariant::Ra | DeckVariant::Rb => playable.to_vec(),
        DeckVariant::Hu => playable
            .iter()
            .filter(|c| !c.subtypes.iter().any(|s| s.eq_ignore_ascii_case("goblin")))
            .cloned()
            .collect(),
        DeckVariant::Go => playable
            .iter()
            .filter(|c| {
                !c.subtypes.iter().any(|s| {
                    s.eq_ignore_ascii_case("human")
                        || s.eq_ignore_ascii_case("fish")
                        || s.eq_ignore_ascii_case("insect")
                        || s.eq_ignore_ascii_case("beast")
                })
            })
            .cloned()
            .collect(),
        DeckVariant::Uu => playable
            .iter()
            .filter(|c| {
                c.colors.is_empty()
                    || c.colors.iter().any(|col| col.eq_ignore_ascii_case("blue"))
            })
            .cloned()
            .collect(),
        DeckVariant::Pr => playable
            .iter()
            .filter(|c| {
                c.colors.iter().any(|col| {
                    col.eq_ignore_ascii_case("red") || col.eq_ignore_ascii_case("purple")
                })
            })
            .cloned()
            .collect(),
        DeckVariant::Gg => playable
            .iter()
            .filter(|c| {
                let banned = ["purple", "blue", "red", "black", "white"];
                !c.colors
                    .iter()
                    .any(|col| banned.iter().any(|b| col.eq_ignore_ascii_case(b)))
            })
            .cloned()
            .collect(),
    };
    base.into_iter()
        .filter(|c| card_is_allowed_in(&c.id, v))
        .collect()
}

/// Builds a deck of `size` cards from `pool`. Enforces RULES S.6: at most
/// 4 copies of any single card id. If the pool is too small to fill the
/// deck without exceeding the cap, the result has fewer than `size` cards.
///
/// `mandatory` is a list of `(card_id, count)` pre-fills: the deck starts
/// with exactly `count` copies of each id before random fill begins.
pub fn build_random_deck(
    pool: &[Card],
    rng: &mut impl Rng,
    size: usize,
    mandatory: &[(&str, u32)],
) -> Vec<Card> {
    use std::collections::BTreeMap;
    let mut copies: BTreeMap<String, u32> = BTreeMap::new();
    let mut deck: Vec<Card> = Vec::with_capacity(size);

    for (id, want) in mandatory {
        let want = (*want).min(4) as usize;
        if let Some(card) = pool.iter().find(|c| c.id == *id) {
            for _ in 0..want {
                if deck.len() >= size {
                    break;
                }
                *copies.entry(card.id.clone()).or_insert(0) += 1;
                deck.push(card.clone());
            }
        }
    }

    let mut attempts = 0;
    let max_attempts = size * 8 + 32;
    while deck.len() < size && attempts < max_attempts {
        attempts += 1;
        let Some(candidate) = pool.choose(rng) else {
            break;
        };
        let count = copies.entry(candidate.id.clone()).or_insert(0);
        if *count >= 4 {
            continue;
        }
        *count += 1;
        deck.push(candidate.clone());
    }
    deck.shuffle(rng);
    deck
}

/// Variant-specific mandatory pre-fills for deck construction. Mono-color
/// variants always run 4× their matching jewel; the tribal variants pre-
/// fill modern-lcd-clock and (for Go) a few guaranteed goblins.
pub fn mandatory_for_variant(v: DeckVariant) -> &'static [(&'static str, u32)] {
    match v {
        DeckVariant::Pr => &[("red-jewel", 4)],
        DeckVariant::Uu => &[("blue-jewel", 4), ("methylene-blue", 2)],
        DeckVariant::Gg => &[("green-jewel", 4)],
        DeckVariant::Hu => &[("modern-lcd-clock", 2)],
        DeckVariant::Go => &[
            ("modern-lcd-clock", 2),
            ("eager-goblin", 2),
            ("goblin-warlord", 2),
        ],
        _ => &[],
    }
}
