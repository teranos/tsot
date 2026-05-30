-- Red human, 1 hand, 2/1, haste + on_attack mill 2. The haste keyword is
-- already enforced by declare_attacker (overrides B.3 summoning sickness),
-- so this creature can swing the turn it lands. The on_attack mill is
-- extra deckout pressure on top of B.2 combat damage.
return {
  id = "haste-human",
  name = "Haste Human",
  type = "creature",
  colors = {"red"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "haste.",
    "whenever this creature attacks, mill 2.",
  },
  stats = {x = 2, y = 1},
  on_attack = function(game, self)
    game.mill(game.opponent(self.owner), 2, "graveyard")
  end,
}
