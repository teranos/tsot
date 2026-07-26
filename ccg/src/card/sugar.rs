//! Loader-side sugar: shorthand handler keys that translate into
//! `on_zone_change` filters.
//!
//! The engine has ONE zone-transition event (`OnZoneChange`). Cards
//! could always write:
//!
//! ```lua
//! on_zone_change = function(game, self, moving, from, to)
//!   if moving.instance_id ~= self.instance_id then return end
//!   if from ~= "board" or to ~= "graveyard" then return end
//!   ...
//! end
//! ```
//!
//! but three lines of filter boilerplate on every death handler drowns
//! the actual card logic. This module lets cards write:
//!
//! ```lua
//! on_die = function(game, self) ... end
//! ```
//!
//! and the loader wraps it into the equivalent `on_zone_change` filter
//! at load time. The engine never sees `on_die` — it's card-side
//! shorthand, not an engine event.
//!
//! Add new sugar keys here as new common patterns emerge. Each entry
//! is a single self-contained wrapper that composes into the final
//! `on_zone_change` handler.
//!
//! Currently supported keys:
//! - `on_die` — self dies (moving == self, board → graveyard)
//! - `on_creature_dies` — watcher, another creature dies
//! - `on_enter_board` — self enters board (not via attachment)
//! - `on_attached_as_cost` — self attached as HAND payment; receives
//!   `partner = { instance_id = <host> }`

use crate::card::EventName;
use mlua::{Function, Lua, Table, Value};
use std::collections::BTreeMap;

fn read_sugar(t: &Table, key: &str) -> mlua::Result<Option<Function>> {
    match t.get::<Value>(key)? {
        Value::Nil => Ok(None),
        Value::Function(f) => Ok(Some(f)),
        other => Err(mlua::Error::runtime(format!(
            "field `{key}` must be a function, got {other:?}"
        ))),
    }
}

/// Read all sugar keys from `t`. If any are present, build a composed
/// `on_zone_change` wrapper (dispatching to each sugar's filter) and
/// insert it into `handlers` under `EventName::OnZoneChange`. Any
/// user-declared `on_zone_change` handler already in `handlers` is
/// composed in as the final fallthrough call — it still runs on every
/// zone move regardless of the sugar filters.
pub(crate) fn apply(
    lua: &Lua,
    t: &Table,
    handlers: &mut BTreeMap<EventName, Function>,
) -> mlua::Result<()> {
    let on_die = read_sugar(t, "on_die")?;
    let on_creature_dies = read_sugar(t, "on_creature_dies")?;
    let on_enter_board = read_sugar(t, "on_enter_board")?;
    let on_attached_as_cost = read_sugar(t, "on_attached_as_cost")?;

    let any_sugar = on_die.is_some()
        || on_creature_dies.is_some()
        || on_enter_board.is_some()
        || on_attached_as_cost.is_some();
    if !any_sugar {
        return Ok(());
    }

    let existing_ozc = handlers.remove(&EventName::OnZoneChange);

    // Lua closures capture the sugar functions as upvalues cleanly; a
    // Rust closure would need mlua's scoped-function machinery which
    // fights the load-time contract (handlers must outlive `lua.scope`
    // calls in per-fire dispatch).
    let factory: Function = lua
        .load(
            r#"
            return function(on_die, on_creature_dies, on_enter_board, on_attached_as_cost, existing_ozc)
                return function(game, self, moving, from, to)
                    if on_die
                       and moving.instance_id == self.instance_id
                       and from == "board" and to == "graveyard" then
                        on_die(game, self)
                    end
                    if on_creature_dies
                       and from == "board" and to == "graveyard"
                       and moving.instance_id ~= self.instance_id then
                        local c = game.card(moving.instance_id)
                        if c and c.type == "creature" then
                            on_creature_dies(game, self, moving)
                        end
                    end
                    if on_enter_board
                       and moving.instance_id == self.instance_id
                       and to == "board"
                       and not game.host_of(self.instance_id) then
                        on_enter_board(game, self)
                    end
                    if on_attached_as_cost
                       and moving.instance_id == self.instance_id
                       and from == "hand" and to == "board" then
                        local host_iid = game.host_of(self.instance_id)
                        if host_iid then
                            on_attached_as_cost(game, self, { instance_id = host_iid })
                        end
                    end
                    if existing_ozc then
                        existing_ozc(game, self, moving, from, to)
                    end
                end
            end
            "#,
        )
        .eval()?;
    let wrapper: Function = factory.call((
        on_die,
        on_creature_dies,
        on_enter_board,
        on_attached_as_cost,
        existing_ozc,
    ))?;
    handlers.insert(EventName::OnZoneChange, wrapper);
    Ok(())
}
