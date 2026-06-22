//! TSOT card primitives shared at the source level between roam and ccg.
//!
//! The two projects compile to incompatible wasm targets (roam:
//! `wasm32-unknown-unknown` + wasm-bindgen; ccg: `wasm32-unknown-emscripten`
//! + mlua) and will forever ship as two separate wasm modules. They can
//! still share Rust types at the source level — this crate is the place
//! for primitives that cross the roam/ccg boundary.
//!
//! Scope rule: only put types here that are useful to BOTH projects.
//! Engine logic, Lua-loaded handlers, full `Card` definitions — all of
//! that lives in ccg. World-state, render, networking — lives in roam.

extern crate alloc;

use alloc::string::String;
use serde::{Deserialize, Serialize};

/// Authoritative identifier for a TSOT card. The string matches ccg's
/// card slug (`id = "amsterdam-city"` in the Lua source), which is the
/// canonical handle for that card across the project. Stored as
/// `String` because cards are constructed at low frequency (pickup,
/// catalog load) — the per-instance allocation cost is negligible vs.
/// the clarity of using the same identifier ccg uses end-to-end.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CardId(pub String);
