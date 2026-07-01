//! Durable health conditions — one row per active problem right now.
//!
//! Companion to the ErrorLog. Events say "what just happened"; Health
//! says "what is wrong right now." Every entry declares a trigger, a
//! resolution rule, and a key. Drift between the three is the failure
//! mode that makes Health worse than logs, not better — so we keep
//! them adjacent in code.
//!
//! Currently one condition, matching roam's Phase 4 first candidate:
//!
//! - `PublishFailing { topic }`
//!   - trigger: `NetEvent::Error(NetError::PublishFailed { topic, .. })`
//!   - resolution: `NetEvent::Message { topic, .. }` (any direction —
//!     one message on that topic proves the mesh works)
//!   - key: `topic` (upsert-against)
//!
//! Lives in its own crate so CI's Linux runner can `cargo test` this
//! logic without pulling rave's wasm-only Bevy renderer deps. The
//! `bevy_ecs` dep is only for the `Resource` derive — no winit, no
//! wgpu, no gpu backend.

use bevy_ecs::resource::Resource;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishFailingEntry {
    pub reason: String,
    pub first_at_ms: u64,
    pub last_at_ms: u64,
    pub occurrences: u32,
}

// BTreeMap keeps render order deterministic (topic-sorted) — matters
// for tests and for a stable panel when multiple topics fail at once.
#[derive(Resource, Default, Debug, Clone)]
pub struct Health {
    pub publish_failing: BTreeMap<String, PublishFailingEntry>,
}

impl Health {
    pub fn active_lines(&self, now_ms: u64) -> Vec<String> {
        self.publish_failing
            .iter()
            .map(|(topic, e)| {
                let age_s = now_ms.saturating_sub(e.first_at_ms) / 1000;
                format!(
                    "PublishFailing {topic} ×{} age={}s reason={}",
                    e.occurrences, age_s, e.reason
                )
            })
            .collect()
    }
}

pub fn record_publish_failed(health: &mut Health, topic: &str, reason: &str, now_ms: u64) {
    health
        .publish_failing
        .entry(topic.to_string())
        .and_modify(|e| {
            e.last_at_ms = now_ms;
            e.occurrences = e.occurrences.saturating_add(1);
            e.reason = reason.to_string();
        })
        .or_insert(PublishFailingEntry {
            reason: reason.to_string(),
            first_at_ms: now_ms,
            last_at_ms: now_ms,
            occurrences: 1,
        });
}

pub fn record_message_seen(health: &mut Health, topic: &str) {
    health.publish_failing.remove(topic);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_failed_creates_entry() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "rave-positions/v1", "NoPeersSubscribedToTopic", 1_000);
        assert_eq!(h.publish_failing.len(), 1);
        let e = &h.publish_failing["rave-positions/v1"];
        assert_eq!(e.occurrences, 1);
        assert_eq!(e.first_at_ms, 1_000);
        assert_eq!(e.last_at_ms, 1_000);
        assert_eq!(e.reason, "NoPeersSubscribedToTopic");
    }

    #[test]
    fn repeated_publish_failed_bumps_occurrences_and_last_at() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "t", "r", 1_000);
        record_publish_failed(&mut h, "t", "r", 1_100);
        record_publish_failed(&mut h, "t", "r", 1_200);
        assert_eq!(h.publish_failing["t"].occurrences, 3);
        assert_eq!(h.publish_failing["t"].first_at_ms, 1_000);
        assert_eq!(h.publish_failing["t"].last_at_ms, 1_200);
    }

    #[test]
    fn different_topics_get_separate_entries() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "a", "r", 100);
        record_publish_failed(&mut h, "b", "r", 200);
        assert_eq!(h.publish_failing.len(), 2);
    }

    #[test]
    fn message_on_same_topic_resolves() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "t", "r", 1_000);
        record_message_seen(&mut h, "t");
        assert!(h.publish_failing.is_empty());
    }

    #[test]
    fn message_on_other_topic_does_not_resolve() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "t", "r", 1_000);
        record_message_seen(&mut h, "other");
        assert_eq!(h.publish_failing.len(), 1);
    }

    #[test]
    fn reason_updates_to_latest_on_repeat() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "t", "NoPeersSubscribedToTopic", 1_000);
        record_publish_failed(&mut h, "t", "ProviderInternal", 1_100);
        assert_eq!(h.publish_failing["t"].reason, "ProviderInternal");
    }

    #[test]
    fn active_lines_renders_topic_sorted() {
        let mut h = Health::default();
        record_publish_failed(&mut h, "b", "no-peers", 500);
        record_publish_failed(&mut h, "a", "provider", 800);
        let lines = h.active_lines(1_500);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("PublishFailing a "));
        assert!(lines[1].starts_with("PublishFailing b "));
        assert!(lines[0].contains("×1"));
        assert!(lines[0].contains("age=0s"));
        assert!(lines[0].contains("reason=provider"));
        assert!(lines[1].contains("age=1s"));
    }

    #[test]
    fn active_lines_empty_when_no_conditions() {
        let h = Health::default();
        assert!(h.active_lines(1_000_000).is_empty());
    }
}
