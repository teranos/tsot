-- Name, color, and symbol not yet specified.
-- Type inferred as CREATURE from stats + flying keyword.
return {
  id = "flyer-match",
  type = "creature",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  abilities = {
    "flying.",
    "gets +3/+0 if the top card of your DECK has the same symbol as an attached card on this creature.",
  },
  stats = {x = 1, y = 1},
}
