//! Activated abilities (RULES A.5–A.10). Activations resolve inline —
//! no stack, no response window per A.5. Extracted from `play.rs` so
//! the cast loop and the activate loop don't share a file.
//!
//! - `activate_ability` — fire an activation, paying its cost and
//!   running its effect.
//! - `can_activate` / `can_activate_with_x` — read-only sim-AI
//!   pre-checks that match `activate_ability`'s validation.

use super::super::context::EventContext;
use super::super::lua_api;
use super::super::state::{GameState, InstanceId, Zone};
use super::errors::ActivateError;
use crate::card::{CardType, CostSource};

impl GameState {
    /// Fire the activated ability at index `ability_idx` on `iid`.
    /// Per RULES A.5: pays the cost, then resolves the effect inline —
    /// no stack, no response window. Caller validates eligibility via
    /// `can_activate` before calling; this method re-validates and
    /// returns an `ActivateError` if the call slipped through stale.
    pub fn activate_ability(
        &mut self,
        iid: &InstanceId,
        ability_idx: usize,
        x_value: Option<i32>,
        choices: super::ActivateChoices,
        mut ctx: Option<&mut EventContext>,
    ) -> Result<(), ActivateError> {
        // First pass: read everything we need from the card_pool entry
        // and from any static-granted activation at this index. Then
        // release the borrows. All subsequent steps may mutate self
        // (set_tapped, smart-discard, fire_validate, etc.) — they
        // can't coexist with immutable borrows on inst/ability.
        let (
            controller,
            is_creature,
            inst_tapped,
            inst_summoning_sick,
            cost_tap,
            components,
            handler,
            validate,
            ability_target,
            allow_x_zero,
            from_zones,
        ) = {
            let inst = self
                .card_pool
                .get(iid)
                .ok_or(ActivateError::SourceMissing)?;
            // Index walks printed activations first, then static-granted
            // ones via activation_at. Both paths share the same shape.
            let ability = self
                .activation_at(iid, ability_idx)
                .ok_or(ActivateError::NoSuchAbility)?;
            (
                inst.controller,
                inst.card().kind == CardType::Creature,
                inst.tapped,
                inst.summoning_sick,
                ability.cost_tap,
                ability.cost_components.clone(),
                ability.effect.clone(),
                ability.validate.clone(),
                ability.target,
                inst.card().allow_x_zero,
                ability.from_zones.clone(),
            )
        };
        // RULES A.9 + P.32: declarative target category. If set and no
        // legal target exists, refuse activation before any cost.
        if let Some(target) = ability_target {
            if !self.is_target_legal(target) {
                return Err(ActivateError::NoLegalTarget);
            }
        }

        // Source must be in one of the ability's declared from_zones.
        if !self.iid_in_any_activation_zone(iid, controller, &from_zones) {
            return Err(ActivateError::NotOnBoard);
        }

        // Tap-cost gate.
        if cost_tap {
            if inst_tapped {
                return Err(ActivateError::AlreadyTapped);
            }
            if is_creature && inst_summoning_sick && !self.has_keyword(iid, "haste") {
                return Err(ActivateError::SummoningSick);
            }
        }

        // Component-cost gate. Variable-X components (`is_x = true`)
        // multiply by x_value; the caller is required to provide a
        // value if any component uses X. Pre-validate every component
        // is payable from the controller's current zones. Once we
        // pass this, the payment loop below cannot fail half-way.
        let has_x = components.iter().any(|c| c.is_x);
        if has_x && x_value.is_none() {
            return Err(ActivateError::CannotPayComponents);
        }
        // RULES P.30: minimum X = 1 unless the card opts into X = 0.
        if has_x {
            if let Some(v) = x_value {
                if v < 1 && !allow_x_zero {
                    return Err(ActivateError::XBelowMinimum);
                }
            }
        }
        let x_val = x_value.unwrap_or(0).max(0);
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        let mut sacrifice_need = 0usize;
        let mut self_exiles = false;
        for c in &components {
            let amount = effective_cost_amount(c, x_val);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice => sacrifice_need += amount,
                CostSource::SelfExile => {
                    // P.5: source moves BOARD → EXILE after effect.
                    if c.amount.max(0) > 0 {
                        self_exiles = true;
                    }
                    let _ = amount;
                }
                CostSource::Attached => {
                    // TODO: ATTACHED in activated cost (non-BOARD-zone
                    // activation slice).
                    return Err(ActivateError::CannotPayComponents);
                }
            }
        }
        let p = self.player(controller);
        if p.hand.len() < hand_need
            || p.deck.len() < mill_need
            || p.graveyard.len() < gy_need
        {
            return Err(ActivateError::CannotPayComponents);
        }

