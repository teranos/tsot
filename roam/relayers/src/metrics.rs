//! CloudWatch metric publishing — parity with the deleted TS relay's set
//! lines 264-334.
//!
//! Six metrics, every 60s, namespace `CWAgent`, dimension
//! `InstanceName=roam-relay-eu-2` (env-overridable). Existing
//! CloudWatch alarms in `infra/observability.tf` match these
//! names + dimensions exactly — diverge and you silently break
//! the on-call paging path.
//!
//! `mem_used_percent` denominator is the systemd `MemoryMax` cap
//! (default 400 MiB), NOT the box's physical RAM. This is
//! deliberate: the 80%-threshold alarm has to fire BEFORE
//! systemd's OOM kill, not after.
//!
//! Failure mode: PutMetricData errors are surfaced once on the
//! first failure and every 60th failure after that, so the
//! journal doesn't get flooded by the same error every minute.
//! The relayer itself does not crash on a metrics hiccup.

use std::time::Duration;

use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

/// Names + dimension shape match what the TS relay published.
pub const NAMESPACE_DEFAULT: &str = "CWAgent";
pub const INSTANCE_DEFAULT: &str = "roam-relay-eu-2";
pub const INTERVAL_DEFAULT: Duration = Duration::from_secs(60);
pub const MEMORY_CAP_MB_DEFAULT: u64 = 400;

/// One observation tick — peer/connection/msg counts the swarm
/// produced this interval, plus process memory snapshot.
pub struct Snapshot {
    pub peers: u64,
    pub conns: u64,
    /// Pubsub messages received in the last interval (NOT total).
    pub pubsub_msgs_per_sec: f64,
    /// Process RSS in bytes. Read from /proc/self/statm on Linux.
    pub mem_rss_bytes: u64,
    /// Process VMS (virtual memory size) in bytes. The TS relay
    /// reports V8's `heapTotal` here; Rust has no managed heap so
    /// we report VmSize from /proc/self/statm instead. Same
    /// metric name, same axis (bytes), different denominator —
    /// alarms on this metric should already key on absolute
    /// bytes, not "heap headroom."
    pub mem_vms_bytes: u64,
}

/// Read /proc/self/statm: page counts for {size, resident, ...}.
/// Linux-only; the deployed box is Ubuntu 24.04 x86_64. On macOS
/// returns zeros so `cargo test` runs without panicking — metrics
/// would just publish 0 if you ever ran the binary on a Mac,
/// which you wouldn't.
pub fn read_proc_memory() -> (u64, u64) {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return (0, 0);
    };
    let mut parts = statm.split_whitespace();
    let vms_pages: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let rss_pages: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let page_size: u64 = 4096; // standard on x86_64 Linux
    (rss_pages * page_size, vms_pages * page_size)
}

