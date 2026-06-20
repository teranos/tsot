-- Reveal is print-only today; engine treats all card data as openly
-- accessible to handlers, so "opponent reveals 4 cards" has no in-game
-- mutation. Rules adjustment (V-section addendum: hand cards default
-- to backside-only-visible, reveal = momentary face-up) pending before
-- this becomes a real mechanic.
return {
  id = "pry",
  name = "Pry",
  colors = {"black"},
  type = "instant",
  cost = {
    {amount = 1, source = "mill"},
    {amount = 1, source = "attached"},
  },
  abilities = {
    "draw a card. target opponent reveals 4 cards.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 1)
  end,
}