        // P.16: SACRIFICE validation, i-th-pairing with components.
        if choices.sacrifice_ids.len() != sacrifice_need {
            return Err(ActivateError::WrongSacrificeCount {
                expected: sacrifice_need,
                got: choices.sacrifice_ids.len(),
            });
        }
        let sac_kinds: Vec<Option<crate::card::CardType>> = components
            .iter()
            .filter(|c| matches!(c.source, CostSource::Sacrifice))
            .flat_map(|c| {
                let n = effective_cost_amount(c, x_val);
                std::iter::repeat_n(c.kind, n)
            })
            .collect();
        let mut sac_seen: std::collections::BTreeSet<&InstanceId> =
            std::collections::BTreeSet::new();
        for (i, sid) in choices.sacrifice_ids.iter().enumerate() {
            if !sac_seen.insert(sid) {
                return Err(ActivateError::DuplicateSacrifice(sid.clone()));
            }
            if !self.player(controller).board.contains(sid) {
                return Err(ActivateError::SacrificePaymentInvalid(sid.clone()));
            }
            let Some(sac_inst) = self.card_pool.get(sid) else {
                return Err(ActivateError::SacrificePaymentInvalid(sid.clone()));
            };
            if sac_inst.controller != controller {
                return Err(ActivateError::SacrificePaymentInvalid(sid.clone()));
            }
            if let Some(required_kind) = sac_kinds.get(i).copied().flatten() {
                if sac_inst.card().kind != required_kind {
                    return Err(ActivateError::SacrificePaymentInvalid(sid.clone()));
                }
            }
        }

        // Expose the X value to both validate and effect handlers via
        // `game.x_value()`. Saved/restored around the entire
        // validate→pay→effect sequence so a card's validate hook can
        // refuse based on X-dependent math (e.g., dark-salamander's
        // "2Y - X must be > 0").
        let prior_x = self.current_activation_x;
        self.current_activation_x = x_value;

        // RULES A.9: optional `validate` hook. If present, the activation
        // can only be initiated when validate returns truthy — typically
        // "a legal target exists." No cost is paid if validate refuses.
        // Without ctx (engine calls without a Lua VM), validate is
        // skipped — caller's responsibility, used by some tests.
        if let Some(v_fn) = validate {
            if let Some(c) = ctx.as_deref_mut() {
                if !lua_api::fire_validate(c.lua, self, c.oracle(), iid, v_fn) {
                    self.current_activation_x = prior_x;
                    return Err(ActivateError::NoLegalTarget);
                }
            }
        }

        // Pay tap cost.
        if cost_tap {
            self.set_tapped(iid, true);
        }

        // Pay component costs. HAND uses the same smart-discard ranking
        // as `game.discard` (least-useful first). MILL takes top of own
        // deck. GRAVEYARD moves cards from GY to EXILE (matching the
        // play-card convention — graveyard payments don't recycle).
        for c in &components {
            let amount = effective_cost_amount(c, x_val);
            match c.source {
                CostSource::Hand => {
                    lua_api::do_smart_discard(self, controller, amount);
                }
                CostSource::Mill => {
                    for _ in 0..amount {
                        if let Some(top) = self.player(controller).deck.first().cloned() {
                            let _ = self.move_card_or_emit(
                                &top,
                                controller,
                                Zone::Deck,
                                Zone::Graveyard,
                                "activate-mill-cost",
                            );
                            self.bump_action("mill", controller);
                        }
                    }
                }
                CostSource::Graveyard => {
                    for _ in 0..amount {
                        if let Some(card) = self.player(controller).graveyard.first().cloned() {
                            let _ = self.move_card_or_emit(
                                &card,
                                controller,
                                Zone::Graveyard,
                                Zone::Exile,
                                "activate-graveyard-cost",
                            );
                        }
                    }
                }
                CostSource::Sacrifice | CostSource::SelfExile => {}
                CostSource::Attached => {
                    unreachable!("attached rejected at validation");
                }
            }
        }
        for sid in &choices.sacrifice_ids {
            let _ = self.move_card_or_emit(
                sid,
                controller,
                Zone::Board,
                Zone::Graveyard,
                "activate-sacrifice-cost",
            );
            self.bump_action("sacrificed_as_cost", controller);
        }
        if let Some(c) = ctx.as_deref_mut() {
            for sid in &choices.sacrifice_ids {
                lua_api::fire_self_only(
                    c.lua,
                    self,
                    c.oracle(),
                    crate::card::EventName::OnDie,
                    sid,
                )
                .map_err(ActivateError::ChoicePending)?;
            }
        }

