//! Playing a card from hand: cost payment, destination, attachment.
//!
//! Mirrors RULES.md P.1, P.2, P.6, P.7, P.11, P.17.

use super::context::EventContext;
use super::lua_api;
use super::state::{GameState, InstanceId, PlayerId};
use crate::card::{CardType, CostSource, EventName};
use std::collections::BTreeSet;

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
    /// This slice supports CREATURE and INSTANT card types.
    UnsupportedType(CardType),
    /// This slice supports HAND, MILL, and GRAVEYARD cost sources.
    UnsupportedCostSource(CostSource),
    /// GRAVEYARD doesn't have enough cards to pay the GRAVEYARD cost.
    InsufficientGraveyardForCost { needed: usize, have: usize },
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
        ctx: Option<&mut EventContext>,
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

        if !matches!(card_kind, CardType::Creature | CardType::Instant) {
            // TODO(types): handle Spell (→ GRAVEYARD per C.10), Artifact (→ BOARD per P.19),
            // Environment (→ BOARD per P.21 + P.22 slot management).
            return Err(PlayError::UnsupportedType(card_kind));
        }

        // Aggregate cost requirements per source.
        let mut hand_needed: usize = 0;
        let mut mill_needed: usize = 0;
        let mut graveyard_needed: usize = 0;
        for c in &card_cost {
            if c.is_x {
                return Err(PlayError::VariableXNotSupported);
            }
            let amount = c.amount.max(0) as usize;
            match c.source {
                CostSource::Hand => hand_needed += amount,
                CostSource::Mill => mill_needed += amount,
                CostSource::Graveyard => graveyard_needed += amount,
                // TODO(costs): support SACRIFICE (P.16) and SELF (P.5).
                other => return Err(PlayError::UnsupportedCostSource(other)),
            }
        }

        if choices.hand_payment_ids.len() != hand_needed {
            return Err(PlayError::WrongHandPaymentCount {
                expected: hand_needed,
                got: choices.hand_payment_ids.len(),
            });
        }

        let mut seen: BTreeSet<&InstanceId> = BTreeSet::new();
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

        let gy_have = self.player(player).graveyard.len();
        if gy_have < graveyard_needed {
            return Err(PlayError::InsufficientGraveyardForCost {
                needed: graveyard_needed,
                have: gy_have,
            });
        }

        // All checks pass — apply mutations.
        let pm = self.player_mut(player);

        // MILL cost: top N of DECK → GRAVEYARD (P.11).
        for _ in 0..mill_needed {
            let top = pm.deck.remove(0);
            pm.graveyard.push(top);
        }

        // GRAVEYARD cost (P.12): most-recent N → EXILE. Deterministic interpretation
        // pending choice API; uses the back of the graveyard vec (newest-first).
        for _ in 0..graveyard_needed {
            if let Some(gy_card) = pm.graveyard.pop() {
                pm.exile.push(gy_card);
            }
        }

        // Remove the played card and HAND payments from hand.
        let pos = pm.hand.iter().position(|x| x == instance).unwrap();
        pm.hand.remove(pos);
        for hid in &choices.hand_payment_ids {
            let pos = pm.hand.iter().position(|x| x == hid).unwrap();
            pm.hand.remove(pos);
        }

        // LUA Phase 1: on_play fires after cost is paid, before destination is set.
        // The card is in no zone at this moment — the handler is the resolution.
        let mut ctx = ctx;
        if let Some(c) = ctx.as_mut() {
            lua_api::fire_self_only(c.lua, self, c.oracle(), EventName::OnPlay, instance);
        }

        // Route to destination zone based on type.
        match card_kind {
            CardType::Creature => {
                let pm = self.player_mut(player);
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

                if let Some(c) = ctx.as_mut() {
                    lua_api::fire_self_only(
                        c.lua,
                        self,
                        c.oracle(),
                        EventName::OnEnterBoard,
                        instance,
                    );
                }
            }
            CardType::Instant => {
                // P.1: instants resolve and go to GRAVEYARD. HAND payments
                // don't attach to anything (no board host) — they also go
                // to GRAVEYARD as discarded cost cards.
                let pm = self.player_mut(player);
                pm.graveyard.push(instance.clone());
                for hid in &choices.hand_payment_ids {
                    pm.graveyard.push(hid.clone());
                }
            }
            _ => unreachable!("validated above"),
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
        s.card_pool.get_mut(&iid).unwrap().card.kind = CardType::Spell;
        assert_eq!(
            s.play_card(PlayerId::A, &iid, PlayChoices::default(), None),
            Err(PlayError::UnsupportedType(CardType::Spell))
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
                source: CostSource::Sacrifice,
                is_x: false,
            }],
        );
        let result = s.play_card(PlayerId::A, &creature, PlayChoices::default(), None);
        assert_eq!(
            result,
            Err(PlayError::UnsupportedCostSource(CostSource::Sacrifice))
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
    fn jellyfish_on_enter_board_bounces_chosen_creature_via_scripted_oracle() {
        use crate::card::CardRegistry;
        use crate::choice::{ScriptedAnswer, ScriptedOracle};
        use crate::game::EventContext;

        let registry = CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let jelly = registry
            .cards()
            .iter()
            .find(|c| c.id == "jellyfish")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let jelly_iid = s.a.hand[0].clone();
        let target_iid = s.b.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&jelly_iid).unwrap();
            inst.card = jelly.clone();
        }
        // Put an opposing creature on B's board to be the target.
        s.b.hand.retain(|x| x != &target_iid);
        s.b.board.push(target_iid.clone());

        // Seed graveyard for the 3-graveyard cost.
        let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
        s.a.graveyard.extend(gy_seeds.clone());

        let b_hand_before = s.b.hand.len();
        let b_board_before = s.b.board.len();

        // 1-hand cost requires a payment; pick any non-self hand card.
        let hand_payment = s
            .a
            .hand
            .iter()
            .find(|x| *x != &jelly_iid)
            .cloned()
            .unwrap();

        // Scripted oracle: pick target_iid.
        let mut oracle = ScriptedOracle::new(vec![ScriptedAnswer::Card(Some(target_iid.clone()))]);

        s.play_card(
            PlayerId::A,
            &jelly_iid,
            PlayChoices {
                hand_payment_ids: vec![hand_payment],
            },
            Some(&mut EventContext::new(registry.lua(), &mut oracle)),
        )
        .unwrap();

        // Target moved from B's board to B's hand.
        assert!(!s.b.board.contains(&target_iid));
        assert!(s.b.hand.contains(&target_iid));
        assert_eq!(s.b.hand.len(), b_hand_before + 1);
        assert_eq!(s.b.board.len(), b_board_before - 1);
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
            Some(&mut crate::game::EventContext::lua_only(registry.lua())),
        )
        .unwrap();

        let count: i32 = registry.lua().globals().get("fire_on_play_count").unwrap();
        assert_eq!(count, 1);
        assert_eq!(s.event_fires[&crate::card::EventName::OnPlay], [1, 0]);
    }

    #[test]
    fn surge_instant_untaps_all_your_creatures_on_play() {
        let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let surge = registry
            .cards()
            .iter()
            .find(|c| c.id == "surge")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let surge_iid = s.a.hand[0].clone();
        let cred1 = s.a.hand[1].clone();
        let cred2 = s.a.hand[2].clone();
        let b_creat = s.b.hand[0].clone();
        {
            let inst = s.card_pool.get_mut(&surge_iid).unwrap();
            inst.card = surge.clone();
        }
        // Put two tapped A creatures on board, plus one tapped B creature.
        s.a.hand.retain(|x| x != &cred1 && x != &cred2);
        s.a.board.push(cred1.clone());
        s.a.board.push(cred2.clone());
        s.card_pool.get_mut(&cred1).unwrap().tapped = true;
        s.card_pool.get_mut(&cred2).unwrap().tapped = true;
        s.b.hand.retain(|x| x != &b_creat);
        s.b.board.push(b_creat.clone());
        s.card_pool.get_mut(&b_creat).unwrap().tapped = true;

        let payment = s.a.hand.iter().find(|x| *x != &surge_iid).cloned().unwrap();
        let payment2 = s
            .a
            .hand
            .iter()
            .find(|x| *x != &surge_iid && **x != payment)
            .cloned()
            .unwrap();
        s.play_card(
            PlayerId::A,
            &surge_iid,
            PlayChoices {
                hand_payment_ids: vec![payment, payment2],
            },
            Some(&mut crate::game::EventContext::lua_only(registry.lua())),
        )
        .unwrap();

        // Both A creatures untapped.
        assert!(!s.card_pool.get(&cred1).unwrap().tapped);
        assert!(!s.card_pool.get(&cred2).unwrap().tapped);
        // B's creature unchanged.
        assert!(s.card_pool.get(&b_creat).unwrap().tapped);
    }

    #[test]
    fn draw_two_instant_plays_from_graveyard_cost_and_draws() {
        let registry = crate::card::CardRegistry::load(std::path::Path::new("cards")).unwrap();
        let draw_two = registry
            .cards()
            .iter()
            .find(|c| c.id == "draw-two")
            .unwrap()
            .clone();

        let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
        let instant_iid = s.a.hand[0].clone();
        // Swap to draw-two's card data (instant type, graveyard cost, handler).
        {
            let inst = s.card_pool.get_mut(&instant_iid).unwrap();
            inst.card = draw_two.clone();
        }
        // Seed the graveyard so the cost can be paid.
        let gy_seeds: Vec<_> = s.a.deck.drain(0..3).collect();
        s.a.graveyard.extend(gy_seeds.clone());

        let hand_before = s.a.hand.len();
        let deck_before = s.a.deck.len();
        let gy_before = s.a.graveyard.len();
        let exile_before = s.a.exile.len();

        s.play_card(
            PlayerId::A,
            &instant_iid,
            PlayChoices::default(),
            Some(&mut crate::game::EventContext::lua_only(registry.lua())),
        )
        .unwrap();

        // - Played card removed from hand.
        // - 3 graveyard cards exiled (cost).
        // - 2 cards drawn from deck into hand.
        // - Played card lands in graveyard.
        assert_eq!(s.a.hand.len(), hand_before - 1 + 2);
        assert_eq!(s.a.deck.len(), deck_before - 2);
        assert_eq!(s.a.graveyard.len(), gy_before - 3 + 1);
        assert_eq!(s.a.exile.len(), exile_before + 3);
        assert!(s.a.graveyard.contains(&instant_iid));
        assert!(!s.a.board.contains(&instant_iid));
        assert_eq!(s.event_fires[&crate::card::EventName::OnPlay], [1, 0]);
        // on_enter_board does NOT fire for instants.
        assert!(!s.event_fires.contains_key(&crate::card::EventName::OnEnterBoard));
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
            Some(&mut crate::game::EventContext::lua_only(registry.lua())),
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
            Some(&mut crate::game::EventContext::lua_only(registry.lua())),
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
