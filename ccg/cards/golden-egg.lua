-- Golden Egg — shiny yellow artifact. Self-exiling cantrip.
return {
  id = "golden-egg",
  name = "Golden Egg",
  type = "artifact",
  colors = {"yellow"},
  face = {"shiny"},
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 4, source = "mill"},
  },
  abilities = {
    "T, exile this: draw a card.",
  },
  flavor = "Worth the hatch.",
  activated = {
    {
      cost = {{source = "tap"}, {source = "self", amount = 1}},
      text = "T, exile this: draw a card.",
      timing = "instant",
      effect = function(game, self)
        game.draw(self.owner, 1)
      end,
    },
  },
}