        // Telemetry: bump per-controller action count so HTML reports
        // can show "X activations per game" alongside plays, attacks,
        // and engine actions. Keyed plainly as "activate" so it sums
        // across all activated abilities.
        self.bump_action("activate", controller);

        // Fire effect. Per A.5 this is inline / synchronous; the
        // handler returning is the end of the activation. The X value
        // remains visible via `game.x_value()` (set above before the
        // validate hook).
        if let Some(c) = ctx {
            lua_api::fire_activated(c.lua, self, c.oracle(), iid, handler)
                .map_err(ActivateError::ChoicePending)?;
        }

        // P.5: source → EXILE after effect. Must use the iid's actual
        // current zone — for `from_zones = {Attached}` the source lives
        // in some host's `attached` list (not Board); for `Graveyard`
        // it lives in the graveyard; etc. Hardcoding `Zone::Board`
        // here would emit `NotInZone` on every non-Board activation.
        if self_exiles {
            if let Some(host) = self.host_of(iid) {
                // Attached source: detach first, then place in exile.
                self.remove_attached(&host, iid);
                self.add_to_zone(iid, controller, Zone::Exile);
            } else if self.player(controller).board.contains(iid) {
                let _ = self.move_card_or_emit(
                    iid,
                    controller,
                    Zone::Board,
                    Zone::Exile,
                    "activate-self-exile-cost",
                );
            } else if self.player(controller).graveyard.contains(iid) {
                let _ = self.move_card_or_emit(
                    iid,
                    controller,
                    Zone::Graveyard,
                    Zone::Exile,
                    "activate-self-exile-cost",
                );
            } else if self.player(controller).hand.contains(iid) {
                let _ = self.move_card_or_emit(
                    iid,
                    controller,
                    Zone::Hand,
                    Zone::Exile,
                    "activate-self-exile-cost",
                );
            } else if self.player(controller).deck.contains(iid) {
                let _ = self.move_card_or_emit(
                    iid,
                    controller,
                    Zone::Deck,
                    Zone::Exile,
                    "activate-self-exile-cost",
                );
            } else if self.player(controller).exile.contains(iid) {
                // Already in exile — no-op (rare but legal: a card
                // could declare SELF-exile from Exile, redundant cost).
            }
        }

