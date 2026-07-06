// Thin re-export shim for the extracted seer-obs workspace crate
// (Task 12). Kept as `crate::obs` so every existing call site
// (`crate::obs::emit`, `crate::obs::gpu_totals`, etc.) still resolves
// without a global sweep. The observability contract itself lives in
// crates/seer-obs/src/lib.rs.

pub use seer_obs::*;
