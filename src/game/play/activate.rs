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
                inst.card.kind == CardType::Creature,
                inst.tapped,
                inst.summoning_sick,
                ability.cost_tap,
                ability.cost_components.clone(),
                ability.effect.clone(),
                ability.validate.clone(),
                ability.target,
                inst.card.allow_x_zero,
            )
        };
        // RULES A.9 + P.32: declarative target category. If set and no
        // legal target exists, refuse activation before any cost.
        if let Some(target) = ability_target {
            if !self.is_target_legal(target) {
                return Err(ActivateError::NoLegalTarget);
            }
        }

        // Source must be on its controller's BOARD. v1 doesn't model
        // activations from hand / graveyard / attached.
        if !self.player(controller).board.contains(iid) {
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
        for c in &components {
            let amount = effective_cost_amount(c, x_val);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile | CostSource::Attached => {
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
                            let _ = self.move_card(&top, controller, Zone::Deck, Zone::Graveyard);
                            self.bump_action("mill", controller);
                        }
                    }
                }
                CostSource::Graveyard => {
                    for _ in 0..amount {
                        if let Some(card) = self.player(controller).graveyard.first().cloned() {
                            let _ = self.move_card(&card, controller, Zone::Graveyard, Zone::Exile);
                        }
                    }
                }
                _ => unreachable!("sacrifice / self-exile rejected at validation"),
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
            // TODO(lua-yield): ActivateError doesn't carry ChoicePending.
            // Lift when an activated ability needs to yield mid-resolution.
            let _ = lua_api::fire_activated(c.lua, self, c.oracle(), iid, handler);
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
        if !self.player(inst.controller).board.contains(iid) {
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
            let is_creature = inst.card.kind == CardType::Creature;
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
        for c in &ability.cost_components {
            let amount = effective_cost_amount(c, x);
            match c.source {
                CostSource::Hand => hand_need += amount,
                CostSource::Mill => mill_need += amount,
                CostSource::Graveyard => gy_need += amount,
                CostSource::Sacrifice | CostSource::SelfExile | CostSource::Attached => {
                    return false;
                }
            }
        }
        let p = self.player(inst.controller);
        p.hand.len() >= hand_need && p.deck.len() >= mill_need && p.graveyard.len() >= gy_need
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
