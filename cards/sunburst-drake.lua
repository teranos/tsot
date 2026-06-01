-- Orange drake. Hydra-style cast (X hand attaches → 0/0 base scales
-- with attached count via source_only static). The activated ability
-- re-encodes the Y - X "two-variable arithmetic" pattern dark-salamander
-- used to carry: damage equals Y (activation's variable cost) minus the
-- drake's own effective X stat. To get net damage you must Y > X — so
-- the bigger the drake's body, the more expensive the burst.
return {
  id = "sunburst-drake",
  name = "Sunburst Drake",
  symbol = "≡",
  colors = {"orange"},
  type = "creature",
  subtypes = {"drake"},
  cost = {{is_x = true, source = "hand"}},
  abilities = {
    "Y hand: deal Y - X damage to target creature (X is this creature's effective X).",
  },
  stats = {x = 0, y = 0},
  static = {
    affects = {scope = "source_only"},
    modifier = {x = "attached", y = "attached"},
  },
  activated = {
    {
      cost = {{is_x = true, source = "hand"}},
      text = "Y hand: deal Y - X damage to target creature.",
      timing = "instant",
      validate = function(game, self)
        -- Refuse activation when Y - X ≤ 0 (no damage, all cost wasted).
        -- Also refuse when no opposing creature exists.
        local y = game.x_value() or 0
        local me = game.card(self.instance_id)
        local x = (me and me.x) or 0
        if (y - x) <= 0 then return false end
        local opp = game.opponent(self.owner)
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then return true end
        end
        return false
      end,
      effect = function(game, self)
        local y = game.x_value() or 0
        local me = game.card(self.instance_id)
        local x = (me and me.x) or 0
        local dmg = y - x
        if dmg <= 0 then return end
        local opp = game.opponent(self.owner)
        local pool = {}
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then table.insert(pool, iid) end
        end
        if #pool == 0 then return end
        game.set_intent("remove_threat")
        local target = game.choose_card(pool, {prompt = "deal Y-X damage"})
        if not target then return end
        game.damage(target, dmg)
        local after = game.card(target)
        if after and after.y and dmg >= after.y then
          game.move(target, "graveyard")
        end
      end,
    },
  },
}
