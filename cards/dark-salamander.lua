-- Black salamander. Cast: X hand cards attach to the salamander
-- (hydra pattern). Effective stats are X/X via the source-only static
-- reading the attached count — so the salamander you cast for X = 3
-- arrives as a 3/3.
--
-- Activated ability:
--   "Y hand: mill your opponent by 2Y - X"
-- where Y is the activation's variable cost (the X-of-the-activation,
-- read in the handler via `game.x_value()`) and X is the salamander's
-- effective X stat at activation time (from `game.card(self).x`,
-- driven by attached count via the source_only static).
--
-- Validate hook refuses the activation when 2Y - X ≤ 0 (would do no
-- mill but cost cards) — per RULES A.9, no hand is paid in that case.
-- Mill payoff: pay Y = X/2 + 1 to mill 2, Y = X to mill X, etc.
--
-- Sim AI caveat: depends on the X-rejection fix in
-- `pick_random_playable_in_hand` for non-creature casts; creature casts
-- with is_x bypass that gate today (same as hydra), so the salamander
-- IS playable in the sim as a base 0/0 that grows via attached count.
return {
  id = "dark-salamander",
  name = "Dark Salamander",
  symbol = "⨳",
  colors = {"black"},
  type = "creature",
  subtypes = {"salamander"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "Y hand: mill your opponent by Y + Y - X.",
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
      text = "Y hand: mill your opponent by 2Y - X.",
      timing = "instant",
      validate = function(game, self)
        -- RULES A.9: refuse if 2Y - X ≤ 0 (no mill, all cost wasted).
        local y = game.x_value() or 0
        local me = game.card(self.instance_id)
        local x = (me and me.x) or 0
        return (2 * y - x) > 0
      end,
      effect = function(game, self)
        local y = game.x_value() or 0
        local me = game.card(self.instance_id)
        local x = (me and me.x) or 0
        local mill = 2 * y - x
        if mill > 0 then
          local opp = game.opponent(self.owner)
          game.mill(opp, mill, "graveyard")
        end
      end,
    },
  },
}
