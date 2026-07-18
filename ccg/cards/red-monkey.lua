-- Red Monkey — 1h, 2/2 with a tap-free hand-pay activated ability.
-- Cost cycle: 1 hand to cast, 2 hand to activate. Activation does
-- NOT require tapping, so the monkey can attack AND activate the same
-- turn (the 2-hand cost is the throttle, not tap availability).
--
-- Effect: deal 2 damage to a target creature (either side). If the
-- creature survives, it gains haste until end of turn — useful for
-- targeting your own freshly-played creature to enable an immediate
-- attack. If the damage kills the target (2 ≥ effective y), we move
-- it to graveyard manually (no SBA loop) and skip the haste grant.
return {
  id = "red-monkey",
  name = "Red Monkey",
  colors = {"red"},
  type = "creature",
  subtypes = {"monkey"},
  symbol = "≡",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "2 hand: deal 2 damage to target creature; if it survives, that creature gains haste until end of turn.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = {{amount = 2, source = "hand"}},
      text = "2 hand: deal 2 damage to target creature; if it survives, gains haste EOT.",
      timing = "instant",
      validate = function(game, self)
        -- RULES A.9: refuse activation when no creature target exists.
        local own = self.owner
        local opp = game.opponent(own)
        for _, iid in ipairs(game.zones(own).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then return true end
        end
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then return true end
        end
        return false
      end,
      effect = function(game, self)
        local own = self.owner
        local opp = game.opponent(own)
        local pool = {}
        for _, iid in ipairs(game.zones(own).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then table.insert(pool, iid) end
        end
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then table.insert(pool, iid) end
        end
        if #pool == 0 then return end
        local target = game.choose_card(pool, {prompt = "deal 2 + haste rider"})
        if not target then return end
        game.damage(target, 2)
        -- C.15 continuous check: if damage dropped effective y ≤ 0,
        -- the engine already moved target to GY. Don't manually move
        -- again — that errors with "card not found in any zone".
        -- Only grant haste when the target survived the damage.
        local after = game.card(target)
        if after and after.y and after.y > 0 then
          game.add_modifier(target, "gains_haste", 0, 0, "end_of_turn")
        end
      end,
    },
  },
}
