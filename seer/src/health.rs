// Ported from rave/src/health.rs — thin re-export of the rave-health
// crate. Call sites keep using `crate::health::*` regardless of which
// project consumes them.
//
// Value in seer: Health is the "what is wrong RIGHT NOW" surface,
// distinct from the sacred-error bus ("what just happened"). Bringing
// it in unlocks a Health Resource for tracking durable conditions —
// e.g. a LeakDetectedEntry set when the verdict fails, cleared when
// the next commit lands below threshold. Follow-up commits wire the
// actual conditions; this port just gets the primitive available.

pub use rave_health::*;
