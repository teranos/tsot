//! Health primitives live in `crates/rave-health/` so CI's Linux
//! runner can execute the tests, not just type-check them. This file
//! is a thin re-export — call sites keep using `crate::health::*`.

pub use rave_health::*;
