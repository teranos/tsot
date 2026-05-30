-- Red human, 1 hand + 1 gy, 2/1, haste + on_attack SELF-mill 2. The
-- haste keyword is already enforced by declare_attacker (overrides B.3
-- summoning sickness), so this creature can swing the turn it lands.
-- The on_attack mill targets the OWNER's deck — it's a fast-clock card
-- that pays for tempo with its caster's deck. Net per attack:
--   opp deck: -2 (B.2 combat damage from X=2)
--   own deck: -2 (on_attack mill self)
-- So the deck-balance is even per swing; you're trading pacing for
-- inevitability.
return {
  id = "haste-human",
  name = "Haste Human",
  type = "creature",
  colors = {"red"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "haste.",
    "whenever this creature attacks, you mill 2.",
  },
  stats = {x = 2, y = 1},
  on_attack = function(game, self)
    game.mill(self.owner, 2, "graveyard")
  end,
}
