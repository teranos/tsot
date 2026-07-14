//! Cardless-sleeve (Z.8) deck representation + round-trips — slice 8.1.
//!
//! A deck can contain cardless-sleeve units (S.4). This proves they can be
//! built into a starting deck (`from_units`) and that they survive both
//! round-trips: SaveFile (serialize current state) and ReplayFile (rebuild
//! initial state from deck ids, via a cardless sentinel).

use tsot::card::CardRegistry;
use tsot::game::{DeckUnit, GameState, Journal};
use tsot::replay::{ReplayFile, SaveFile, CARDLESS_SLEEVE_ID};

/// A 50-unit deck: 5 real cards (dealt to hand at S.1), then one cardless
/// sleeve on top of the DECK, then real cards. Returns (units, real_id).
fn deck_with_cardless_on_top(registry: &CardRegistry) -> Vec<DeckUnit> {
    let card = registry.cards()[0].clone();
    let mut units: Vec<DeckUnit> = (0..5).map(|_| DeckUnit::Card(card.clone())).collect();
    units.push(DeckUnit::Cardless);
    for _ in 0..44 {
        units.push(DeckUnit::Card(card.clone()));
    }
    units
}

fn plain_deck(registry: &CardRegistry) -> Vec<DeckUnit> {
    let card = registry.cards()[0].clone();
    (0..50).map(|_| DeckUnit::Card(card.clone())).collect()
}

#[test]
fn from_units_places_a_cardless_sleeve_on_top_of_the_deck() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let s = GameState::from_units(deck_with_cardless_on_top(&registry), plain_deck(&registry));

    // Units 0..5 are dealt to HAND (S.1); unit 5 (the cardless one) is the
    // top of the DECK.
    let top = s.a.deck[0].clone();
    assert!(
        s.card_pool.get(&top).unwrap().is_cardless(),
        "cardless sleeve sits on top of the deck"
    );
    assert!(
        s.a.hand.iter().all(|iid| !s.card_pool.get(iid).unwrap().is_cardless()),
        "the 5 dealt cards are all real"
    );
}

#[test]
fn cardless_sleeve_survives_save_load() {
    // rebind_handlers must skip cardless sleeves (their blank card has id
    // "", which is not in the registry).
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let s = GameState::from_units(deck_with_cardless_on_top(&registry), plain_deck(&registry));
    let cardless_iid = s.a.deck[0].clone();
    assert!(s.card_pool.get(&cardless_iid).unwrap().is_cardless());

    let json = SaveFile::from_state(&s, 0).to_json().unwrap();
    let restored = SaveFile::from_json(&json).unwrap().restore(&registry).unwrap();

    assert!(
        restored.card_pool.get(&cardless_iid).unwrap().is_cardless(),
        "cardless sleeve survives save/load"
    );
}

#[test]
fn replay_rebuild_reconstructs_cardless_units_from_the_sentinel() {
    let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
    let real = registry.cards()[0].id.clone();

    // Deck ids with the cardless sentinel at position 5 (top of DECK).
    let mut ids_a: Vec<String> = vec![real.clone(); 5];
    ids_a.push(CARDLESS_SLEEVE_ID.to_string());
    ids_a.extend(vec![real.clone(); 44]);
    let ids_b = vec![real; 50];

    let replay = ReplayFile {
        seed: 0,
        deck_a_card_ids: ids_a,
        deck_b_card_ids: ids_b,
        journal: Journal::new(),
    };
    let state = replay.rebuild_initial_state(&registry).unwrap();

    let top = state.a.deck[0].clone();
    assert!(
        state.card_pool.get(&top).unwrap().is_cardless(),
        "the sentinel rebuilds a cardless sleeve at the top of the deck"
    );
}
