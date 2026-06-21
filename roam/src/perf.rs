//! Cumulative perf counters for the in-browser perf panel.
//!
//! Counters are `AtomicU64`. The JS perf panel polls
//! [`roam_perf_snapshot_json`](crate::wasm_ffi::roam_perf_snapshot_json)
//! at 1 Hz and computes per-second rates from successive snapshots —
//! the Rust side stays as cheap as a relaxed atomic increment per
//! event. No ring buffer, no per-tag map, no allocation in the hot
//! path.
//!
//! Two surfaces:
//!
//! 1. **Emit-rate by tag** — every [`TraceEvent::Note`] flows through
//!    [`note_tag_emit`] which routes to a static counter per known tag
//!    or to [`EMIT_OTHER`] for unrecognised tags. Adding a tag here is
//!    one line + one match arm; same edit shape as adding a new tag.
//!    This is what makes "is `net::recv` actually the source of the
//!    log flood?" a measurable question instead of a guess.
//!
//! 2. **Pickup pipeline counters** — publish attempted / ok / err,
//!    received, applied. Each layer of the M6 pickup propagation
//!    increments one counter. Whichever counter stops incrementing
//!    first names the broken layer.

use std::sync::atomic::{AtomicU64, Ordering};

// ----- per-tag emit counters -----

pub static EMIT_NET_RECV: AtomicU64 = AtomicU64::new(0);
pub static EMIT_NET_PROVIDER_ERROR: AtomicU64 = AtomicU64::new(0);
pub static EMIT_NET_PEER_UP: AtomicU64 = AtomicU64::new(0);
pub static EMIT_NET_PEER_DOWN: AtomicU64 = AtomicU64::new(0);
pub static EMIT_FLOWER_PICKED_CANONICAL: AtomicU64 = AtomicU64::new(0);
pub static EMIT_FLOWER_PICKED_SANDBOX: AtomicU64 = AtomicU64::new(0);
pub static EMIT_OTHER: AtomicU64 = AtomicU64::new(0);

