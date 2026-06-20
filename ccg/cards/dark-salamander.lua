-- Black salamander. Cast: X hand cards attach to the salamander
-- (hydra pattern) + X graveyard cards exile per P.12 — both is_x
-- components share the same X. Effective stats are X/X via the
-- source-only static reading the attached count, so a salamander
-- cast for X = 3 arrives as a 3/3 after committing 3 GY cards.
-- Per P.12a at least one of those GY pitches must be black.
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
  cost = {
    {is_x = true, source = "hand"},
    {is_x = true, source = "graveyard"},
  },
  abilities = {
    "X hand, X graveyard: enters as X/X salamander.",
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
