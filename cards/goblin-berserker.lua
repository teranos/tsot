-- Red goblin, 1 hand + 2 mill, 1/1, on_attack discard 1 + draw 1. Net
-- card-quality "looter" effect — swap a card from hand for a draw every
-- time it attacks. The discard is deterministic front-of-hand for now
-- (no choice yet); when a discard-with-choice API lands, the handler
-- can prompt the user.
return {
  id = "goblin-berserker",
  name = "Goblin Berserker",
  colors = {"red"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "whenever this creature attacks, discard a card and draw a card.",
  },
  stats = {x = 1, y = 1},
  on_attack = function(game, self)
    game.discard(self.owner, 1)
    game.draw(self.owner, 1)
  end,
}
