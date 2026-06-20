-- Type and symbol not yet specified.
-- Schema gap: the X in hand-cost and the X in graveyard-cost are linked (same value).
-- Currently represented as two independent is_x components; convention is that all
-- is_x costs in a card share the same X.
return {
  id = "recast",
  name = "Recast",
  colors = {"red"},
  cost = {
    {is_x = true, source = "hand"},
    {is_x = true, source = "graveyard"},
    {amount = 1, source = "self"},
  },
  abilities = {
    "during this turn you may cast the non-creature cards used as a cost to play this card. costs for those cards still need to be paid.",
  },
}
