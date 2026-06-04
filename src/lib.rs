// S12: `Arc<CardRegistry>` is shared cheaply across the engine + AI
// rollouts even though `CardRegistry` owns a `!Send + !Sync` `mlua::Lua`
// VM. We never cross threads with it (each thread / rollout owns its
// own); the Arc is only used to avoid cloning the (uncloneable) Lua
// VM between owners. Suppress clippy's correctness warning everywhere.
#![allow(clippy::arc_with_non_send_sync)]

pub mod card;
pub mod cast_routing;
pub mod choice;
pub mod game;
pub mod replay;
pub mod sim;

// FFI surface for the WASM frontend. Browser JS calls these via
// emscripten's `Module.ccall("tsot_*", ...)`. JSON-string payloads
// over `*const c_char` for arguments and returns — keeps the FFI
// boundary simple at the cost of (de)serialization on each call.
//
// The module itself compiles on every target so the session-management
// plumbing (GameSession, install/with/clear) is exercisable by `cargo
// test`. Only the `#[no_mangle] extern "C"` exports + the
// `emscripten_sleep` extern declaration are gated to wasm32, since
// emscripten owns those symbols.
pub mod wasm_ffi;

pub use cast_routing::CastRouting;

pub use card::{
    Card, CardRegistry, CardType, CostComponent, CostSource, EventName, ModifierValue, Stats,
    StaticAffects, StaticController, StaticDef, Timing,
};
pub use choice::{
    ChoiceOracle, ChooseCardRequest, ChooseIntRequest, ChoosePlayerRequest, NoopOracle,
    RandomOracle, RecordingOracle, ScriptedAnswer, ScriptedOracle,
};
pub use game::{
    CardInstance, GameState, Modifier, MoveError, Phase, PlayerId, PlayerState, StatusEffect, Zone,
};
