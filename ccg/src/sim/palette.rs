//! Color/symbol palette constraint for **probe** deck construction.
//!
//! `make evolve` draws genomes uniformly at random over the whole
//! playable pool. That produces incoherent 5-color soup, which is fine
//! as a raw baseline but a poor lens for "how good is *this* card" — you
//! want the EA to build a deck in the card's own identity.
//!
//! This module constrains construction to a **palette** anchored on the
//! probed card:
//!
//!   - allowed colors = the anchor card's colors, plus **at most one**
//!     more color (a single splash), and **never more than 4** colors
//!     total;
//!   - allowed symbols = the anchor card's symbols, plus **at most
//!     three** more symbols;
//!   - both expansions may apply at once (a deck can splash a color *and*
//!     pick up new symbols);
//!   - **colorless cards are always eligible** and never consume the
//!     color budget; a card with no symbols is likewise always
//!     symbol-eligible (the empty set is a subset of any palette).
//!
//! The constraint is expressed as an invariant ([`palette_ok`]) plus a
//! repair ([`enforce_palette`]) applied at the same points the EA
//! re-enforces the pin — init and after each crossover/mutate. The whole
//! thing is gated behind `EvolveConfig::palette_anchor: Option<_>`, so
//! `make evolve` (anchor `None`) is byte-for-byte unchanged: probe-only.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;

use crate::card::Card;
use crate::replay::CARDLESS_SLEEVE_ID;

/// RULES: a deck may never carry more than 4 colors.
pub const MAX_COLORS: usize = 4;
/// Splash budget: at most one color beyond the anchor's colors.
pub const EXTRA_COLORS: usize = 1;
/// Splash budget: at most three symbols beyond the anchor's symbols.
pub const EXTRA_SYMBOLS: usize = 3;

/// The colors + symbols a probe deck is built around — taken from the
/// probed card. Colors are stored lowercased for case-insensitive
/// comparison; symbols are the glyphs verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteAnchor {
    pub colors: BTreeSet<String>,
    pub symbols: BTreeSet<String>,
}

impl PaletteAnchor {
    pub fn from_card(card: &Card) -> Self {
        PaletteAnchor {
            colors: card.colors.iter().map(|c| c.to_ascii_lowercase()).collect(),
            symbols: card.symbols.iter().cloned().collect(),
        }
    }
}

fn card_map(pool: &[Card]) -> HashMap<&str, &Card> {
    pool.iter().map(|c| (c.id.as_str(), c)).collect()
}

/// The distinct colors (lowercased) and symbols a genome actually uses.
/// Cardless sleeves and ids not present in `pool` contribute nothing.
fn used_colors_symbols(
    genome: &[String],
    map: &HashMap<&str, &Card>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut colors = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    for id in genome {
        if id == CARDLESS_SLEEVE_ID {
            continue;
        }
        if let Some(card) = map.get(id.as_str()) {
            for c in &card.colors {
                colors.insert(c.to_ascii_lowercase());
            }
            for s in &card.symbols {
                symbols.insert(s.clone());
            }
        }
    }
    (colors, symbols)
}

/// Does `genome` satisfy the palette invariant for `anchor`? The oracle
/// the repair restores and the tests assert against:
///
///   - at most [`EXTRA_COLORS`] colors beyond the anchor's,
///   - never more than [`MAX_COLORS`] colors total,
///   - at most [`EXTRA_SYMBOLS`] symbols beyond the anchor's.
pub fn palette_ok(genome: &[String], pool: &[Card], anchor: &PaletteAnchor) -> bool {
    let map = card_map(pool);
    let (colors_used, symbols_used) = used_colors_symbols(genome, &map);
    let extra_colors = colors_used.difference(&anchor.colors).count();
    let extra_symbols = symbols_used.difference(&anchor.symbols).count();
    extra_colors <= EXTRA_COLORS
        && colors_used.len() <= MAX_COLORS
        && extra_symbols <= EXTRA_SYMBOLS
}

