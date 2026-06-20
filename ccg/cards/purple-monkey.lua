-- Purple Monkey — 1h, 2/2. Artifact removal in a creature body.
-- Cost cycle: 1 hand to cast, 2 hand to activate. Activation destroys
-- one opposing non-creature on board — primarily artifacts (jewels,
-- crystals, lcd-clock). Notable interaction: tapping the 2-hand cost
-- to wipe an enemy jewel deletes their cost-substitution engine.
return {
  id = "purple-monkey",
  name = "Purple Monkey",
  colors = {"purple"},
  type = "creature",
  subtypes = {"monkey"},
  symbol = "≡",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "2 hand: destroy target opposing non-creature on the board.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = {{amount = 2, source = "hand"}},
      text = "2 hand: destroy target opposing non-creature.",
      timing = "instant",
      validate = function(game, self)
        -- RULES A.9: needs an opposing non-creature (artifact, etc.).
        local opp = game.opponent(self.owner)
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type ~= "creature" then return true end
        end
        return false
      end,
      effect = function(game, self)
        local opp = game.opponent(self.owner)
        local pool = {}
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type ~= "creature" then table.insert(pool, iid) end
        end
        if #pool == 0 then return end
        local target = game.choose_card(pool, {prompt = "destroy non-creature"})
        if not target then return end
        game.move(target, "graveyard")
      end,
    },
  },
}
