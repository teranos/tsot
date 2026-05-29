-- Name and color not yet specified.
-- Second ability ("next card you play, may cast as instant") deferred:
-- needs a per-turn modifier registry that we don't have yet.
return {
  id = "draw-two",
  symbol = "⨳",
  type = "instant",
  cost = {{amount = 3, source = "graveyard"}},
  abilities = {
    "draw two cards.",
    "the next card you play, you may cast it as if it was an instant.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 2)
  end,
}
