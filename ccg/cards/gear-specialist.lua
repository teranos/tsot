return {
  id = "gear-specialist",
  name = "Gear Specialist",
  colors = {"orange"},
  type = "creature",
  subtypes = {"human", "mechanic"},
  symbols = {"⨳", "⋈"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 3, source = "graveyard"},
  },
  abilities = {
    "when this creature enters the board, return target artifact to its owner's hand.",
  },
  stats = {x = 2, y = 1.5},
  on_enter_board = function(game, self)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c and c.type == "artifact" then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    local pick = game.choose_card(pool, { optional = false, prompt = "bounce an artifact" })
    if pick then
      game.move(pick, "hand")
    end
  end,
}
