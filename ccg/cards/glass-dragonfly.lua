-- Azure glass insect cycle. 1/1 for 3 graveyard (P.12a needs at least
-- one azure GY pitch to anchor). Source-only static refuses any
-- attachment per the engine `CannotBeAttachedTo` restriction.
return {
  id = "glass-dragonfly",
  name = "Glass Dragonfly",
  colors = {"azure"},
  type = "creature",
  subtypes = {"insect"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "flying.",
    "cards can't be attached to this creature.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {scope = "source_only"},
    restrictions = {"cannot_be_attached_to"},
  },
}
