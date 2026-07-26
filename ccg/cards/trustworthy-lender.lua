-- Symbol not yet specified.
return {
  id = "trustworthy-lender",
  name = "Trustworthy Lender",
  colors = {"white"},
  type = "creature",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "when this creature dies, return cards attached to it to your hand.",
  },
  stats = {x = 2, y = 2},
  on_zone_change = function(game, self, moving, from, to)
    if moving.instance_id ~= self.instance_id then return end
    if from ~= "board" or to ~= "graveyard" then return end
    for _, aid in ipairs(self.attached) do
      game.move(aid, "hand")
    end
  end,
}