/// Repair `genome` into `anchor`'s palette in place, preserving its
/// length. No-op when it already satisfies [`palette_ok`].
///
/// Which single splash color and which (up to three) splash symbols to
/// keep are chosen from what the genome already contains, shuffled by
/// `rng` — so different genomes settle on different splashes and the EA
/// still explores. Off-palette cards are dropped and the freed slots
/// refilled by a uniform draw over the eligible pool (per-card cap
/// respected), falling back to cardless sleeves if the eligible pool
/// can't fill the deck.
pub fn enforce_palette(
    genome: &mut Vec<String>,
    pool: &[Card],
    anchor: &PaletteAnchor,
    per_card_cap: u32,
    rng: &mut StdRng,
) {
    if palette_ok(genome, pool, anchor) {
        return;
    }
    let target = genome.len();
    let map = card_map(pool);
    let (colors_used, symbols_used) = used_colors_symbols(genome, &map);

    // Pick the splash to keep from what the genome already carries,
    // shuffled so different genomes settle on different splashes.
    let color_room = MAX_COLORS
        .saturating_sub(anchor.colors.len())
        .min(EXTRA_COLORS);
    let mut splash_colors: Vec<String> =
        colors_used.difference(&anchor.colors).cloned().collect();
    splash_colors.shuffle(rng);
    let mut allowed_colors = anchor.colors.clone();
    allowed_colors.extend(splash_colors.into_iter().take(color_room));

    let mut splash_symbols: Vec<String> =
        symbols_used.difference(&anchor.symbols).cloned().collect();
    splash_symbols.shuffle(rng);
    let mut allowed_symbols = anchor.symbols.clone();
    allowed_symbols.extend(splash_symbols.into_iter().take(EXTRA_SYMBOLS));

    let eligible = |card: &Card| -> bool {
        card.colors
            .iter()
            .all(|c| allowed_colors.contains(&c.to_ascii_lowercase()))
            && card.symbols.iter().all(|s| allowed_symbols.contains(s))
    };

    // Keep every in-palette card (and cardless sleeves), recounting so
    // the refill honors the per-card cap across kept + drawn.
    let mut copies: BTreeMap<String, u32> = BTreeMap::new();
    let mut kept: Vec<String> = Vec::with_capacity(target);
    for id in genome.iter() {
        if id == CARDLESS_SLEEVE_ID {
            kept.push(id.clone());
            continue;
        }
        if let Some(card) = map.get(id.as_str()) {
            if eligible(card) {
                let n = copies.entry(id.clone()).or_insert(0);
                if (*n as usize) < per_card_cap as usize {
                    *n += 1;
                    kept.push(id.clone());
                }
            }
        }
    }

    // Refill the freed slots with a uniform draw over the eligible pool,
    // cap-respecting; fall back to cardless sleeves if the eligible pool
    // is exhausted, so the deck never comes back short.
    let eligible_ids: Vec<&str> = pool
        .iter()
        .filter(|c| eligible(c))
        .map(|c| c.id.as_str())
        .collect();
    while kept.len() < target {
        let candidates: Vec<&str> = eligible_ids
            .iter()
            .copied()
            .filter(|id| (*copies.get(*id).unwrap_or(&0) as usize) < per_card_cap as usize)
            .collect();
        match candidates.choose(rng) {
            Some(pick) => {
                *copies.entry((*pick).to_string()).or_insert(0) += 1;
                kept.push((*pick).to_string());
            }
            None => kept.push(CARDLESS_SLEEVE_ID.to_string()),
        }
    }

    *genome = kept;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardRegistry;
    use rand::SeedableRng;
    use std::path::Path;

    fn registry() -> CardRegistry {
        CardRegistry::load(Path::new("cards")).expect("load cards/")
    }

    fn anchor_for(reg: &CardRegistry, id: &str) -> PaletteAnchor {
        PaletteAnchor::from_card(reg.get(id).unwrap_or_else(|| panic!("card {id} missing")))
    }

    #[test]
    fn anchor_from_ankle_scorcher_is_mono_red_no_symbols() {
        let reg = registry();
        let a = anchor_for(&reg, "ankle-scorcher");
        assert_eq!(a.colors, BTreeSet::from(["red".to_string()]));
        assert!(a.symbols.is_empty(), "ankle-scorcher declares no symbols");
    }

    #[test]
    fn palette_ok_rejects_five_color_soup_under_mono_red_anchor() {
        // A genome spanning the whole corpus almost certainly carries
        // more than one non-red color — the exact incoherence we want to
        // forbid.
        let reg = registry();
        let anchor = anchor_for(&reg, "ankle-scorcher");
        let pool: Vec<Card> = reg.cards().to_vec();
        let soup: Vec<String> = pool.iter().take(50).map(|c| c.id.clone()).collect();
        assert!(
            !palette_ok(&soup, &pool, &anchor),
            "a 50-card whole-corpus slice should violate a mono-red palette"
        );
    }

    #[test]
    fn enforce_palette_repairs_violating_genome() {
        let reg = registry();
        let anchor = anchor_for(&reg, "ankle-scorcher");
        let pool: Vec<Card> = reg.cards().to_vec();
        let mut genome: Vec<String> = pool.iter().take(50).map(|c| c.id.clone()).collect();
        assert_eq!(genome.len(), 50);
        let mut rng = StdRng::seed_from_u64(0xA11E_77E);
        enforce_palette(&mut genome, &pool, &anchor, 3, &mut rng);
        assert_eq!(genome.len(), 50, "repair preserves deck length");
        assert!(
            palette_ok(&genome, &pool, &anchor),
            "repaired genome must satisfy the palette invariant"
        );
    }

    #[test]
    fn enforce_palette_respects_per_card_cap() {
        let reg = registry();
        let anchor = anchor_for(&reg, "ankle-scorcher");
        let pool: Vec<Card> = reg.cards().to_vec();
        let mut genome: Vec<String> = pool.iter().take(50).map(|c| c.id.clone()).collect();
        let mut rng = StdRng::seed_from_u64(7);
        enforce_palette(&mut genome, &pool, &anchor, 3, &mut rng);
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for id in &genome {
            if id == CARDLESS_SLEEVE_ID {
                continue;
            }
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
        for (id, n) in &counts {
            assert!(*n <= 3, "card {id} appears {n}× — exceeds cap 3");
        }
    }

    #[test]
    fn enforce_palette_is_noop_when_already_ok() {
        // A deck of only the anchor card is trivially in-palette; repair
        // must leave it untouched.
        let reg = registry();
        let anchor = anchor_for(&reg, "ankle-scorcher");
        let pool: Vec<Card> = reg.cards().to_vec();
        let mut genome: Vec<String> = vec!["ankle-scorcher".to_string(); 50];
        let before = genome.clone();
        let mut rng = StdRng::seed_from_u64(3);
        enforce_palette(&mut genome, &pool, &anchor, 3, &mut rng);
        assert_eq!(genome, before, "already-in-palette genome is unchanged");
    }

    #[test]
    fn enforce_palette_is_deterministic_per_seed() {
        let reg = registry();
        let anchor = anchor_for(&reg, "ankle-scorcher");
        let pool: Vec<Card> = reg.cards().to_vec();
        let base: Vec<String> = pool.iter().take(50).map(|c| c.id.clone()).collect();
        let mut g1 = base.clone();
        let mut g2 = base.clone();
        let mut r1 = StdRng::seed_from_u64(99);
        let mut r2 = StdRng::seed_from_u64(99);
        enforce_palette(&mut g1, &pool, &anchor, 3, &mut r1);
        enforce_palette(&mut g2, &pool, &anchor, 3, &mut r2);
        assert_eq!(g1, g2, "same seed → same repair");
    }
}