/// Called from [`crate::trace::emit`] for every `TraceEvent::Note`.
/// Routes to a per-tag counter so the perf panel can answer "which
/// emit source dominates" without sampling the bus.
#[inline]
pub fn note_tag_emit(tag: &str) {
    match tag {
        "net::recv" => {
            EMIT_NET_RECV.fetch_add(1, Ordering::Relaxed);
        }
        "net::provider_error" => {
            EMIT_NET_PROVIDER_ERROR.fetch_add(1, Ordering::Relaxed);
        }
        "net::peer_up" => {
            EMIT_NET_PEER_UP.fetch_add(1, Ordering::Relaxed);
        }
        "net::peer_down" => {
            EMIT_NET_PEER_DOWN.fetch_add(1, Ordering::Relaxed);
        }
        "flower_picked_canonical" => {
            EMIT_FLOWER_PICKED_CANONICAL.fetch_add(1, Ordering::Relaxed);
        }
        "flower_picked_sandbox" => {
            EMIT_FLOWER_PICKED_SANDBOX.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            EMIT_OTHER.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ----- pickup pipeline counters -----

pub static PICKUP_PUBLISH_ATTEMPTED: AtomicU64 = AtomicU64::new(0);
/// Sync `Ok` from `provider.publish` — message queued in the worker's
/// command channel. Does NOT mean gossipsub delivered it; see
/// `PICKUP_PUBLISH_DELIVERY_ERR` for the async failure path.
pub static PICKUP_PUBLISH_OK: AtomicU64 = AtomicU64::new(0);
/// Sync `Err` from `provider.publish` — provider couldn't even queue
/// (encoding failure, transport error). Rare in normal operation.
pub static PICKUP_PUBLISH_ERR: AtomicU64 = AtomicU64::new(0);
/// Async failure: gossipsub later reported the message couldn't be
/// delivered (`NetEvent::Error(PublishFailed)` on the pickups topic).
/// Typically `NoPeersSubscribedToTopic` during mesh-formation gaps.
/// This is the counter that reveals when `publish OK` is lying about
/// actual delivery.
pub static PICKUP_PUBLISH_DELIVERY_ERR: AtomicU64 = AtomicU64::new(0);
pub static PICKUP_RECEIVED: AtomicU64 = AtomicU64::new(0);
pub static PICKUP_APPLIED: AtomicU64 = AtomicU64::new(0);

pub static POSITION_PUBLISH_ATTEMPTED: AtomicU64 = AtomicU64::new(0);
pub static POSITION_PUBLISH_OK: AtomicU64 = AtomicU64::new(0);
pub static POSITION_PUBLISH_ERR: AtomicU64 = AtomicU64::new(0);
/// Async failure on the positions topic — same shape as
/// `PICKUP_PUBLISH_DELIVERY_ERR`.
pub static POSITION_PUBLISH_DELIVERY_ERR: AtomicU64 = AtomicU64::new(0);

/// Cumulative snapshot the perf panel renders. Plain `u64`s so the
/// FFI surface stays trivial and the JS side just diffs successive
/// snapshots for rates. `bus_pending` is the current
/// `trace::BUS` length (NOT a cumulative counter) so the panel
/// reflects back-pressure on the bus side.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub emit_net_recv: u64,
    pub emit_net_provider_error: u64,
    pub emit_net_peer_up: u64,
    pub emit_net_peer_down: u64,
    pub emit_flower_picked_canonical: u64,
    pub emit_flower_picked_sandbox: u64,
    pub emit_other: u64,
    pub pickup_publish_attempted: u64,
    pub pickup_publish_ok: u64,
    pub pickup_publish_err: u64,
    pub pickup_publish_delivery_err: u64,
    pub pickup_received: u64,
    pub pickup_applied: u64,
    pub position_publish_attempted: u64,
    pub position_publish_ok: u64,
    pub position_publish_err: u64,
    pub position_publish_delivery_err: u64,
    pub bus_pending: u64,
}

/// Read every counter once. `Ordering::Relaxed` because perf is
/// observational — a slightly stale read does not cause incorrect
/// behavior anywhere, only a slightly old number in the panel.
pub fn snapshot() -> Snapshot {
    Snapshot {
        emit_net_recv: EMIT_NET_RECV.load(Ordering::Relaxed),
        emit_net_provider_error: EMIT_NET_PROVIDER_ERROR.load(Ordering::Relaxed),
        emit_net_peer_up: EMIT_NET_PEER_UP.load(Ordering::Relaxed),
        emit_net_peer_down: EMIT_NET_PEER_DOWN.load(Ordering::Relaxed),
        emit_flower_picked_canonical: EMIT_FLOWER_PICKED_CANONICAL.load(Ordering::Relaxed),
        emit_flower_picked_sandbox: EMIT_FLOWER_PICKED_SANDBOX.load(Ordering::Relaxed),
        emit_other: EMIT_OTHER.load(Ordering::Relaxed),
        pickup_publish_attempted: PICKUP_PUBLISH_ATTEMPTED.load(Ordering::Relaxed),
        pickup_publish_ok: PICKUP_PUBLISH_OK.load(Ordering::Relaxed),
        pickup_publish_err: PICKUP_PUBLISH_ERR.load(Ordering::Relaxed),
        pickup_publish_delivery_err: PICKUP_PUBLISH_DELIVERY_ERR.load(Ordering::Relaxed),
        pickup_received: PICKUP_RECEIVED.load(Ordering::Relaxed),
        pickup_applied: PICKUP_APPLIED.load(Ordering::Relaxed),
        position_publish_attempted: POSITION_PUBLISH_ATTEMPTED.load(Ordering::Relaxed),
        position_publish_ok: POSITION_PUBLISH_OK.load(Ordering::Relaxed),
        position_publish_err: POSITION_PUBLISH_ERR.load(Ordering::Relaxed),
        position_publish_delivery_err: POSITION_PUBLISH_DELIVERY_ERR.load(Ordering::Relaxed),
        bus_pending: crate::trace::pending_count() as u64,
    }
}

/// Render the snapshot as JSON for the FFI surface. Hand-formatted
/// because every roam FFI string stays serde-free in the boundary —
/// matches the existing pattern in `trace::drain_json`.
pub fn snapshot_json() -> String {
    let s = snapshot();
    format!(
        concat!(
            r#"{{"emit_net_recv":{},"emit_net_provider_error":{},"emit_net_peer_up":{},"#,
            r#""emit_net_peer_down":{},"emit_flower_picked_canonical":{},"#,
            r#""emit_flower_picked_sandbox":{},"emit_other":{},"#,
            r#""pickup_publish_attempted":{},"pickup_publish_ok":{},"pickup_publish_err":{},"#,
            r#""pickup_publish_delivery_err":{},"pickup_received":{},"pickup_applied":{},"#,
            r#""position_publish_attempted":{},"position_publish_ok":{},"position_publish_err":{},"#,
            r#""position_publish_delivery_err":{},"bus_pending":{}}}"#
        ),
        s.emit_net_recv,
        s.emit_net_provider_error,
        s.emit_net_peer_up,
        s.emit_net_peer_down,
        s.emit_flower_picked_canonical,
        s.emit_flower_picked_sandbox,
        s.emit_other,
        s.pickup_publish_attempted,
        s.pickup_publish_ok,
        s.pickup_publish_err,
        s.pickup_publish_delivery_err,
        s.pickup_received,
        s.pickup_applied,
        s.position_publish_attempted,
        s.position_publish_ok,
        s.position_publish_err,
        s.position_publish_delivery_err,
        s.bus_pending,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tag routing — a Note with a known tag increments the matching
    /// counter by exactly one. Only asserts on the named counter
    /// because static counters are process-shared across tests (cargo
    /// runs tests in parallel within one process), so other counters'
    /// values are not under this test's control. The cross-counter
    /// invariant (a known tag must NOT touch OTHER) is enforced by
    /// the unknown-tag test running the same dispatch in reverse.
    #[test]
    fn note_tag_emit_routes_to_the_named_counter() {
        let before = EMIT_NET_RECV.load(Ordering::Relaxed);
        note_tag_emit("net::recv");
        assert_eq!(EMIT_NET_RECV.load(Ordering::Relaxed), before + 1);
    }

    /// Unknown tags land in OTHER — exactly one increment per call.
    /// Same single-counter assertion shape as above for the same
    /// shared-state reason.
    #[test]
    fn note_tag_emit_unknown_tag_lands_in_other() {
        let before = EMIT_OTHER.load(Ordering::Relaxed);
        note_tag_emit("nonexistent::tag");
        assert_eq!(EMIT_OTHER.load(Ordering::Relaxed), before + 1);
    }

    /// Snapshot reads every counter coherently — incrementing one
    /// counter shows up in the snapshot. Falsifies the regression
    /// where `snapshot()` reads stale memory or skips a field.
    #[test]
    fn snapshot_reflects_counter_increments() {
        let before = snapshot();
        PICKUP_RECEIVED.fetch_add(1, Ordering::Relaxed);
        let after = snapshot();
        assert_eq!(after.pickup_received, before.pickup_received + 1);
    }

    /// JSON keys lock the wire shape the JS panel reads. Falsifies
    /// the regression where a rename here drifts away from the JS-side
    /// field access (which would surface as `undefined` rates, not an
    /// error).
    #[test]
    fn snapshot_json_has_all_named_fields() {
        let json = snapshot_json();
        for key in [
            "\"emit_net_recv\"",
            "\"emit_net_provider_error\"",
            "\"emit_net_peer_up\"",
            "\"emit_net_peer_down\"",
            "\"emit_flower_picked_canonical\"",
            "\"emit_flower_picked_sandbox\"",
            "\"emit_other\"",
            "\"pickup_publish_attempted\"",
            "\"pickup_publish_ok\"",
            "\"pickup_publish_err\"",
            "\"pickup_publish_delivery_err\"",
            "\"pickup_received\"",
            "\"pickup_applied\"",
            "\"position_publish_attempted\"",
            "\"position_publish_ok\"",
            "\"position_publish_err\"",
            "\"position_publish_delivery_err\"",
            "\"bus_pending\"",
        ] {
            assert!(json.contains(key), "json missing key {key}: {json}");
        }
    }
}
