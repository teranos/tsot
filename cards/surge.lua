-- Blue mass-untap instant. Use case: untap mid-combat to enable activated
-- abilities or post-block plays, or untap pre-combat for vigilance-like
-- timing tricks.
return {
  id = "surge",
  name = "Surge",
  colors = {"blue"},
  type = "instant",
  cost = {{amount = 2, source = "hand"}},
  abilities = {
    "untap all creatures you control.",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).board) do
      game.untap(iid)
    end
  end,
}