        self.current_activation_x = prior_x;
        Ok(())
    }

    /// Read-only eligibility check for the sim AI's activation pass.
    /// Returns true iff a subsequent `activate_ability(iid, ability_idx)`
    /// call would succeed. Matches `activate_ability`'s validation
    /// exactly so the AI never speculatively calls and fails.
    pub fn can_activate(&self, iid: &InstanceId, ability_idx: usize) -> bool {
        // Permissive pre-check: treats is_x components as "affordable
        // at X=1." The exact X is chosen by the caller (sim AI) and
        // re-validated inside `activate_ability`. Returns true here
        // when the AI should consider this activation; the AI is
        // expected to follow up with a concrete x_value if needed.
        self.can_activate_with_x(iid, ability_idx, 1)
    }

    /// Like `can_activate` but checks affordability for a specific
    /// X value. Useful for the sim AI when it wants to commit to a
    /// specific X before calling `activate_ability`.
    pub fn can_activate_with_x(
        &self,
        iid: &InstanceId,
        ability_idx: usize,
        x_value: i32,
    ) -> bool {
        let Some(inst) = self.card_pool.get(iid) else {
            return false;
        };
        let Some(ability) = self.activation_at(iid, ability_idx) else {
            return false;
        };
        if !self.iid_in_any_activation_zone(iid, inst.controller, &ability.from_zones) {
            return false;
        }
        // RULES P.32: declarative target category — refuse if no legal
        // target exists. Mirrors the engine's activate_ability gate.
        if let Some(target) = ability.target {
            if !self.is_target_legal(target) {
                return false;
            }
        }
        if ability.cost_tap {
            if inst.tapped {
                return false;
            }
            let is_creature = inst.card().kind == CardType::Creature;
            if is_creature && inst.summoning_sick && !self.has_keyword(iid, "haste") {
                return false;
            }
        }
        // Component-cost affordability with the supplied X value.
        // is_x components multiply by x_value.
        let x = x_value.max(0);
        let mut hand_need = 0usize;
        let mut mill_need = 0usize;
        let mut gy_need = 0usize;
        let mut sacrifice_need = 0usize;
        for c in &ability.cost_components {
            let amount = effective_cost_amount(c, x);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice => sacrifice_need += amount,
                CostSource::SelfExile => {
                    // P.5: source itself pays — already verified on board.
                }
                CostSource::Attached => {
                    return false;
                }
            }
        }
        let p = self.player(inst.controller);
        if p.hand.len() < hand_need || p.deck.len() < mill_need || p.graveyard.len() < gy_need {
            return false;
        }
        // SACRIFICE pre-flight: need at least N controllable BOARD cards.
        // (Caller supplies the exact sacrifice_ids when firing; the gate
        // just checks "is there enough to sacrifice.")
        if sacrifice_need > 0 && p.board.iter().filter(|i| *i != iid).count() < sacrifice_need {
            return false;
        }
        true
    }
}

/// Per-component effective amount: `is_x` components multiply by the
/// activation's X value; non-X components use the printed `amount`.
fn effective_cost_amount(c: &crate::card::CostComponent, x_value: i32) -> usize {
    if c.is_x {
        x_value.max(0) as usize
    } else {
        c.amount.max(0) as usize
    }
}

#[cfg(test)]
mod choice_pending_tests {
    //! Mirror of the `fire_self_only` propagation tests at the
    //! `activate_ability` boundary. Pins the contract that when an
    //! activated ability's `effect` calls `game.choose_*` against a
    //! HumanReplayOracle with no replay, `activate_ability` returns
    //! `Err(ActivateError::ChoicePending(_))` rather than silently
    //! eating the suspend.

    use super::*;
    use crate::card::{ActivatedAbility, Timing};
    use crate::choice::{ChoicePending, RandomOracle};
    use crate::game::context::EventContext;
    use crate::game::test_helpers::deck_of;
    use crate::game::{GameState, PlayerId};
    use crate::sim::human::HumanReplayOracle;
    use mlua::Lua;
    use rand::SeedableRng;

