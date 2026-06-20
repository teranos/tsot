//! Per-call event context bundling the Lua VM + choice oracle.
//!
//! Engine methods that fire handlers take `Option<&mut EventContext>`. `None`
//! means "skip handler execution" — used by tests of pure game logic. `Some`
//! means "run handlers"; the oracle answers any `game.choose_card` /
//! `game.confirm` prompts they emit.
//!
//! `EventContext::lua_only(lua)` constructs a context whose oracle is an
//! internal `NoopOracle` — for tests that exercise a handler but don't
//! actually invoke any choice prompts. `EventContext::new(lua, &mut oracle)`
//! constructs one with a real oracle (sim's `RandomOracle`, test's
//! `ScriptedOracle`, etc.).

use crate::choice::{ChoiceOracle, NoopOracle};
use mlua::Lua;

pub struct EventContext<'a> {
    pub lua: &'a Lua,
    noop: NoopOracle,
    oracle_override: Option<&'a mut dyn ChoiceOracle>,
}

impl<'a> EventContext<'a> {
    pub fn new(lua: &'a Lua, oracle: &'a mut dyn ChoiceOracle) -> Self {
        Self {
            lua,
            noop: NoopOracle,
            oracle_override: Some(oracle),
        }
    }

    pub fn lua_only(lua: &'a Lua) -> Self {
        Self {
            lua,
            noop: NoopOracle,
            oracle_override: None,
        }
    }

    pub fn oracle(&mut self) -> &mut dyn ChoiceOracle {
        match &mut self.oracle_override {
            Some(o) => *o,
            None => &mut self.noop,
        }
    }
}
