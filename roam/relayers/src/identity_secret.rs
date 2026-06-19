//! Ed25519 identity loading for the relayer.
//!
//! Split out of `main.rs` so the decode path is testable without
//! AWS. The contract is "given bytes that came from the Secrets
//! Manager secret value, return the libp2p Keypair they encode,
//! or fail loud with a context that names what to do next."
//!
//! TDD note (relayers.md, Q2): the *real* parity check is "does
//! the existing deployed secret decode" — that's V3 in the doc
//! and requires either the actual secret bytes or a fixture
//! extracted from it. The unit tests here exercise the function
//! against bytes the libp2p crate itself produces. Round-trip
//! confidence, not deployed-secret confidence. When V3 lands,
//! add a fixture-driven test alongside.

use libp2p::identity::Keypair;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("secret bytes are not a libp2p canonical protobuf keypair (see roam/relay/relayers.md Q2): {source}")]
    NotCanonicalProtobuf {
        #[source]
        source: libp2p::identity::DecodingError,
    },
    #[error("secret string is not valid base64: invalid character 0x{0:02x} at position {1}")]
    InvalidBase64Char(u8, usize),
    #[error("secret string has wrong base64 length: {0} characters (must be a multiple of 4)")]
    InvalidBase64Length(usize),
}

/// Decode the libp2p protobuf-encoded keypair the Secrets Manager
/// secret holds. Refuses to fall through to "generate a fresh
/// keypair" on failure — silently rotating the PeerId would break
/// every browser's known bootstrap address.
///
/// Handles two on-the-wire shapes:
///
/// 1. **Canonical**: rust-libp2p's full protobuf encoding. Tried
///    first via `Keypair::from_protobuf_encoding`.
/// 2. **js-libp2p Ed25519 raw**: the deployed secret format —
///    `08 01 12 40 <64 bytes>` = `PrivateKey { type: Ed25519,
///    data: <secret_32 || public_32> }`, where the 64 data bytes
///    are the raw concatenated keypair (not a further nested
///    protobuf). rust-libp2p 0.56's strict canonical decoder
///    rejects this shape because it tries to parse the data field
///    as another protobuf and quick-protobuf chokes on a random
///    byte that looks like wire-type-3 ("group"). The 4-byte
///    envelope is unambiguous; we strip it and pass the 64 raw
///    bytes to `Keypair::ed25519_from_bytes`, which accepts the
///    secret+public concatenation directly.
///
/// PeerId is preserved across both paths because it derives from
/// the public key bits, which are the same regardless of envelope.
pub fn decode_identity(bytes: &[u8]) -> Result<Keypair, IdentityError> {
    // Shape 1: canonical rust-libp2p protobuf.
    if let Ok(kp) = Keypair::from_protobuf_encoding(bytes) {
        return Ok(kp);
    }

    // Shape 2: js-libp2p Ed25519 envelope with 64 raw bytes.
    // 0x08 0x01 = field 1 (KeyType), wire-type 0 (varint), value 1 (Ed25519)
    // 0x12 0x40 = field 2 (Data), wire-type 2 (length-delimited), length 64
    const JS_LIBP2P_ED25519_ENVELOPE: [u8; 4] = [0x08, 0x01, 0x12, 0x40];
    if bytes.len() == 68 && bytes.starts_with(&JS_LIBP2P_ED25519_ENVELOPE) {
        let mut raw = bytes[4..68].to_vec();
        if let Ok(kp) = Keypair::ed25519_from_bytes(&mut raw) {
            return Ok(kp);
        }
    }

    // Both paths failed — surface the canonical decode's typed error
    // (it's the most informative one for the operator).
    Keypair::from_protobuf_encoding(bytes)
        .map_err(|source| IdentityError::NotCanonicalProtobuf { source })
}

/// Decode RFC 4648 standard base64 (the format AWS Secrets Manager
/// uses when you paste binary bytes into the SecretString field —
/// the deployed `roam/relay/identity` secret is exactly this shape
/// per the Q3 verification: 92 base64 chars → 68 raw protobuf bytes
/// starting with `08 01 12 40 …` = KeyType=Ed25519 + Data length 64).
/// Written inline rather than adding a base64 crate dep so the
/// relayer's build surface stays minimal.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, IdentityError> {
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|b| !matches!(b, b'\n' | b'\r' | b' ' | b'\t'))
        .collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(IdentityError::InvalidBase64Length(cleaned.len()));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    for (chunk_idx, chunk) in cleaned.chunks(4).enumerate() {
        let base = chunk_idx * 4;
        let v0 = b64_val(chunk[0], base)?;
        let v1 = b64_val(chunk[1], base + 1)?;
        let v2_opt = if chunk[2] == b'=' { None } else { Some(b64_val(chunk[2], base + 2)?) };
        let v3_opt = if chunk[3] == b'=' { None } else { Some(b64_val(chunk[3], base + 3)?) };
        out.push((v0 << 2) | (v1 >> 4));
        if let Some(v2) = v2_opt {
            out.push(((v1 & 0x0f) << 4) | (v2 >> 2));
            if let Some(v3) = v3_opt {
                out.push(((v2 & 0x03) << 6) | v3);
            }
        }
    }
    Ok(out)
}

