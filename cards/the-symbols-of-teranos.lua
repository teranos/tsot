-- The flagship lore card of the corpus. Named after RULES.md itself
-- ("The Symbols of Teranos — Rules"). Pure identity card: no type, no
-- cost, no subtype, no colors. When multi-symbol schema lands, it
-- carries every known symbol in the corpus — its identity-set per
-- P.7a then contains the union of every symbol-glyph other cards use,
-- making it a near-universal HAND payment for anything that
-- participates in symbol-matching.
--
-- BLOCKED ON MULTI-SYMBOL SCHEMA: today `pub symbol: String` in card.rs
-- holds exactly one symbol per card. Until the schema changes to
-- `symbols: Vec<String>` (or single-shorthand-plus-explicit-array),
-- this card cannot carry all 8 corpus symbols simultaneously. Per the
-- rules update being drafted (C.1, C.11, C.13, P.7a updated to plural
-- "symbols"), the card's intended state is:
--
--   symbols = {"꩜", "⨳", "⋈", "⊨", "am", "≡", "IX", "ax"}
--
-- That's the 8 distinct symbols in the corpus at the moment this card
-- is being created: ꩜ (Pulse, 5 cards), ⨳ (4 cards), ⋈ (3 cards),
-- ⊨ (3 cards), am (5 cards, monkey tribe), ≡ (1 card, amsterdam-city),
-- IX (1 card, dark-salamander), ax (1 card, scavenger-rat).
--
-- Until the schema lands, the symbol field is intentionally omitted so
-- nothing implies a single-symbol identity. When multi-symbol support
-- ships, fill in the array above and remove this paragraph.
--
-- Colors: NONE. The card is colorless on purpose — it's the symbols
-- alone that carry its identity. Per P.7a, a card with no colors and
-- no symbols has empty identity (matches nothing as payment, accepts
-- any payment as a cast). Once multi-symbol lands and this card
-- carries all 8 symbols, identity-set goes from empty → 8 elements,
-- making it a near-universal payment-match (overlaps any other card
-- carrying any of those symbols) while remaining itself a wildcard
-- recipient of any HAND payment (since it has no colors to constrain
-- payment matching from the other direction).
--
-- Typeless. No cost. No subtype. No colors. No abilities. The card is
-- pure identity: its name and (when multi-symbol schema lands) its set
-- of symbols. Nothing else. The engine treats a card with no declared
-- type as kind Unspecified — uncastable through play_card, which is
-- the intended state for a pure-identity object. It exists in the
-- corpus as a referenceable card, not as a deck-playable card.
return {
  id = "the-symbols-of-teranos",
  name = "The Symbols of Teranos",
  colors = {},
}