    /// Today: discard at `activate.rs:201` swallowed the Pending and
    /// `activate_ability` returned `Ok(())` — the human never saw the
    /// prompt. After the fix: propagates as `Err(ChoicePending(_))`.
    #[test]
    fn activate_ability_returns_choice_pending_when_effect_yields() {
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let card_iid = state.a.hand[0].clone();

        // Park the iid on BOARD so the activation passes the NotOnBoard /
        // SummoningSick gates. Match B.3 untap/sick reset.
        state
            .move_card(&card_iid, PlayerId::A, Zone::Hand, Zone::Board)
            .unwrap();
        state.set_summoning_sick(&card_iid, false);

        // Install a single activated ability whose `effect` calls
        // `game.choose_card` on a 1-element pool. The pool element is
        // a placeholder iid — the wrapper short-circuits to Pending
        // before any candidate is read.
        let target = state.b.hand[0].clone();
        let effect_src = format!(
            r#"return function(game, self)
                 local picked = game.choose_card({{ "{target}" }}, {{ prompt = "test" }})
                 if picked ~= nil then game.damage(picked, 1) end
               end"#
        );
        let effect: mlua::Function = lua.load(&effect_src).eval().unwrap();
        state
            .card_pool
            .get_mut(&card_iid)
            .unwrap()
            .card_mut()
            .activated
            .push(ActivatedAbility {
                cost_tap: false,
                cost_components: vec![],
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
                from_zones: vec![crate::card::ActivationZone::Board],
            });

        // Empty replay → the first choose_card call returns Pending.
        let mut oracle = HumanReplayOracle::new(
            RandomOracle::new(rand::rngs::StdRng::seed_from_u64(0)),
            Some(PlayerId::A),
        );

        let result = {
            let mut ctx = EventContext::new(&lua, &mut oracle);
            state.activate_ability(
                &card_iid,
                0,
                None,
                crate::game::play::ActivateChoices::default(),
                Some(&mut ctx),
            )
        };

        match result {
            Err(ActivateError::ChoicePending(ChoicePending::Card(req))) => {
                assert_eq!(req.asker, Some(PlayerId::A));
                assert!(
                    !req.pool.is_empty(),
                    "ChoicePending must carry the original pool back up"
                );
            }
            other => panic!(
                "expected Err(ActivateError::ChoicePending(Card)), got {other:?}"
            ),
        }
    }

    use crate::card::{CardType, CostComponent, CostSource};
    use crate::game::play::ActivateChoices;

    #[test]
    fn activate_ability_with_sacrifice_cost_moves_victim_to_graveyard() {
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let source_iid = state.a.hand[0].clone();
        let victim_iid = state.a.hand[1].clone();

        // Both on BOARD, both controllable + sacrificable. Mark victim as
        // a creature so the kind filter accepts it.
        state
            .move_card(&source_iid, PlayerId::A, Zone::Hand, Zone::Board)
            .unwrap();
        state
            .move_card(&victim_iid, PlayerId::A, Zone::Hand, Zone::Board)
            .unwrap();
        state.set_summoning_sick(&source_iid, false);
        state.set_summoning_sick(&victim_iid, false);
        state.card_pool.get_mut(&victim_iid).unwrap().card_mut().kind = CardType::Creature;

        // Effect bumps a global so we can pin "effect ran" cleanly.
        let effect: mlua::Function = lua
            .load(
                r#"return function(game, self)
                     _G.sac_activation_effect_count = (_G.sac_activation_effect_count or 0) + 1
                   end"#,
            )
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(&source_iid)
            .unwrap()
            .card_mut()
            .activated
            .push(ActivatedAbility {
                cost_tap: false,
                cost_components: vec![CostComponent {
                    amount: 1,
                    source: CostSource::Sacrifice,
                    is_x: false,
                    kind: Some(CardType::Creature),
                }],
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
                from_zones: vec![crate::card::ActivationZone::Board],
            });

        lua.globals()
            .set("sac_activation_effect_count", 0_i32)
            .unwrap();

        let result = {
            let mut ctx = EventContext::lua_only(&lua);
            state.activate_ability(
                &source_iid,
                0,
                None,
                ActivateChoices {
                    sacrifice_ids: vec![victim_iid.clone()],
                },
                Some(&mut ctx),
            )
        };

        assert!(result.is_ok(), "activation should succeed: {result:?}");
        assert!(
            state.a.graveyard.contains(&victim_iid),
            "victim must be in controller's graveyard"
        );
        assert!(
            !state.a.board.contains(&victim_iid),
            "victim must leave board"
        );
        let fired: i32 = lua
            .globals()
            .get("sac_activation_effect_count")
            .unwrap();
        assert_eq!(fired, 1, "activation effect should have run once");
    }

