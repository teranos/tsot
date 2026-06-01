-- Black salamander. Cast: X hand cards attach to the salamander
-- (hydra pattern). Effective stats are X/X via the source-only static
-- reading the attached count — so the salamander you cast for X = 3
-- arrives as a 3/3.
--
-- Activated ability:
--   "Y hand: mill your opponent by 2Y"
-- where Y is the activation's variable cost (read via `game.x_value()`).
return {
  id = "dark-salamander",
  name = "Dark Salamander",
  symbol = "⨳",
  colors = {"black"},
  type = "creature",
  subtypes = {"salamander"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "Y hand: mill your opponent by Y + Y.",
  },
  stats = {x = 0, y = 0},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {x = "attached", y = "attached"},
  },
  activated = {
    {
      cost = {{is_x = true, source = "hand"}},
      text = "Y hand: mill your opponent by 2Y.",
      timing = "instant",
      effect = function(game, self)
        local y = game.x_value() or 0
        local mill = 2 * y
        if mill > 0 then
          local opp = game.opponent(self.owner)
          game.mill(opp, mill, "graveyard")
        end
      end,
    },
  },
}
