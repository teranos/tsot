-- Green defender that pings whatever it blocks.
return {
  id = "thorn-beetle",
  name = "Thorn Beetle",
  colors = {"green"},
  type = "creature",
  subtypes = {"insect"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "defender.",
    "whenever this creature blocks, deal 1 damage to the attacker.",
  },
  stats = {x = 0, y = 3},
  on_block = function(game, self, attacker)
    game.damage(attacker.instance_id, 1)
  end,
}
