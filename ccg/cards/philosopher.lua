return {
  id = "philosopher",
  name = "Philosopher",
  symbol = "⨳",
  colors = {"white", "black"},
  type = "creature",
  subtypes = {"human", "philosopher"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "graveyard"},
  },
  stats = {x = 2, y = 2},
  abilities = {
    "humans you control get +0/+1.",
    "T: return target human you control to your hand.",
  },
  static = {
    affects = {
      subtypes = {"human"},
      controller = "owner",
      exclude_self = true,
    },
    modifier = {x = 0, y = 1},
  },
  activated = {
    {
      cost = "tap",
      text = "T: return target human you control to your hand.",
      timing = "instant",
      validate = function(game, self)
        for _, iid in ipairs(game.zones(self.owner).board) do
          local c = game.card(iid)
          if c and c.subtypes then
            for _, st in ipairs(c.subtypes) do
              if st == "human" then
                return true
              end
            end
          end
        end
        return false
      end,
      effect = function(game, self)
        local pool = {}
        for _, iid in ipairs(game.zones(self.owner).board) do
          local c = game.card(iid)
          if c and c.subtypes then
            for _, st in ipairs(c.subtypes) do
              if st == "human" then
                table.insert(pool, iid)
                break
              end
            end
          end
        end
        if #pool == 0 then return end
        local target = game.choose_card(pool, {prompt = "return which human to hand?"})
        if target then
          game.move(target, "hand")
        end
      end,
    },
  },
}
