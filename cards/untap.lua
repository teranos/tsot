-- Blue/green instant: untap a target creature + cantrip. Cost 1 hand.
--
-- Pool = all tapped creatures on either BOARD. The sim's choose_card
-- oracle prefers candidates controlled by the asker, so in practice
-- this untaps one of your own tapped creatures — typically a vigilance-
-- shaped tempo trick (free another attack this turn / unblock a blocker
-- for next turn).
--
-- The cantrip (draw 1 after resolving the untap) makes the card
-- self-replacing even if no legal target exists. Net hand-count change
-- for the caster:
--   -1 (cost pitch) -1 (spell leaves) +1 (cantrip) = -1
--
-- Symbol not yet specified.
return {
  id = "untap",
  name = "Untap",
  type = "instant",
  colors = {"blue", "green"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "untap target creature. draw a card.",
  },
  on_play = function(game, self)
    local pool = {}
    for _, pid in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(pid).board) do
        local c = game.card(iid)
        if c and c.type == "creature" and c.tapped then
          table.insert(pool, iid)
        end
      end
    end
    if #pool > 0 then
      local target = game.choose_card(pool, {prompt = "untap target creature"})
      if target then
        game.untap(target)
      end
    end
    -- Cantrip fires whether or not a target existed — self-replacing.
    game.draw(self.owner, 1)
  end,
}
