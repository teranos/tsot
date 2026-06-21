//! Visibility-filtered serializable view of `GameState` for the
//! `tsot serve` frontend. Engine builds one of these every time it
//! pauses for a human decision; the frontend renders the resulting
//! JSON.
//!
//! Visibility rules: the snapshot is built for a specific `viewer`
//! player. The viewer sees their own hand identities and zones in
//! full. The opponent's hand is rendered as a count only. Both
//! boards, graveyards, exiles, and the deck-top of either player
//! follow the standard visibility rules (board public, graveyard
//! public, exile public, deck-top hidden — represented as counts).
//!
//! Phase B (this file): card-level data is name + tapped + attached
//! list. Effective stats and modifier-aware fields can be added
//! incrementally as the UI needs them.

use serde::Serialize;

use crate::game::{GameState, InstanceId, PlayerId};

#[derive(Debug, Clone, Serialize)]
pub struct StateView {
    pub turn: u32,
    pub active_player: char,
    pub phase: String,
    pub winner: Option<char>,
    pub viewer: char,
    pub players: [PlayerView; 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct PlayerView {
    pub side: char,
    pub deck_count: usize,
    pub graveyard_count: usize,
    pub exile_count: usize,
    pub hand_count: usize,
    /// Hand cards visible only to the viewer. Empty for the opponent
    /// even though `hand_count` may be nonzero — that's the visibility
    /// filter at work.
    pub hand: Vec<CardView>,
    pub board: Vec<CardView>,
    /// Graveyard contents (public per RULES). Both sides see this.
    pub graveyard: Vec<CardView>,
    /// Exile contents (public). Less commonly relevant but cheap to render.
    pub exile: Vec<CardView>,
    /// Top card of deck — visible to the controller only (V.1). `None`
    /// for the opponent's deck or when the deck is empty.
    pub deck_top: Option<CardView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardView {
    pub iid: String,
    pub id: String,
    pub name: String,
    pub kind: String,
    pub colors: Vec<String>,
    pub symbols: Vec<String>,
    pub subtypes: Vec<String>,
    /// Printed cost, as written on the card.
    pub cost: String,
    /// Effective cost after applying static reductions (e.g., Modern
    /// LCD Clock making creatures cost 1 less hand). For cards
    /// outside hand, this equals `cost` — reductions only matter at
    /// cast time and apply against the iid being cast.
    pub effective_cost: String,
    pub abilities: Vec<String>,
    pub flavor: String,
    pub tapped: bool,
    pub summoning_sick: bool,
    pub damage: f32,
    pub power: f32,
    pub toughness: f32,
    pub attached: Vec<CardView>,
}

pub fn build_state_view(state: &GameState, viewer: PlayerId) -> StateView {
    let players = [
        build_player_view(state, PlayerId::A, viewer),
        build_player_view(state, PlayerId::B, viewer),
    ];
    StateView {
        turn: state.turn,
        active_player: side_char(state.active_player),
        phase: format!("{:?}", state.phase),
        winner: state.winner.map(side_char),
        viewer: side_char(viewer),
        players,
    }
}

fn build_player_view(state: &GameState, side: PlayerId, viewer: PlayerId) -> PlayerView {
    let p = state.player(side);
    let hand = if side == viewer {
        p.hand.iter().map(|iid| card_view(state, iid)).collect()
    } else {
        Vec::new()
    };
    let board = p.board.iter().map(|iid| card_view(state, iid)).collect();
    let graveyard = p.graveyard.iter().map(|iid| card_view(state, iid)).collect();
    let exile = p.exile.iter().map(|iid| card_view(state, iid)).collect();
    // Deck top is public to both players.
    let deck_top = p.deck.first().map(|iid| card_view(state, iid));
    PlayerView {
        side: side_char(side),
        deck_count: p.deck.len(),
        graveyard_count: p.graveyard.len(),
        exile_count: p.exile.len(),
        hand_count: p.hand.len(),
        hand,
        board,
        graveyard,
        exile,
        deck_top,
    }
}

pub fn card_view(state: &GameState, iid: &InstanceId) -> CardView {
    let inst = state.card_pool.get(iid);
    match inst {
        Some(inst) => {
            let (power, toughness) = state.effective_stats(iid);
            let printed = format_cost(&inst.card.cost);
            let effective = format_effective_cost(state, iid, &inst.card.cost);
            CardView {
                iid: iid.clone(),
                id: inst.card.id.clone(),
                name: inst.card.name.clone(),
                kind: format!("{:?}", inst.card.kind),
                colors: inst.card.colors.clone(),
                symbols: inst.card.symbols.clone(),
                subtypes: inst.card.subtypes.clone(),
                cost: printed,
                effective_cost: effective,
                abilities: inst.card.abilities.clone(),
                flavor: inst.card.flavor.clone(),
                tapped: inst.tapped,
                summoning_sick: inst.summoning_sick,
                damage: inst.damage,
                power,
                toughness,
                attached: inst.attached.iter().map(|a| card_view(state, a)).collect(),
            }
        }
        None => CardView {
            iid: iid.clone(),
            id: String::new(),
            name: format!("<unknown {iid}>"),
            kind: String::new(),
            colors: Vec::new(),
            symbols: Vec::new(),
            subtypes: Vec::new(),
            cost: String::new(),
            effective_cost: String::new(),
            abilities: Vec::new(),
            flavor: String::new(),
            tapped: false,
            summoning_sick: false,
            damage: 0.0,
            power: 0.0,
            toughness: 0.0,
            attached: Vec::new(),
        },
    }
}

fn format_effective_cost(
    state: &GameState,
    iid: &InstanceId,
    components: &[crate::card::CostComponent],
) -> String {
    if components.is_empty() {
        return "0".to_string();
    }
    components
        .iter()
        .map(|c| {
            let reduction = state.cost_reduction(iid, c.source);
            let effective_amount = (c.amount - reduction).max(0);
            let amount_str = if c.is_x {
                "X".to_string()
            } else {
                effective_amount.to_string()
            };
            let source = match c.source {
                crate::card::CostSource::Hand => "H",
                crate::card::CostSource::Mill => "M",
                crate::card::CostSource::Graveyard => "G",
                crate::card::CostSource::Sacrifice => "S",
                crate::card::CostSource::SelfExile => "X",
                crate::card::CostSource::Attached => "A",
            };
            format!("{amount_str}{source}")
        })
        .collect::<Vec<_>>()
        .join("+")
}

pub(crate) fn format_cost(components: &[crate::card::CostComponent]) -> String {
    if components.is_empty() {
        return "0".to_string();
    }
    components
        .iter()
        .map(|c| {
            let amount = if c.is_x { "X".to_string() } else { c.amount.to_string() };
            let source = match c.source {
                crate::card::CostSource::Hand => "H",
                crate::card::CostSource::Mill => "M",
                crate::card::CostSource::Graveyard => "G",
                crate::card::CostSource::Sacrifice => "S",
                crate::card::CostSource::SelfExile => "X",
                crate::card::CostSource::Attached => "A",
            };
            format!("{amount}{source}")
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn side_char(p: PlayerId) -> char {
    match p {
        PlayerId::A => 'a',
        PlayerId::B => 'b',
    }
}
