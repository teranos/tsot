return {
  id = "chromatic-burn",
  name = "Chromatic Burn",
  colors = {"red"},
  type = "spell",
  cost = {
    {amount = 3, source = "graveyard"},
  },
  abilities = {
    "each player mills X * 3, where X is the number of distinct colors among cards on the board.",
  },
  on_play = function(game, self)
    local colors_seen = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c then
          for _, col in ipairs(c.colors) do
            colors_seen[col] = true
          end
        end
      end
    end
    local x = 0
    for _ in pairs(colors_seen) do x = x + 1 end
    local mill_n = x * 3
    if mill_n > 0 then
      game.mill(self.owner, mill_n, "graveyard")
      game.mill(game.opponent(self.owner), mill_n, "graveyard")
    end
  end,
}