/// Build the 6 MetricDatum values matching the deleted TS relay's set.
/// Pure function — takes the snapshot, the instance dimension,
/// and the cap; returns the data ready to send. Testable.
pub fn build_metric_data(
    snap: &Snapshot,
    instance_name: &str,
    memory_cap_mb: u64,
) -> Vec<MetricDatum> {
    let dimension = Dimension::builder()
        .name("InstanceName")
        .value(instance_name)
        .build();

    let mem_cap_bytes = memory_cap_mb * 1024 * 1024;
    let mem_pct = if mem_cap_bytes > 0 {
        (snap.mem_rss_bytes as f64 / mem_cap_bytes as f64) * 100.0
    } else {
        0.0
    };

    let make = |name: &str, value: f64, unit: StandardUnit| {
        MetricDatum::builder()
            .metric_name(name)
            .dimensions(dimension.clone())
            .value(value)
            .unit(unit)
            .build()
    };

    vec![
        make("procstat_memory_rss", snap.mem_rss_bytes as f64, StandardUnit::Bytes),
        make("procstat_memory_vms", snap.mem_vms_bytes as f64, StandardUnit::Bytes),
        make("mem_used_percent", mem_pct, StandardUnit::Percent),
        make("relay_peer_count", snap.peers as f64, StandardUnit::Count),
        make("relay_connection_count", snap.conns as f64, StandardUnit::Count),
        make("relay_pubsub_msgs_per_sec", snap.pubsub_msgs_per_sec, StandardUnit::CountSecond),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 6 metric names must match the TS relay's exactly —
    /// existing CloudWatch alarms in infra/observability.tf key
    /// on these strings. Diverge and on-call paging silently
    /// breaks.
    #[test]
    fn metric_names_match_ts_relay_exactly() {
        let snap = Snapshot {
            peers: 0,
            conns: 0,
            pubsub_msgs_per_sec: 0.0,
            mem_rss_bytes: 0,
            mem_vms_bytes: 0,
        };
        let data = build_metric_data(&snap, "test", 400);
        let names: Vec<&str> = data
            .iter()
            .map(|d| d.metric_name().expect("metric name set"))
            .collect();

        assert_eq!(
            names,
            vec![
                "procstat_memory_rss",
                "procstat_memory_vms",
                "mem_used_percent",
                "relay_peer_count",
                "relay_connection_count",
                "relay_pubsub_msgs_per_sec",
            ],
            "metric name + order must match the deleted TS relay's set exactly"
        );
    }

    /// Dimension shape must be `[{Name: 'InstanceName', Value: <instance>}]`,
    /// matching what the deleted TS relay published.
    #[test]
    fn dimension_shape_matches_ts_relay() {
        let snap = Snapshot {
            peers: 0,
            conns: 0,
            pubsub_msgs_per_sec: 0.0,
            mem_rss_bytes: 0,
            mem_vms_bytes: 0,
        };
        let data = build_metric_data(&snap, "roam-relay-eu-2", 400);
        let dims = data[0].dimensions();

        assert_eq!(dims.len(), 1, "exactly one dimension");
        assert_eq!(
            dims[0].name().expect("dimension name set"),
            "InstanceName"
        );
        assert_eq!(
            dims[0].value().expect("dimension value set"),
            "roam-relay-eu-2"
        );
    }

    /// `mem_used_percent` denominator must be the systemd cap, not the
    /// box's RAM — encoded here as a test so the next developer can't
    /// accidentally swap it.
    #[test]
    fn mem_used_percent_uses_cap_not_box_ram() {
        // 200 MiB RSS, 400 MiB cap → 50%
        let snap = Snapshot {
            peers: 0,
            conns: 0,
            pubsub_msgs_per_sec: 0.0,
            mem_rss_bytes: 200 * 1024 * 1024,
            mem_vms_bytes: 0,
        };
        let data = build_metric_data(&snap, "test", 400);
        let pct_datum = data
            .iter()
            .find(|d| d.metric_name() == Some("mem_used_percent"))
            .expect("mem_used_percent metric present");

        let pct = pct_datum.value().expect("value set");
        assert!(
            (pct - 50.0).abs() < 0.001,
            "200 MiB RSS against 400 MiB cap must be 50%, got {pct}"
        );
    }

    /// Cap of 0 must produce 0%, not NaN or infinity. Edge case
    /// — if the env var is misconfigured, we'd rather publish 0
    /// than divide by zero.
    #[test]
    fn mem_cap_zero_produces_zero_percent() {
        let snap = Snapshot {
            peers: 0,
            conns: 0,
            pubsub_msgs_per_sec: 0.0,
            mem_rss_bytes: 100 * 1024 * 1024,
            mem_vms_bytes: 0,
        };
        let data = build_metric_data(&snap, "test", 0);
        let pct = data
            .iter()
            .find(|d| d.metric_name() == Some("mem_used_percent"))
            .and_then(|d| d.value())
            .expect("mem_used_percent value");

        assert_eq!(pct, 0.0, "cap=0 must produce 0% not NaN/inf");
    }
}
