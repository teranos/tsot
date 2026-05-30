-- Blue mass-untap instant + cantrip. 2 hand cost. Use case: untap mid-
-- combat to enable activated abilities or post-block plays, or pre-combat
-- for vigilance-like timing. Cantrip makes the card self-replacing.
-- Net hand-count change for the caster:
--   -2 (cost) -1 (spell leaves) +1 (cantrip) = -2
return {
  id = "surge",
  name = "Surge",
  colors = {"blue"},
  type = "instant",
  cost = {{amount = 2, source = "hand"}},
  abilities = {
    "untap all creatures you control. draw a card.",
  },
  on_play = function(game, self)
    for _, iid in ipairs(game.zones(self.owner).board) do
      game.untap(iid)
    end
    game.draw(self.owner, 1)
  end,
}
