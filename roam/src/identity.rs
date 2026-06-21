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

/// W3C `did:key` Ed25519 multicodec varint prefix.
/// Two bytes: `0xed` (low 7 bits of the multicodec code 0xed +
/// continuation bit), `0x01` (remaining high bits, no continuation).
/// Together they identify the payload that follows as a raw
/// Ed25519-public-key per the multicodec table.
const ED25519_DID_KEY_MULTICODEC: [u8; 2] = [0xed, 0x01];

/// Encode a 32-byte Ed25519 public key as a `did:key` URI per the
/// W3C did:key spec. The encoding is:
///   `did:key:` + `z` (multibase prefix for base58btc) +
///   `base58btc(ed25519_multicodec_varint || pubkey_bytes)`.
///
/// The resulting string always starts with the literal prefix
/// `did:key:z6Mk` for Ed25519 keys — that prefix is the deterministic
/// base58btc encoding of the multicodec varint `0xed 0x01` followed
/// by the leading bits of any 32-byte pubkey. It's the W3C-documented
/// identifier for "this DID uses an Ed25519 public key."
///
/// Falsifiable via `ed25519_did_key_round_trip` below — encoding then
/// decoding yields the original 32 bytes.
pub fn ed25519_pubkey_to_did_key(pubkey: &[u8; 32]) -> String {
    let mut payload = Vec::with_capacity(2 + 32);
    payload.extend_from_slice(&ED25519_DID_KEY_MULTICODEC);
    payload.extend_from_slice(pubkey);
    let encoded = bs58::encode(payload).into_string();
    format!("did:key:z{encoded}")
}

/// Errors decoding a `did:key` back to its Ed25519 pubkey. Each
/// variant pins one specific failure mode so the call site can give
/// the user actionable text rather than a flattened "invalid DID."
#[derive(Debug, PartialEq, Eq)]
pub enum DidKeyError {
    /// String didn't start with `did:key:z` — not a base58btc-encoded
    /// did:key URI.
    MissingPrefix,
    /// The base58btc payload couldn't be decoded.
    Base58(String),
    /// Decoded payload was the wrong length for an Ed25519 did:key
    /// (must be exactly 2 bytes multicodec + 32 bytes pubkey).
    UnexpectedPayloadLength(usize),
    /// Multicodec prefix wasn't the Ed25519 marker `0xed 0x01`.
    NotEd25519,
}

/// Decode a `did:key:z6Mk…` string back to its raw 32-byte Ed25519
/// public key. Inverse of `ed25519_pubkey_to_did_key`.
pub fn did_key_to_ed25519_pubkey(did: &str) -> Result<[u8; 32], DidKeyError> {
    let body = did
        .strip_prefix("did:key:z")
        .ok_or(DidKeyError::MissingPrefix)?;
    let payload = bs58::decode(body)
        .into_vec()
        .map_err(|e| DidKeyError::Base58(e.to_string()))?;
    if payload.len() != 2 + 32 {
        return Err(DidKeyError::UnexpectedPayloadLength(payload.len()));
    }
    if payload[..2] != ED25519_DID_KEY_MULTICODEC {
        return Err(DidKeyError::NotEd25519);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&payload[2..]);
    Ok(out)
}

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

    /// M1 — encoding shape pinned. Any Ed25519 pubkey through
    /// `ed25519_pubkey_to_did_key` produces the `did:key:z6Mk…`
    /// prefix. The `z6Mk` portion is deterministic: it's the
    /// base58btc encoding of the multicodec varint `0xed 0x01` plus
    /// the leading bits of any 32-byte pubkey. If the multicodec
    /// prefix or the multibase encoding ever changes, this trips.
    #[test]
    fn ed25519_did_key_has_z6mk_prefix() {
        let pubkey = [0u8; 32];
        let did = ed25519_pubkey_to_did_key(&pubkey);
        assert!(
            did.starts_with("did:key:z6Mk"),
            "expected did:key:z6Mk… prefix, got {did}"
        );
    }

    /// M1 — round-trip. Encoding and then decoding any 32-byte pubkey
    /// yields the original 32 bytes. This is the falsifiable check
    /// that the encoder + decoder are exact inverses; mutate either
    /// side and this fails immediately.
    #[test]
    fn ed25519_did_key_round_trip() {
        let pubkey = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let did = ed25519_pubkey_to_did_key(&pubkey);
        let decoded = did_key_to_ed25519_pubkey(&did).expect("round-trip decode");
        assert_eq!(decoded, pubkey);
    }

    /// M1 — distinct pubkeys produce distinct DIDs. Catches a
    /// regression where the encoder ignores its input and emits a
    /// fixed string.
    #[test]
    fn ed25519_distinct_pubkeys_produce_distinct_did_keys() {
        let a = [0u8; 32];
        let b = [1u8; 32];
        assert_ne!(
            ed25519_pubkey_to_did_key(&a),
            ed25519_pubkey_to_did_key(&b),
        );
    }

    /// M1 — decode rejects strings without the `did:key:z` prefix.
    #[test]
    fn did_key_decode_rejects_missing_prefix() {
        assert_eq!(
            did_key_to_ed25519_pubkey("z6MkSomething"),
            Err(DidKeyError::MissingPrefix),
        );
    }

    /// M1 — decode rejects strings whose multicodec prefix isn't
    /// `0xed 0x01`. Constructed by encoding a wrong multicodec
    /// prefix (`0x12 0x00`, the multihash for sha2-256) followed
    /// by 32 bytes; the result is a syntactically-valid did:key URI
    /// that names a different key type.
    #[test]
    fn did_key_decode_rejects_non_ed25519_multicodec() {
        let mut payload = vec![0x12, 0x00];
        payload.extend_from_slice(&[0u8; 32]);
        let bad = format!("did:key:z{}", bs58::encode(payload).into_string());
        assert_eq!(did_key_to_ed25519_pubkey(&bad), Err(DidKeyError::NotEd25519));
    }
}
