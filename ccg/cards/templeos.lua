return {
  id = "templeos",
  name = "TempleOS",
  colors = {},
  face = {"glow"},
  frame = "transparent",
  type = "artifact",
  subtypes = {"temple"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 6, source = "graveyard"},
  },
  abilities = {
    "when this card enters the board, return target artifact from your graveyard to your hand.",
    "T, discard an artifact: return target artifact from your graveyard to your hand.",
    "4 attached, sacrifice this: return target creature to its owner's hand.",
  },
  on_enter_board = function(game, self)
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).graveyard) do
      local c = game.card(iid)
      if c and c.type == "artifact" then
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then return end
    local pick = game.choose_card(pool, { optional = true, prompt = "return artifact from graveyard" })
    if pick then
      game.move(pick, "hand")
    end
  end,
  activated = {
    {
      cost_tap = true,
      cost = {{amount = 1, source = "hand", kind = "artifact"}},
      text = "T, discard an artifact: return target artifact from your graveyard to your hand.",
      timing = "instant",
      effect = function(game, self)
        local pool = {}
        for _, iid in ipairs(game.zones(self.owner).graveyard) do
          local c = game.card(iid)
          if c and c.type == "artifact" then
            table.insert(pool, iid)
          end
        end
        if #pool == 0 then return end
        local pick = game.choose_card(pool, { optional = false, prompt = "return artifact from graveyard" })
        if pick then
          game.move(pick, "hand")
        end
      end,
    },
    {
      cost = {
        {amount = 4, source = "attached"},
        {amount = 1, source = "self"},
      },
      text = "4 attached, sacrifice this: return target creature to its owner's hand.",
      timing = "instant",
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
        if #pool == 0 then return end
        local pick = game.choose_card(pool, { optional = false, prompt = "bounce a creature" })
        if pick then
          game.move(pick, "hand")
        end
      end,
    },
  },
}
