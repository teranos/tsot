return {
  id = "axolotl",
  name = "Axolotl",
  colors = {"pink"},
  type = "creature",
  subtypes = {"axolotl"},
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "when this creature attacks, you may reveal a pink card from your hand. when you do, this creature gets +3/+0 until end of turn.",
  },
  stats = {x = -1, y = 1},
  on_attack = function(game, self)
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).hand) do
      local c = game.card(iid)
      if c then
        for _, col in ipairs(c.colors) do
          if col == "pink" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    if not game.confirm("reveal a pink card from hand?") then return end
    local target = game.choose_card(pool, { optional = false, prompt = "reveal a pink card" })
    if target then
      game.add_modifier(self.instance_id, "stat_boost", 3, 0, "end_of_turn")
    end
  end,
}
