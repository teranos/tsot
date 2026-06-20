return {
  id = "condemn",
  name = "Condemn",
  colors = {"white"},
  type = "spell",
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "kill target tapped non-white creature.",
  },
  on_play = function(game, self)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c and c.type == "creature" and c.tapped then
          local is_white = false
          if c.colors then
            for _, col in ipairs(c.colors) do
              if col == "white" then
                is_white = true
                break
              end
            end
          end
          if not is_white then
            table.insert(pool, iid)
          end
        end
      end
    end
    if #pool == 0 then return end
    game.set_intent("remove_threat")
    local target = game.choose_card(pool, {prompt = "condemn which tapped creature?"})
    if target then
      game.move(target, "graveyard")
    end
  end,
}
