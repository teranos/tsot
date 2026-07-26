-- Purple goblin — ghostly recurring 1/1 fueled by your graveyard.
-- Costs 1 graveyard to play (can't be cast on an empty graveyard); when it
-- dies, you may return it to your hand for another cycle. Self-limiting:
-- each replay requires another graveyard card, so it doesn't loop infinitely
-- without a fuel source.
return {
  id = "phantom-goblin",
  name = "Phantom Goblin",
  symbol = "꩜",
  colors = {"purple"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "when this creature dies, mill 1, then you may return it to your hand.",
  },
  stats = {x = 1, y = 1},
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if from ~= "board" or to ~= "graveyard" then return end
    game.mill(self.owner, 1, "graveyard")
    if game.confirm("return Phantom Goblin to your hand?") then
      game.move(self.instance_id, "hand")
    end
  end,
}
