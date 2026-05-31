-- Blue Monkey — 1h, 2/2. Card-advantage engine on a stick.
-- Cost cycle: 1 hand to cast, 2 hand to activate. Activation draws one
-- card — net -1 card per activation (paid 2, drew 1) so this only wins
-- value when the cards spent are otherwise useless or the drawn card
-- is high-impact. Combos with the jewel cycle's `T: draw, discard` —
-- discard 2 jewels for an instant draw, then untap and tap them next
-- turn for two more draws.
return {
  id = "blue-monkey",
  name = "Blue Monkey",
  colors = {"blue"},
  type = "creature",
  subtypes = {"monkey"},
  symbol = "am",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "2 hand: draw a card.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = {{amount = 2, source = "hand"}},
      text = "2 hand: draw a card.",
      timing = "instant",
      effect = function(game, self)
        game.draw(self.owner, 1)
      end,
    },
  },
}
