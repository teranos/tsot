//! M4 — runtime predicate for what `CANONICAL.md` means by "identified."
//!
//! Downstream callers (M6 world-state routing, M7 promotion flow,
//! v0.4 multiplayer pickup write-propagation) read identity classification
//! from here. Today's rule is the cheapest defensible one: non-empty
//! persistent bytes on self, "any peer we've heard from" on the peer
//! side. The 0.3.2 identity hard-fail (`assets/src/identity.js` +
//! `assets/src/js-bridge.js`) makes the self rule strictly stronger
//! than its surface reads — the network refuses to start without
//! persistent bytes, so by the time anyone asks at runtime, the bytes
//! have already round-tripped through IndexedDB. Future identity-stack
//! picks (did:key, UCAN, WebAuthn, ATProto) tighten the implementation
//! without changing the signature or call sites.

use crate::net::PeerId;

/// Are we identified for canonical-world purposes? Today: non-empty
/// persistent identity bytes. Tightens later (did:key resolution,
/// UCAN proofs) without changing the signature.
pub fn is_identified_self(identity_bytes: Option<&[u8]>) -> bool {
    identity_bytes.is_some_and(|b| !b.is_empty())
}

/// Is this remote peer identified? Today: we've seen them at all.
/// Tightens later when M5 lands gossipsub signature verification at
/// the relayer — the rule will become "we've verified at least one
/// signed message from this PeerId."
pub fn is_identified_peer(seen_peers: &[PeerId], peer: &PeerId) -> bool {
    seen_peers.contains(peer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Self: non-empty persistent bytes → identified. The 0.3.2
    /// identity hard-fail guarantees this is the only branch reached
    /// in practice once the network is up.
    #[test]
    fn non_empty_identity_bytes_count_as_identified_self() {
        let bytes: Vec<u8> = (0..68).map(|i| i as u8).collect();
        assert!(is_identified_self(Some(&bytes)));
    }

    /// Self: empty bytes → NOT identified. Catches the regression
    /// where a future change collapses the predicate to `_ => true`.
    #[test]
    fn empty_identity_bytes_are_not_identified_self() {
        assert!(!is_identified_self(Some(&[])));
    }

    /// Self: missing bytes → NOT identified. Catches the regression
    /// where a future change ignores its input entirely.
    #[test]
    fn no_identity_bytes_are_not_identified_self() {
        assert!(!is_identified_self(None));
    }

    /// Peer: known peer → identified. Today's "seen at all" rule.
    #[test]
    fn known_peer_is_identified() {
        let alice = PeerId("12D3KooWAlice".into());
        let seen = vec![alice.clone()];
        assert!(is_identified_peer(&seen, &alice));
    }

    /// Peer: unknown peer → NOT identified. Falsifies the "always true"
    /// collapse; under the post-M5 rule this is also where unsigned
    /// or signature-failing peers will fall.
    #[test]
    fn unknown_peer_is_not_identified() {
        let alice = PeerId("12D3KooWAlice".into());
        let bob = PeerId("12D3KooWBob".into());
        let seen = vec![alice];
        assert!(!is_identified_peer(&seen, &bob));
    }

    /// Peer: empty seen-list → NOT identified. The boot state before
    /// any gossipsub message has been received; nobody is identified
    /// yet. Implicitly covered by `unknown_peer_is_not_identified` but
    /// worth pinning explicitly so a future refactor can't slip an
    /// "empty list means everyone" default past the suite.
    #[test]
    fn empty_seen_list_means_no_peer_is_identified() {
        let peer = PeerId("12D3KooWAnyone".into());
        assert!(!is_identified_peer(&[], &peer));
    }
}
