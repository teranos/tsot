-- Black artifact: graveyard-powered anthem. While your graveyard has 5
-- or more cards, creatures you control get +1/+1 and have flying.
--
-- Uses all three STATIC Phase 2 capabilities on one static: state-reading
-- condition (graveyard threshold), combined stat-and-keyword modifier,
-- and Phase 1 affects (subtype + controller).
--
-- Cost 2 hand + 2 mill: an investment that ironically adds 2 cards to
-- the graveyard you're stocking. Symbol not yet specified.
return {
  id = "ossuary",
  name = "Ossuary",
  colors = {"black"},
  type = "artifact",
  subtypes = {"relic"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "while your graveyard has 5 or more cards, creatures you control get +1/+1 and have flying.",
  },
  static = {
    affects = {
      kind = "creature",
      controller = "owner",
      exclude_self = true,
    },
    modifier = {x = 1, y = 1, keyword = "flying"},
    condition = {kind = "owner_graveyard_size", min = 5},
  },
}
