//! Identity helpers for the libp2p slice.
//!
//! Ed25519 keypair management — generate fresh on first visit, restore
//! from libp2p-canonical protobuf bytes on subsequent visits. PeerId
//! is derived from the same key. Slice B intentionally omits did:key
//! surfacing; tighten later without changing call sites.

#[cfg(target_arch = "wasm32")]
pub use real::{generate_identity_protobuf, load_or_generate_keypair};

#[cfg(target_arch = "wasm32")]
mod real {
    use crate::net::NetError;
    use libp2p::identity;

    /// `bytes` carries the libp2p-canonical protobuf-encoded keypair
    /// the JS bridge loaded from IndexedDB. `None` → generate fresh
    /// (the bridge persists the bytes after this call returns so the
    /// next session restores them). Decode failure surfaces as an
    /// error — refusing to fall through to "generate fresh" is
    /// deliberate so a corrupt stored identity is visible, not a
    /// silent PeerId rotation behind the user's back.
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
    /// protobuf encoding. JS bridge calls this on first visit when
    /// IndexedDB has no `rave/identity/v1` entry; the bridge stores
    /// the bytes and feeds them back to `load_or_generate_keypair`
    /// next session so PeerId stays stable.
    pub fn generate_identity_protobuf() -> Result<Vec<u8>, NetError> {
        identity::Keypair::generate_ed25519()
            .to_protobuf_encoding()
            .map_err(|e| NetError::ProviderInternal {
                reason: format!("identity encode: {e}"),
            })
    }
}

#[cfg(target_arch = "wasm32")]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::NetError;
    use libp2p::PeerId;

    #[test]
    fn fresh_keypair_round_trips_via_protobuf() {
        let kp1 = load_or_generate_keypair(None).expect("fresh generate");
        let bytes = kp1.to_protobuf_encoding().expect("encode");
        let kp2 = load_or_generate_keypair(Some(&bytes)).expect("restore");
        assert_eq!(PeerId::from(kp1.public()), PeerId::from(kp2.public()));
    }

    #[test]
    fn generate_identity_protobuf_returns_decodable_bytes() {
        let bytes = generate_identity_protobuf().expect("generate");
        let kp = load_or_generate_keypair(Some(&bytes)).expect("restore");
        let pub_bytes = kp
            .public()
            .try_into_ed25519()
            .expect("ed25519")
            .to_bytes();
        assert_eq!(pub_bytes.len(), 32);
    }

    #[test]
    fn corrupt_bytes_surface_as_error() {
        let result = load_or_generate_keypair(Some(&[0xFF; 8]));
        assert!(matches!(result, Err(NetError::ProviderInternal { .. })));
    }
}