fn b64_val(c: u8, pos: usize) -> Result<u8, IdentityError> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(IdentityError::InvalidBase64Char(c, pos)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;

    /// Round-trip: a freshly-generated Ed25519 keypair, encoded
    /// to canonical protobuf, decoded back, produces the same
    /// PeerId. This pins that `decode_identity` calls the right
    /// libp2p API and doesn't accidentally pick a different
    /// encoding format (the libp2p crate has several — only
    /// `from_protobuf_encoding` matches what the secret holds).
    #[test]
    fn decode_identity_round_trips_generated_ed25519() {
        let original = Keypair::generate_ed25519();
        let expected_peer_id = PeerId::from(original.public());
        let bytes = original
            .to_protobuf_encoding()
            .expect("to_protobuf_encoding on a freshly-generated keypair must succeed");

        let decoded = decode_identity(&bytes).expect("round-trip decode must succeed");
        let decoded_peer_id = PeerId::from(decoded.public());

        assert_eq!(
            decoded_peer_id, expected_peer_id,
            "decoded PeerId must match the originally-generated one — \
             if this ever fails, the relayer would silently change its \
             bootstrap address on restart"
        );
    }

    /// Garbage bytes must fail loud with the typed error variant.
    /// The error message must mention the doc reference (Q2) so
    /// an operator hitting this in production has a clear pointer
    /// to the recovery path.
    #[test]
    fn decode_identity_rejects_garbage_bytes() {
        let garbage = b"this is not a libp2p protobuf keypair, it is words";

        let err = decode_identity(garbage).expect_err("garbage bytes must not decode");

        let msg = format!("{err}");
        assert!(
            msg.contains("Q2") || msg.contains("relayers.md"),
            "error message must point to the doc; got: {msg}"
        );
    }

    /// Empty bytes must also fail loud — the secret could be
    /// empty for a real reason (just-created secret, broken
    /// rotation), and the relayer must refuse to start rather
    /// than fall through to a fresh keypair.
    #[test]
    fn decode_identity_rejects_empty_bytes() {
        let empty: &[u8] = &[];

        let err = decode_identity(empty).expect_err("empty bytes must not decode");

        let msg = format!("{err}");
        assert!(
            msg.contains("Q2") || msg.contains("relayers.md"),
            "error message must point to the doc; got: {msg}"
        );
    }

    /// The header bytes the deployed secret starts with —
    /// `08 01 12 40` = libp2p protobuf KeyType=Ed25519 + Data
    /// length 64. If the base64 decoder ever stops producing
    /// these for the canonical encoding "CAESQA==", the deployed
    /// secret won't load. Pin it.
    #[test]
    fn decode_base64_produces_libp2p_protobuf_header() {
        let header = decode_base64("CAESQA==").expect("CAESQA== must decode");
        assert_eq!(
            header,
            vec![0x08, 0x01, 0x12, 0x40],
            "the libp2p canonical Ed25519 protobuf header must decode bit-exact"
        );
    }

    /// Round-trip a longer sequence — 12 ASCII bytes (16 chars
    /// base64, no padding) — to catch off-by-one bugs in the
    /// chunk loop or the bit-shift order. If the decoder gets
    /// any byte wrong the keypair load will fail later in a
    /// confusing way; better to fail here.
    #[test]
    fn decode_base64_round_trips_known_payload() {
        // "Hello World!" — 12 bytes, 16 base64 chars, no padding.
        let decoded = decode_base64("SGVsbG8gV29ybGQh").expect("must decode");
        assert_eq!(decoded, b"Hello World!".to_vec());
    }

    /// Invalid chars must fail loud with position info so a
    /// future operator can see exactly where the secret value
    /// got corrupted.
    #[test]
    fn decode_base64_rejects_invalid_char() {
        let err = decode_base64("AAA!").expect_err("invalid char must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("position 3") || msg.contains("0x21"),
            "error must point to the bad character; got: {msg}"
        );
    }

    /// Non-multiple-of-4 length must fail. base64 strings are
    /// always padded to a multiple of 4; anything else is
    /// truncated input.
    #[test]
    fn decode_base64_rejects_wrong_length() {
        let err = decode_base64("ABC").expect_err("3 chars must fail");
        let msg = format!("{err}");
        assert!(msg.contains("length") || msg.contains("multiple of 4"), "got: {msg}");
    }

    /// The js-libp2p Ed25519 envelope fallback path — the shape
    /// the deployed `roam/relay/identity` secret actually uses.
    /// Synthesize the envelope from a freshly-generated rust-
    /// libp2p Keypair (whose 64-byte raw form is accessible via
    /// `into_ed25519().to_bytes()`), wrap with `08 01 12 40`,
    /// decode through the new fallback, and assert the PeerId
    /// round-trips. If this ever fails, the deployed secret
    /// won't load and `relay.sbvh.nl` browsers lose their known
    /// bootstrap address.
    #[test]
    fn decode_identity_handles_js_libp2p_envelope_with_raw_64_bytes() {
        let original = Keypair::generate_ed25519();
        let expected_peer_id = PeerId::from(original.public());

        let ed25519_kp = original
            .try_into_ed25519()
            .expect("generated as Ed25519 — must extract as Ed25519");
        let raw_64 = ed25519_kp.to_bytes();
        assert_eq!(raw_64.len(), 64, "Ed25519 raw form must be 64 bytes (secret + public)");

        let mut envelope = vec![0x08, 0x01, 0x12, 0x40];
        envelope.extend_from_slice(&raw_64);
        assert_eq!(envelope.len(), 68, "envelope must be 4 header + 64 data");

        let decoded = decode_identity(&envelope).expect(
            "js-libp2p envelope with raw 64-byte Ed25519 must decode via the fallback path",
        );
        let decoded_peer_id = PeerId::from(decoded.public());

        assert_eq!(
            decoded_peer_id, expected_peer_id,
            "PeerId must be preserved across the js-libp2p envelope fallback path — \
             if this fails, the deployed Secrets Manager identity won't decode and \
             every browser's known bootstrap address breaks at the cut-over"
        );
    }
}
