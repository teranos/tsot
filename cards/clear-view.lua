-- Transparent artifact. Lives on board (or in graveyard) as a flexible
-- HAND-payment substitute drawn from beyond the hand zone.
--
-- Intent:
--   "While this card is in your graveyard, you may exile it instead of
--    discarding a card from your hand to pay 1 HAND-source cost
--    component of a spell you cast."
--
-- Mechanically this is GY → EXILE substitution for HAND payment. It
-- also bypasses P.7a identity matching for that one component — any
-- spell's HAND cost can be paid by exiling Clear View, regardless of
-- color/symbol identity. Transparent's role is conceptual: the card
-- "sees through" the identity-match requirement.
--
-- DESIGN-ANCHORED — engine support missing:
--   1. **PlayChoices.gy_payment_ids** (or similar) — currently HAND
--      payments come from `hand_payment_ids: Vec<InstanceId>`, all
--      required to be in HAND. Need a parallel `gy_payment_ids` that
--      moves cards GY → EXILE and counts toward HAND cost satisfaction.
--   2. **Identity-match bypass flag** on the substitute payment — for
--      Clear View specifically, exclude this payment slot from the
--      P.7a check. Could be a per-card flag (`bypasses_identity = true`)
--      or a generic "this card can substitute for HAND payment from GY"
--      marker.
--   3. **Sim AI integration** — `pick_random_playable_in_hand`'s
--      affordability check needs to know about GY-pay candidates so
--      it doesn't reject otherwise-castable spells when hand is short.
--      Also the choice of WHICH Clear View to exile (if multiple in GY)
--      is currently arbitrary; smart-pitch heuristic doesn't apply (the
--      decision is binary: use Clear View or don't).
--
-- Until that lands, the card loads with intent in `abilities` and
-- the on-board behavior is inert. Cast cost: 1 hand — cheap, since
-- Clear View's value materializes once it's in the graveyard, not on
-- the board. Per C.13 transparent cards have no symbol.
return {
  id = "clear-view",
  name = "Clear View",
  colors = {"transparent"},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "while this card is in your graveyard, you may exile it to pay 1 hand-source cost component of a spell you cast. ignores hand-payment identity matching.",
  },
  flavor = "Read the contract through the contract.",
}
