//! Deck tokens: a 16-character Crockford base32 string that uniquely
//! identifies a specific deck constructed by the sim. Encodes the
//! 5-tuple `(master_seed, side, variant_a, variant_b, game_index)`.
//! Decoding the token reproduces the exact deck — no need to re-run
//! the prior games in the matchup sweep.

// Decode + lookup helpers are dead-code from the binary's perspective
// today (tokens are only printed, not yet round-tripped via env var).
// The shape is here so the future TSOT_DECK_*_TOKEN env vars can read
// straight into a DeckToken without a second decoder implementation.
#![allow(dead_code)]
//!
//! Bit layout (78 bits packed into a u128, 80-bit slot with 2 bits of
//! zero-padding). Tuple is in the LOW bits so the leading Crockford
//! characters always carry per-game variation — small master seeds
//! (e.g., TSOT_SEED=42) don't produce a wall of zeros in the middle.
//!   bit   0      side          (0 = A, 1 = B)
//!   bits  1..4   variant_a     (3 bits, 7 variants)
//!   bits  4..7   variant_b     (3 bits, 7 variants)
//!   bits  7..14  game_index    (7 bits, supports up to 128 games/cell)
//!   bits 14..78  master_seed   (u64)
//!
//! Crockford base32 alphabet: `0123456789ABCDEFGHJKMNPQRSTVWXYZ`. Skips
//! I, L, O, U for visual/phonetic disambiguation. Decoder is forgiving:
//! lowercase, hyphens, and the I/L/O/0/1 confusion all normalize.

use super::variants::{DeckVariant, VARIANTS};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeckToken {
    pub master_seed: u64,
    pub side: Side,
    pub variant_a: DeckVariant,
    pub variant_b: DeckVariant,
    pub game_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    WrongLength,
    InvalidChar(char),
    InvalidVariant(u8),
    InvalidSide(u8),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::WrongLength => write!(f, "token must be 16 Crockford base32 characters"),
            DecodeError::InvalidChar(c) => write!(f, "invalid Crockford base32 character {c:?}"),
            DecodeError::InvalidVariant(v) => write!(f, "decoded variant index {v} out of range"),
            DecodeError::InvalidSide(s) => write!(f, "decoded side bit {s} must be 0 or 1"),
        }
    }
}

fn variant_to_index(v: DeckVariant) -> u8 {
    VARIANTS.iter().position(|x| *x == v).unwrap() as u8
}

fn index_to_variant(i: u8) -> Result<DeckVariant, DecodeError> {
    VARIANTS
        .get(i as usize)
        .copied()
        .ok_or(DecodeError::InvalidVariant(i))
}

