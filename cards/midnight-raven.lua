return {
  id = "midnight-raven",
  name = "Midnight raven",
  symbol = "⋈",
  colors = {"black"},
  type = "creature",
  subtypes = {"bird"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "flying.",
    "whenever this creature attacks, put the top card of your DECK on the bottom.",
  },
  stats = {x = 1, y = 1},
  on_attack = function(game, self)
    local top = game.deck_top(self.owner)
    if top then
      game.move(top, "deck")
    end
  end,
}
