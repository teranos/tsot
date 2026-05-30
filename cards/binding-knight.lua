return {
  id = "binding-knight",
  name = "Binding Knight",
  colors = {"white"},
  type = "creature",
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "whenever this creature attacks, tap an untapped creature an opponent controls.",
  },
  stats = {x = 2, y = 2},
  on_attack = function(game, self)
    local opp = game.opponent(self.owner)
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and not c.tapped then
        game.tap(iid)
        break
      end
    end
  end,
}
