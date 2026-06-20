return {
  id = "sparkle",
  name = "Sparkle",
  colors = {"red"},
  face = {"glow", "shiny"},
  type = "instant",
  cost = {
    {amount = 1, source = "mill"},
    {amount = 1, source = "attached"},
  },
  abilities = {
    "draw a card. target creature gets +1/+0 until end of turn.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 1)
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
    local target = game.choose_card(pool, {prompt = "+1/+0 EOT to which creature?"})
    if target then
      game.add_modifier(target, "stat_boost", 1, 0, "end_of_turn")
    end
  end,
}
