-- Transparent artifact. Lives on board (or in graveyard) as a HAND-
-- payment-slot substitute drawn from beyond the hand zone.
--
-- Intent:
--   "While this card is in your graveyard, you may exile it to fill 1
--    HAND-source slot of a spell you cast."
--
-- Important constraint Clear View does NOT remove: P.7a identity
-- matching still applies to every OTHER HAND payment. Clear View has
-- empty identity (no colors, no symbols by C.13), so it contributes
-- no identity overlap to satisfy P.7a for the cast as a whole. This
-- means:
--   - A 1-hand spell with non-empty identity (any colored card)
--     cannot be paid by Clear View alone — there are no other slots
--     to satisfy P.7a, and Clear View itself doesn't match.
--   - A 2-hand spell with non-empty identity CAN be paid using Clear
--     View for one slot only if the OTHER slot is paid by a card that
--     matches the cast's identity.
-- Net effect: Clear View is a "stretch hand size by 1" enabler for
-- multi-hand casts where you already have at least one matching card.
-- Strictly weaker than a wildcard. Empty-identity casts (colorless,
-- no-symbol) can use Clear View freely since P.7a is wildcard-cast.
--
-- DESIGN-ANCHORED — engine support missing:
--   1. **PlayChoices.gy_payment_ids** (or similar) — currently HAND
--      payments come from `hand_payment_ids: Vec<InstanceId>`, all
--      required to be in HAND. Need a parallel `gy_payment_ids` that
--      moves cards GY → EXILE and counts toward HAND cost slot fill.
--      P.7a per-payment check skips Clear View slots (Clear View has
--      empty identity, so the check would always fail anyway — but
--      it should be a deliberate skip, not an accidental pass).
--   2. **Sim AI integration** — `pick_random_playable_in_hand`'s
--      affordability check needs to know: a Clear View in graveyard
--      adds 1 to "effective hand-pay capacity" for casts whose other
--      payments still satisfy P.7a. The AI's smart-pitch heuristic
--      should prefer NOT discarding Clear Views (their value is in GY,
--      not in hand).
--
-- Cast cost: 1 hand — cheap, since Clear View's value materializes
-- once it's in the graveyard. Per C.13 transparent cards have no
-- symbols.
return {
  id = "clear-view",
  name = "Clear View",
  colors = {"transparent"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to fill 1 hand-source slot of a spell you cast. clear view does not satisfy P.7a identity for the cast — other hand payments must.",
  },
  flavor = "Read the contract through the contract.",
}