    #[test]
    fn activate_ability_with_self_exile_cost_moves_source_to_exile_after_effect() {
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let source_iid = state.a.hand[0].clone();

        state
            .move_card(&source_iid, PlayerId::A, Zone::Hand, Zone::Board)
            .unwrap();
        state.set_summoning_sick(&source_iid, false);

        // Effect records self.instance_id at fire time so we can verify
        // the source is still readable during the effect (i.e., the
        // SelfExile move happens AFTER the effect, not before).
        let effect: mlua::Function = lua
            .load(
                r#"return function(game, self)
                     _G.self_exile_observed_iid = self.instance_id
                   end"#,
            )
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(&source_iid)
            .unwrap()
            .card_mut()
            .activated
            .push(ActivatedAbility {
                cost_tap: false,
                cost_components: vec![CostComponent {
                    amount: 1,
                    source: CostSource::SelfExile,
                    is_x: false,
                    kind: None,
                }],
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
                from_zones: vec![crate::card::ActivationZone::Board],
            });

        let result = {
            let mut ctx = EventContext::lua_only(&lua);
            state.activate_ability(
                &source_iid,
                0,
                None,
                ActivateChoices::default(),
                Some(&mut ctx),
            )
        };

        assert!(result.is_ok(), "activation should succeed: {result:?}");
        let observed: String = lua.globals().get("self_exile_observed_iid").unwrap();
        assert_eq!(
            observed,
            source_iid.to_string(),
            "effect must observe self before the SelfExile move",
        );
        assert!(
            state.a.exile.contains(&source_iid),
            "source must be in exile after activation"
        );
        assert!(
            !state.a.board.contains(&source_iid),
            "source must leave board"
        );
    }

    #[test]
    fn activate_ability_fires_from_graveyard_when_zone_declared() {
        use crate::card::ActivationZone;
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let card_iid = state.a.hand[0].clone();
        state
            .move_card(&card_iid, PlayerId::A, Zone::Hand, Zone::Graveyard)
            .unwrap();

        let effect: mlua::Function = lua
            .load(
                r#"return function(game, self)
                     _G.gy_activation_count = (_G.gy_activation_count or 0) + 1
                   end"#,
            )
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(&card_iid)
            .unwrap()
            .card_mut()
            .activated
            .push(ActivatedAbility {
                cost_tap: false,
                cost_components: vec![],
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
                from_zones: vec![ActivationZone::Graveyard],
            });

        lua.globals().set("gy_activation_count", 0_i32).unwrap();
        assert!(
            state.can_activate(&card_iid, 0),
            "graveyard-zoned activation should pass can_activate"
        );
        let result = {
            let mut ctx = EventContext::lua_only(&lua);
            state.activate_ability(
                &card_iid,
                0,
                None,
                ActivateChoices::default(),
                Some(&mut ctx),
            )
        };
        assert!(result.is_ok(), "activation from graveyard: {result:?}");
        let fired: i32 = lua.globals().get("gy_activation_count").unwrap();
        assert_eq!(fired, 1);
    }

    #[test]
    fn activate_ability_fires_from_attached_when_zone_declared() {
        use crate::card::ActivationZone;
        let lua = Lua::new();
        let mut state = GameState::new(deck_of(10, "a"), deck_of(10, "b"));
        let host_iid = state.a.hand[0].clone();
        let attached_iid = state.a.hand[1].clone();
        state
            .move_card(&host_iid, PlayerId::A, Zone::Hand, Zone::Board)
            .unwrap();
        state.a.hand.retain(|i| i != &attached_iid);
        state.add_attached(&host_iid, &attached_iid);

        let effect: mlua::Function = lua
            .load(
                r#"return function(game, self)
                     _G.attached_activation_count = (_G.attached_activation_count or 0) + 1
                   end"#,
            )
            .eval()
            .unwrap();
        state
            .card_pool
            .get_mut(&attached_iid)
            .unwrap()
            .card_mut()
            .activated
            .push(ActivatedAbility {
                cost_tap: false,
                cost_components: vec![],
                text: String::new(),
                timing: Timing::Instant,
                validate: None,
                target: None,
                effect,
                from_zones: vec![ActivationZone::Attached],
            });

        lua.globals().set("attached_activation_count", 0_i32).unwrap();
        assert!(
            state.can_activate(&attached_iid, 0),
            "attached-zoned activation should pass can_activate"
        );
        let result = {
            let mut ctx = EventContext::lua_only(&lua);
            state.activate_ability(
                &attached_iid,
                0,
                None,
                ActivateChoices::default(),
                Some(&mut ctx),
            )
        };
        assert!(result.is_ok(), "activation from attached: {result:?}");
        let fired: i32 = lua.globals().get("attached_activation_count").unwrap();
        assert_eq!(fired, 1);
    }
}
