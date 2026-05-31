-- Single-color vanilla rat. "Can't block cats." Mill cost reflects
-- the rat-flavor "eats through your stuff" — a small tax on top of
-- the hand cost, eating the top of your deck.
return {
  id = "temple-rat",
  name = "Temple Rat",
  type = "creature",
  colors = {"white"},
  subtypes = {"rat"},
  cannot_block_subtypes = {"cat"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  stats = {x = 3, y = 1},
  abilities = {"can't block cats."},
}
