return {
  id = "mortal-bee",
  name = "Mortal Bee",
  colors = {"white"},
  type = "creature",
  subtypes = {"insect"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "flying.",
    "whenever this creature attacks, exile 1 card from each opponent's deck. this creature doesn't untap on your next turn.",
  },
  stats = {x = 3, y = 3},
  on_attack = function(game, self)
    game.mill(game.opponent(self.owner), 1, "exile")
    game.add_status(self.instance_id, "skip_untap", 1)
  end,
}
