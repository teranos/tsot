return {
  id = "signal-goblin",
  name = "Signal Goblin",
  symbols = { U = "꩜" },
  colors = {"blue", "red"},
  holes = {"C"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 1, y = 1},
  abilities = {
    "T, 1 hand, 1 graveyard: deal 1 damage to target creature, then draw a card.",
  },
  activated = {
    {
      cost = {
        {source = "tap"},
        {amount = 1, source = "hand"},
        {amount = 1, source = "graveyard"},
      },
      text = "T, 1 hand, 1 graveyard: deal 1 damage to target creature, then draw a card.",
      timing = "instant",
      validate = function(game, self)
        for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
          for _, iid in ipairs(game.zones(side).board) do
            local c = game.card(iid)
            if c and c.type == "creature" then
              return true
            end
          end
        end
        return false
      end,
      effect = function(game, self)
        local pool = {}
        for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
          for _, iid in ipairs(game.zones(side).board) do
            local c = game.card(iid)
            if c and c.type == "creature" then
              table.insert(pool, iid)
            end
          end
        end
        if #pool > 0 then
          local target = game.choose_card(pool, {prompt = "deal 1 damage to which creature?"})
          if target then
            game.damage(target, 1)
          end
        end
        game.draw(self.owner, 1)
      end,
    },
  },
}