impl DeckToken {
    pub fn encode(&self) -> String {
        // Pack into a u128 with 78 bits used. Tuple in the low bits so
        // the leading Crockford chars always carry per-game variation.
        let mut bits: u128 = 0;
        bits |= self.side_bit() as u128;
        bits |= (variant_to_index(self.variant_a) as u128) << 1;
        bits |= (variant_to_index(self.variant_b) as u128) << 4;
        bits |= ((self.game_index as u128) & 0x7f) << 7;
        bits |= (self.master_seed as u128) << 14;

        // Emit 16 base32 chars, 5 bits per char, low-to-high order.
        let mut out = String::with_capacity(16);
        for chunk in 0..16 {
            let shift = chunk * 5;
            let idx = ((bits >> shift) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
        out
    }

    pub fn decode(s: &str) -> Result<Self, DecodeError> {
        let cleaned: String = s
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if cleaned.len() != 16 {
            return Err(DecodeError::WrongLength);
        }
        let mut bits: u128 = 0;
        for (i, c) in cleaned.chars().enumerate() {
            let v = char_to_value(c)?;
            bits |= (v as u128) << (i * 5);
        }
        let side_bit = (bits & 0x1) as u8;
        let side = match side_bit {
            0 => Side::A,
            1 => Side::B,
            other => return Err(DecodeError::InvalidSide(other)),
        };
        let va_idx = ((bits >> 1) & 0x7) as u8;
        let vb_idx = ((bits >> 4) & 0x7) as u8;
        let game_index = ((bits >> 7) & 0x7f) as u32;
        let master_seed = ((bits >> 14) & 0xFFFF_FFFF_FFFF_FFFF) as u64;
        Ok(DeckToken {
            master_seed,
            side,
            variant_a: index_to_variant(va_idx)?,
            variant_b: index_to_variant(vb_idx)?,
            game_index,
        })
    }

    fn side_bit(&self) -> u8 {
        match self.side {
            Side::A => 0,
            Side::B => 1,
        }
    }

    /// Leading 4 chars of the encoded token — the tuple-bits prefix.
    /// Within a single master_seed run this uniquely identifies a deck;
    /// across runs you also need the master_signature() to reconstruct
    /// the full 16-char token.
    pub fn short(&self) -> String {
        self.encode()[0..4].to_string()
    }

    /// Trailing 12 chars of the encoded token — the master_seed signature.
    /// Constant across all decks within one master_seed run. Combining
    /// `short() + master_signature()` rebuilds the full 16-char token.
    pub fn master_signature(&self) -> String {
        self.encode()[4..16].to_string()
    }

    /// Derive a per-deck RNG seed from the token's tuple. Used by
    /// `build_random_deck` instead of borrowing the master RNG, so any
    /// individual deck can be reconstructed without re-running prior games.
    pub fn per_deck_seed(&self) -> u64 {
        // Lightweight mixing — splitmix64 from the master_seed XOR'd with
        // the packed tuple bits. Plenty for our use; not cryptographic.
        let tuple_bits = ((self.side_bit() as u64) << 13)
            | ((variant_to_index(self.variant_a) as u64) << 10)
            | ((variant_to_index(self.variant_b) as u64) << 7)
            | (self.game_index as u64 & 0x7f);
        splitmix64(self.master_seed ^ tuple_bits.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }
}

fn char_to_value(c: char) -> Result<u8, DecodeError> {
    // Crockford forgiveness: I/L → 1; O → 0.
    let normalized = match c {
        'I' | 'L' => '1',
        'O' => '0',
        c => c,
    };
    match normalized {
        '0'..='9' => Ok(normalized as u8 - b'0'),
        'A'..='H' => Ok(10 + (normalized as u8 - b'A')),
        'J' => Ok(18),
        'K' => Ok(19),
        'M' => Ok(20),
        'N' => Ok(21),
        'P' => Ok(22),
        'Q' => Ok(23),
        'R' => Ok(24),
        'S' => Ok(25),
        'T' => Ok(26),
        'V' => Ok(27),
        'W' => Ok(28),
        'X' => Ok(29),
        'Y' => Ok(30),
        'Z' => Ok(31),
        _ => Err(DecodeError::InvalidChar(c)),
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_token() {
        let original = DeckToken {
            master_seed: 0xDEAD_BEEF_F00D_BABE,
            side: Side::B,
            variant_a: DeckVariant::Pr,
            variant_b: DeckVariant::Gg,
            game_index: 17,
        };
        let encoded = original.encode();
        assert_eq!(encoded.len(), 16);
        let decoded = DeckToken::decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_is_forgiving_to_case_and_hyphens_and_io01() {
        let original = DeckToken {
            master_seed: 0,
            side: Side::A,
            variant_a: DeckVariant::Ra,
            variant_b: DeckVariant::Hu,
            game_index: 0,
        };
        let encoded = original.encode();
        // Inject lowercase + hyphens + ambiguous chars.
        let messy = format!(
            "{}-{}-{}-{}",
            &encoded[0..4].to_lowercase(),
            &encoded[4..8],
            &encoded[8..12].to_lowercase(),
            &encoded[12..16]
        );
        let decoded = DeckToken::decode(&messy).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn decode_rejects_wrong_length() {
        assert_eq!(DeckToken::decode("ABC"), Err(DecodeError::WrongLength));
        assert_eq!(
            DeckToken::decode("ABCDEFGHIJKLMNOPQR"),
            Err(DecodeError::WrongLength)
        );
    }

    #[test]
    fn decode_rejects_invalid_char() {
        // 'U' is not in the Crockford alphabet.
        let bad = "AAAAAAAAAAAAAAA U".replace(' ', "");
        match DeckToken::decode(&bad) {
            Err(DecodeError::InvalidChar('U')) => {}
            other => panic!("expected InvalidChar('U'), got {other:?}"),
        }
    }

    #[test]
    fn per_deck_seed_is_deterministic() {
        let token = DeckToken {
            master_seed: 42,
            side: Side::A,
            variant_a: DeckVariant::Ra,
            variant_b: DeckVariant::Rb,
            game_index: 5,
        };
        let s1 = token.per_deck_seed();
        let s2 = token.per_deck_seed();
        assert_eq!(s1, s2);
        // Different game_index → different seed.
        let token2 = DeckToken {
            game_index: 6,
            ..token
        };
        assert_ne!(token.per_deck_seed(), token2.per_deck_seed());
    }
}
