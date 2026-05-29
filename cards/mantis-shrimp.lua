-- Red/blue companion to zebra in the "if-attached-as-cost-of-matching-color"
-- cycle. Playable as a vanilla 2/2 today; the conditional reveal-and-draw
-- trigger awaits an `on_attach_as_cost` event (not in the Phase 1 taxonomy).
return {
  id = "mantis-shrimp",
  name = "Mantis Shrimp",
  colors = {"red", "blue"},
  type = "creature",
  subtypes = {"shrimp"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "If this card gets attached as a cost to a red or blue card, you may reveal it and draw a card.",
  },
  stats = {x = 2, y = 2},
}
