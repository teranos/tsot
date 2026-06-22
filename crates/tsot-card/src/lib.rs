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

#![no_std]

use serde::{Deserialize, Serialize};

/// Opaque identifier for a TSOT card. The integer's meaning is the
/// catalog agreement between the two projects; both treat it as an
/// uninterpreted handle. Zero is reserved as "unset/invalid" by
/// convention so a default-constructed `CardId` never collides with a
/// real card.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CardId(pub u32);
