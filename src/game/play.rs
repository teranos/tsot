//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId};
use crate::card::{CardType, CostSource, EventName};
use mlua::Lua;
use std::collections::HashSet;

/// Player-supplied choices when playing a card.
/// In this slice, only HAND payments require choice (which cards to spend).
/// MILL payments are deterministic (top N of DECK).
#[derive(Debug, Clone, Default)]
pub struct PlayChoices {
    /// One InstanceId per HAND cost-card the player chooses to spend.
    pub hand_payment_ids: Vec<InstanceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayError {
    GameOver,
    NotInHand,
    /// This slice only supports playing CREATURE cards.
    UnsupportedType(CardType),
    /// This slice only supports HAND and MILL cost sources.
    UnsupportedCostSource(CostSource),
    /// This slice doesn't support variable-X costs yet.
    VariableXNotSupported,
    /// HAND payment count must equal the card's total HAND cost.
    WrongHandPaymentCount { expected: usize, got: usize },
    /// A chosen HAND payment isn't in the player's hand, or is the card being played itself.
    HandPaymentInvalid(InstanceId),
    /// A HAND payment ID appears more than once in the choices.
    DuplicateHandPayment(InstanceId),
    /// DECK doesn't have enough cards to pay the MILL cost.
    InsufficientDeckForMill { needed: usize, have: usize },
}

impl GameState {
    /// Play `instance` from `player`'s HAND, paying its cost via `choices`.
    ///
    /// Atomic: returns Err and leaves state unchanged if any validation fails.
    /// Mutations only occur after all checks pass.
    ///
    /// Restrictions in this slice:
    ///   - Only CREATURE cards.
    ///   - Only `HAND` and `MILL` cost sources.
    ///   - No variable X (`is_x: true` returns Err).
    ///   - No timing checks (caller is responsible for C.6 / C.10 / U.7 / U.8).
    ///
    /// On success:
    ///   - MILL cost paid (top N of player's DECK → GRAVEYARD, per P.11).
    ///   - HAND payments removed from hand.
    ///   - Played card moved from HAND to BOARD (per P.2).
    ///   - HAND payments attached to the played card (per P.6), face-down (per P.17).
    pub fn play_card(
        &mut self,
        player: PlayerId,
        instance: &InstanceId,
        choices: PlayChoices,
        lua: Option<&Lua>,
    ) -> Result<(), PlayError> {
        if self.winner.is_some() {
            return Err(PlayError::GameOver);
        }

        if !self.player(player).hand.contains(instance) {
            return Err(PlayError::NotInHand);
        }

        // Snapshot card data so the borrow on card_pool can be dropped.
        let inst_ref = self.card_pool.get(instance).ok_or(PlayError::NotInHand)?;
        let card_kind = inst_ref.card.kind;
        let card_cost = inst_ref.card.cost.clone();

        if !matches!(card_kind, CardType::Creature) {
            // TODO(types): handle Instant (→ GRAVEYARD per P.1, timing per C.6),
            // Spell (→ GRAVEYARD, timing per C.10), Artifact (→ BOARD per P.19),
            // Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }

        // Aggregate cost requirements per source.
        let mut hand_needed: usize = 0;
        let mut mill_needed: usize = 0;
        for c in &card_cost {
            if c.is_x {
                return Err(PlayError::VariableXNotSupported);
            }
            let amount = c.amount.max(0) as usize;
            match c.source {
                CostSource::Hand => hand_needed += amount,
                CostSource::Mill => mill_needed += amount,
                // TODO(costs): support GRAVEYARD (P.12), SACRIFICE (P.16),
                // and SELF (P.5). Variable X (`is_x`) also belongs here.
                other => return Err(PlayError::UnsupportedCostSource(other)),
            }
        }

        if choices.hand_payment_ids.len() != hand_needed {
            return Err(PlayError::WrongHandPaymentCount {
                expected: hand_needed,
                got: choices.hand_payment_ids.len(),
            });
        }

        let mut seen: HashSet<&InstanceId> = HashSet::new();
        for hid in &choices.hand_payment_ids {
            if !seen.insert(hid) {
                return Err(PlayError::DuplicateHandPayment(hid.clone()));
            }
            if hid == instance {
                return Err(PlayError::HandPaymentInvalid(hid.clone()));
            }
            if !self.player(player).hand.contains(hid) {
                return Err(PlayError::HandPaymentInvalid(hid.clone()));
            }
        }

        let deck_have = self.player(player).deck.len();
        if deck_have < mill_needed {
            return Err(PlayError::InsufficientDeckForMill {
                needed: mill_needed,
                have: deck_have,
            });
        }

        // LUA Phase 1: on_play fires after validation, before mutations.
        // The played card is still in HAND when the handler runs.
        if let Some(lua) = lua {
            lua_api::fire_self_only(lua, self, EventName::OnPlay, instance);
        }

        // All checks pass — apply mutations.
        let pm = self.player_mut(player);

        for _ in 0..mill_needed {
            let top = pm.deck.remove(0);
            pm.graveyard.push(top);
        }

        let pos = pm.hand.iter().position(|x| x == instance).unwrap();
        pm.hand.remove(pos);

        for hid in &choices.hand_payment_ids {
            let pos = pm.hand.iter().position(|x| x == hid).unwrap();
            pm.hand.remove(pos);
        }

        pm.board.push(instance.clone());

        let inst_mut = self.card_pool.get_mut(instance).unwrap();
        inst_mut.summoning_sick = true; // B.3
        for hid in &choices.hand_payment_ids {
            inst_mut.attached.push(hid.clone());
        }

        for hid in &choices.hand_payment_ids {
            if let Some(a) = self.card_pool.get_mut(hid) {
                a.face_down = true;
            }
        }

        // LUA Phase 1: on_enter_board fires after the card is on BOARD and
        // attachments are wired (so handlers see self.attached correctly).
        if let Some(lua) = lua {
            lua_api::fire_self_only(lua, self, EventName::OnEnterBoard, instance);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CostComponent;
    use crate::game::test_helpers::*;

    #[test]
    fn play_creature_with_no_cost_moves_to_board() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        assert!(s
            .play_card(PlayerId::A, &iid, PlayChoices::default(), None)
            .is_ok());
        assert!(!s.a.hand.contains(&iid));
        assert!(s.a.board.contains(&iid));
    }

    #[test]
    fn play_creature_with_hand_cost_attaches_payments() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        let payment = s.a.hand[1].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
            }],
        );
        let choices = PlayChoices {
            hand_payment_ids: vec![payment.clone()],
        };
        assert!(s
            .play_card(PlayerId::A, &creature, choices, None)
            .is_ok());
        assert!(s.a.board.contains(&creature));
        assert!(!s.a.hand.contains(&creature));
        assert!(!s.a.hand.contains(&payment));
        let inst = s.card_pool.get(&creature).unwrap();
        assert!(inst.attached.contains(&payment));
        assert!(s.card_pool.get(&payment).unwrap().face_down);
    }

    #[test]
    fn play_creature_with_mill_cost_mills_top_of_deck() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        let top_three: Vec<_> = s.a.deck.iter().take(3).cloned().collect();
        let deck_before = s.a.deck.len();
        let graveyard_before = s.a.graveyard.len();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 3,
                source: CostSource::Mill,
                is_x: false,
            }],
        );
        assert!(s
            .play_card(PlayerId::A, &creature, PlayChoices::default(), None)
            .is_ok());
        assert_eq!(s.a.deck.len(), deck_before - 3);
        assert_eq!(s.a.graveyard.len(), graveyard_before + 3);
        for tid in &top_three {
            assert!(s.a.graveyard.contains(tid));
        }
        assert!(s.a.board.contains(&creature));
    }

    #[test]
    fn play_combined_hand_and_mill_cost() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        let pay = s.a.hand[1].clone();
        set_cost(
            &mut s,
            &creature,
            vec![
                CostComponent {
                    amount: 1,
                    source: CostSource::Hand,
                    is_x: false,
                },
                CostComponent {
                    amount: 2,
                    source: CostSource::Mill,
                    is_x: false,
                },
            ],
        );
        let result = s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices {
                hand_payment_ids: vec![pay.clone()],
            },
            None,
        );
        assert!(result.is_ok());
        assert!(s.a.board.contains(&creature));
        assert!(s.card_pool.get(&creature).unwrap().attached.contains(&pay));
        assert_eq!(s.a.deck.len(), 43);
    }

    #[test]
    fn play_card_errors_when_not_in_hand() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let in_deck = s.a.deck[0].clone();
        assert_eq!(
            s.play_card(PlayerId::A, &in_deck, PlayChoices::default(), None),
            Err(PlayError::NotInHand)
        );
    }

    #[test]
    fn play_card_errors_when_hand_payment_count_wrong() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        let pay = s.a.hand[1].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 2,
                source: CostSource::Hand,
                is_x: false,
            }],
        );
        let result = s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices {
                hand_payment_ids: vec![pay],
            },
            None,
        );
        assert_eq!(
            result,
            Err(PlayError::WrongHandPaymentCount {
                expected: 2,
                got: 1
            })
        );
    }

    #[test]
    fn play_card_errors_when_paying_with_self() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 1,
                source: CostSource::Hand,
                is_x: false,
            }],
        );
        let result = s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices {
                hand_payment_ids: vec![creature.clone()],
            },
            None,
        );
        assert_eq!(result, Err(PlayError::HandPaymentInvalid(creature)));
    }

    #[test]
    fn play_card_errors_on_duplicate_hand_payment() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        let pay = s.a.hand[1].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 2,
                source: CostSource::Hand,
                is_x: false,
            }],
        );
        let result = s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices {
                hand_payment_ids: vec![pay.clone(), pay.clone()],
            },
            None,
        );
        assert_eq!(result, Err(PlayError::DuplicateHandPayment(pay)));
    }

    #[test]
    fn play_card_errors_when_insufficient_deck_for_mill() {
        let mut s = GameState::new(deck_of(10, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 100,
                source: CostSource::Mill,
                is_x: false,
            }],
        );
        let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
        assert_eq!(
            result,
            Err(PlayError::InsufficientDeckForMill {
                needed: 100,
                have: 5,
            })
        );
    }

    #[test]
    fn play_card_errors_on_unsupported_type() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let iid = s.a.hand[0].clone();
        s.card_pool.get_mut(&iid).unwrap().card.kind = CardType::Instant;
        assert_eq!(
            s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
            Err(PlayError::UnsupportedType(CardType::Instant))
        );
    }

    #[test]
    fn play_card_errors_on_variable_x_cost() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 0,
                source: CostSource::Hand,
                is_x: true,
            }],
        );
        let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
        assert_eq!(result, Err(PlayError::VariableXNotSupported));
    }

    #[test]
    fn play_card_errors_on_unsupported_cost_source() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 1,
                source: CostSource::Graveyard,
                is_x: false,
            }],
        );
        let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
        assert_eq!(
            result,
            Err(PlayError::UnsupportedCostSource(CostSource::Graveyard))
        );
    }

    #[test]
    fn play_card_leaves_state_unchanged_on_error() {
        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        set_cost(
            &mut s,
            &creature,
            vec![CostComponent {
                amount: 100,
                source: CostSource::Mill,
                is_x: false,
            }],
        );
        let hand_before = s.a.hand.clone();
        let deck_before = s.a.deck.clone();
        let board_before = s.a.board.clone();
        let graveyard_before = s.a.graveyard.clone();
        let _ = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
        assert_eq!(s.a.hand, hand_before);
        assert_eq!(s.a.deck, deck_before);
        assert_eq!(s.a.board, board_before);
        assert_eq!(s.a.graveyard, graveyard_before);
    }

    fn registry_with_fixture(name: &str, source: &str) -> crate::card::CardRegistry {
        let tmp = std::env::temp_dir().join(format!("tsot_fixture_{name}"));
        std::fs::create_dir_all(&tmp).unwrap();
        if let Ok(rd) = std::fs::read_dir(&tmp) {
            for entry in rd.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let path = tmp.join(format!("{name}.lua"));
        std::fs::write(&path, source).unwrap();
        crate::card::CardRegistry::load(&tmp).unwrap()
    }

    #[test]
    fn on_play_handler_fires_before_card_leaves_hand() {
        let registry = registry_with_fixture(
            "on_play",
            r#"return {
                id = "fire-on-play",
                on_play = function(game, self)
                    -- Record whether the card is still in hand when we see it.
                    _G.fire_on_play_count = (_G.fire_on_play_count or 0) + 1
                end,
            }"#,
        );
        let fixture = registry
            .cards()
            .iter()
            .find(|c| c.id == "fire-on-play")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&creature).unwrap();
            inst.card.handlers = fixture.handlers.clone();
            inst.card.id = fixture.id.clone();
        }

        registry
            .lua()
            .globals()
            .set("fire_on_play_count", 0_i32)
            .unwrap();
        s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices::default(),
            Some(registry.lua()),
        )
        .unwrap();

        let count: i32 = registry.lua().globals().get("fire_on_play_count").unwrap();
        assert_eq!(count, 1);
        assert_eq!(s.event_fires[&crate::card::EventName::OnPlay], [1, 0]);
    }

    #[test]
    fn goblin_scribe_draws_a_card_on_enter_board() {
        let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let scribe = registry
            .cards()
            .iter()
            .find(|c| c.id == "goblin-scribe")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&creature).unwrap();
            inst.card.handlers = scribe.handlers.clone();
            inst.card.id = scribe.id.clone();
        }
        let hand_before = s.a.hand.len();
        let deck_before = s.a.deck.len();

        s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices::default(),
            Some(registry.lua()),
        )
        .unwrap();

        // Hand lost the played card and gained one drawn card → net zero.
        // (-1 for play, +1 for ETB draw.)
        assert_eq!(s.a.hand.len(), hand_before);
        assert_eq!(s.a.deck.len(), deck_before - 1);
        assert!(s.a.board.contains(&creature));
        assert_eq!(
            s.event_fires[&crate::card::EventName::OnEnterBoard],
            [1, 0]
        );
    }

    #[test]
    fn on_enter_board_handler_fires_after_card_on_board() {
        let registry = registry_with_fixture(
            "on_enter_board",
            r#"return {
                id = "fire-on-enter",
                on_enter_board = function(game, self)
                    _G.fire_on_enter_count = (_G.fire_on_enter_count or 0) + 1
                end,
            }"#,
        );
        let fixture = registry
            .cards()
            .iter()
            .find(|c| c.id == "fire-on-enter")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let creature = s.a.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&creature).unwrap();
            inst.card.handlers = fixture.handlers.clone();
            inst.card.id = fixture.id.clone();
        }

        registry
            .lua()
            .globals()
            .set("fire_on_enter_count", 0_i32)
            .unwrap();
        s.play_card(
            PlayerId::A,
            &creature,
            PlayChoices::default(),
            Some(registry.lua()),
        )
        .unwrap();

        let count: i32 = registry
            .lua()
            .globals()
            .get("fire_on_enter_count")
            .unwrap();
        assert_eq!(count, 1);
        assert!(s.a.board.contains(&creature));
        assert_eq!(
            s.event_fires[&crate::card::EventName::OnEnterBoard],
            [1, 0]
        );
    }
}
