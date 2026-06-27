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
    // RED: load_or_generate_keypair + generate_identity_protobuf are
    // referenced by the tests below but not yet defined. GREEN commit
    // adds the impl here.
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
