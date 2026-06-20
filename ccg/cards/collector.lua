return {
  id = "collector",
  name = "Collector",
  colors = {"orange"},
  type = "creature",
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "when this creature enters the board, search your deck for a shiny card and move it to your hand.",
  },
  stats = {x = 1, y = 1},
  on_enter_board = function(game, self)
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c and c.face then
        for _, fa in ipairs(c.face) do
          if fa == "shiny" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    local pick = game.choose_card(pool, { optional = false, prompt = "tutor a shiny card" })
    if pick then
      game.move(pick, "hand")
    end
  end,
}
