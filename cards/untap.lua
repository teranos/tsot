-- Blue/green instant: untap a target creature. Cost 1 hand.
--
-- Pool = all tapped creatures on either BOARD (the printed text is
-- generic — "target creature"). The sim's choose_card oracle prefers
-- candidates controlled by the asker (the prefer-self target heuristic
-- in choice.rs), so in practice this untaps one of your own tapped
-- creatures — typically a vigilance-shaped tempo trick (free another
-- attack this turn / unblock a blocker for next turn).
--
-- Symbol not yet specified.
return {
  id = "untap",
  name = "Untap",
  type = "instant",
  colors = {"blue", "green"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "untap target creature.",
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
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "untap target creature"})
    if target then
      game.untap(target)
    end
  end,
}
