return {
  id = "spectrum-cull",
  name = "Spectrum Cull",
  colors = {},
  type = "spell",
  cost = {
    {amount = 1, source = "hand"},
    {is_x = true, source = "graveyard"},
    {is_x = true, source = "mill"},
    {amount = 1, source = "self"},
  },
  abilities = {
    "choose X creatures each with a different color; destroy them.",
  },
  on_play = function(game, self)
    local x = game.x_value() or 0
    if x <= 0 then return end
    local used_colors = {}
    for _ = 1, x do
      local pool = {}
      for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
        for _, iid in ipairs(game.zones(side).board) do
          local c = game.card(iid)
          if c and c.type == "creature" and #c.colors > 0 then
            local clean = true
            for _, col in ipairs(c.colors) do
              if used_colors[col] then
                clean = false
                break
              end
            end
            if clean then table.insert(pool, iid) end
          end
        end
      end
      if #pool == 0 then return end
      local pick = game.choose_card(pool, { optional = false, prompt = "destroy a creature (color must be unused)" })
      if not pick then return end
      local c = game.card(pick)
      if c then
        for _, col in ipairs(c.colors) do
          used_colors[col] = true
        end
      end
      game.move(pick, "graveyard")
    end
  end,
}
