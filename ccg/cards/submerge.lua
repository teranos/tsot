return {
  id = "submerge",
  name = "Submerge",
  colors = {"azure"},
  type = "instant",
  cost = {
    {amount = 1, source = "mill"},
    {amount = 1, source = "attached"},
  },
  abilities = {
    "draw a card. you may put the top card of your deck on the bottom.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 1)
    local top = game.deck_top(self.owner)
    if top and game.confirm("put top of deck on bottom?") then
      game.move(top, "deck")
    end
  end,
}
