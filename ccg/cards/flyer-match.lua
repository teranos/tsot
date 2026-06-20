-- Name, color, and symbol not yet specified.
-- Type inferred as CREATURE from stats + flying keyword.
return {
  id = "flyer-match",
  type = "creature",
  symbols = {"꩜", "⨳", "⋈", "⊨"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  abilities = {
    "flying.",
    "gets +3/+0 if the top card of your DECK has the same symbol as an attached card on this creature.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {scope = "source_only"},
    condition = {kind = "deck_top_symbol_matches_attached"},
    modifier = {x = 3, y = 0},
  },
}
