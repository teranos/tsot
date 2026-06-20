-- White Monkey — 1h, 2/2. Board-wide anthem with vigilance rider.
-- Cost cycle: 1 hand to cast, 2 hand to activate. Activation pumps
-- every creature you control by +2/+2 AND grants them vigilance, both
-- until end of turn. Vigilance keeps the attacking creatures untapped
-- through combat — useful for stacking activations in main phase 2 OR
-- preserving blocker presence on the opponent's swing-back.
--
-- "Creatures you control" includes the white monkey itself. Self-pump:
-- a freshly-played white monkey becomes 4/4 with vigilance the same
-- turn (B.3 summoning sickness blocks attack, but the buff stacks for
-- whatever combat the controller does declare).
return {
  id = "white-monkey",
  name = "White Monkey",
  colors = {"white"},
  type = "creature",
  subtypes = {"monkey"},
  symbol = "≡",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "2 hand: creatures you control get +2/+2 and vigilance until end of turn.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = {{amount = 2, source = "hand"}},
      text = "2 hand: creatures you control get +2/+2 and vigilance until end of turn.",
      timing = "instant",
      effect = function(game, self)
        local own = self.owner
        for _, iid in ipairs(game.zones(own).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then
            game.add_modifier(iid, "stat_boost", 2, 2, "end_of_turn")
            game.add_modifier(iid, "gains_vigilance", 0, 0, "end_of_turn")
          end
        end
      end,
    },
  },
}
