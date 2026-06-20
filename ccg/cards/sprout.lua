return {
  id = "sprout",
  name = "Sprout",
  colors = {"green"},
  type = "instant",
  cost = {
    {amount = 1, source = "mill"},
    {amount = 1, source = "attached"},
  },
  abilities = {
    "draw a card. you may put the bottom card of your deck on top.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 1)
    local bottom = game.deck_bottom(self.owner)
    if bottom and game.confirm("put bottom of deck on top?") then
      game.move_to_deck_top(bottom)
    end
  end,
}
