-- Red goblin lord: 1/1 for 3 mill. Phase 2 motivator — combined stat AND
-- keyword on one static. Affects scoped to controller = owner (warband, not
-- warlord's literal-text "all" reading).
--
-- Mechanically: drops a 1/1 and immediately makes every other goblin you
-- control bigger by +1/+1 AND gives them haste. In the red deck this is a
-- finisher — eager-goblin, goblin-berserker, and phantom-goblin all gain
-- value the moment warchief lands.
return {
  id = "goblin-warchief",
  name = "Goblin Warchief",
  colors = {"red"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 3, source = "mill"},
  },
  abilities = {
    "other goblins you control get +1/+1 and have haste.",
  },
  stats = {x = 1, y = 1},
  static = {
    affects = {
      subtypes = {"goblin"},
      controller = "owner",
      exclude_self = true,
    },
    modifier = {x = 1, y = 1, keyword = "haste"},
  },
}
