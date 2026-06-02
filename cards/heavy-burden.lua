-- Pink instant: 1 graveyard. Kills a target creature whose combined
-- printed cost is 4 or more. Aimed at the high-cost end of the curve —
-- cheap removal that only works on big things.
return {
  id = "heavy-burden",
  name = "Heavy Burden",
  colors = {"pink"},
  type = "instant",
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "kill a target creature whose combined cost is 4 or more.",
  },
  on_play = function(game, self)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c and c.type == "creature" and (c.combined_cost or 0) >= 4 then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    game.set_intent("remove_threat")
    local target = game.choose_card(pool, {prompt = "kill which big creature?"})
    if target then
      game.move(target, "graveyard")
    end
  end,
}
