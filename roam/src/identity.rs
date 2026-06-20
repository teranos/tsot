//! Identity primitives and the M4 predicate.
//!
//! M4 — runtime classification of "identified" for `CANONICAL.md`.
//! Downstream callers (M6 world-state routing, M7 promotion flow,
//! v0.4 multiplayer pickup write-propagation) read identity
//! classification via `is_identified_self` / `is_identified_peer`.
//! Today's rule is the cheapest defensible one: non-empty persistent
//! bytes on self, "any peer we've heard from" on the peer side. The
//! 0.3.2 identity hard-fail (`assets/src/identity.js` +
//! `assets/src/js-bridge.js`) makes the self rule strictly stronger
//! than its surface reads — the network refuses to start without
//! persistent bytes. Future stack picks (did:key, UCAN, WebAuthn,
//! ATProto) tighten the implementation without changing signatures
//! or call sites.
//!
//! Keypair handling (`load_or_generate_keypair`, `generate_identity_protobuf`)
//! consolidated here from `net::rust_libp2p` per IDENTITY.md C3. The
//! JS bridge calls `roam_net_generate_identity_bytes` on first visit
//! to mint a fresh Ed25519 keypair, persists the protobuf-encoded
//! bytes in IndexedDB, and passes them back on every subsequent
//! `roam_net_worker_provider_init`.

use crate::net::PeerId;

#[cfg(target_arch = "wasm32")]
mod keypair {
    use crate::net::NetError;
    use libp2p::identity;

    /// Decode the libp2p-canonical protobuf-encoded keypair the JS
    /// bridge loaded from IndexedDB. `None` → generate fresh (the
    /// bridge will persist the bytes after this call returns so the
    /// next session loads them back). Refusing to fall through to
    /// "generate fresh" on a decode failure is deliberate: a corrupt
    /// stored identity surfaces as an error, not a silent PeerId
    /// rotation behind the user's back.
    pub fn load_or_generate_keypair(
        bytes: Option<&[u8]>,
    ) -> Result<identity::Keypair, NetError> {
        match bytes {
            Some(b) => identity::Keypair::from_protobuf_encoding(b).map_err(|e| {
                NetError::ProviderInternal {
                    reason: format!("identity decode: {e}"),
                }
            }),
            None => Ok(identity::Keypair::generate_ed25519()),
        }
    }

    /// Mint a fresh Ed25519 keypair and return its libp2p-canonical
    /// protobuf encoding. Called by the JS bridge on first visit
    /// (when IndexedDB has no `roam/identity/v1` entry); the bridge
    /// stores the bytes and passes them back on every subsequent
    /// `roam_net_worker_provider_init` so PeerId stays stable across
    /// sessions.
    pub fn generate_identity_protobuf() -> Result<Vec<u8>, NetError> {
        identity::Keypair::generate_ed25519()
            .to_protobuf_encoding()
            .map_err(|e| NetError::ProviderInternal {
                reason: format!("identity encode: {e}"),
            })
    }
}

#[cfg(target_arch = "wasm32")]
pub use keypair::{generate_identity_protobuf, load_or_generate_keypair};

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
