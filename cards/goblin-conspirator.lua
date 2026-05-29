-- Black goblin of the cycle: 1/1, 1 hand + 2 mill, on-play reveal + draw.
return {
  id = "goblin-conspirator",
  name = "Goblin Conspirator",
  colors = {"black"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "when you play this card you may reveal another goblin card from your hand. when you do, draw a card.",
  },
  stats = {x = 1, y = 1},
  on_play = function(game, self)
    if not game.confirm("reveal a goblin from your hand?") then
      return
    end
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).hand) do
      local c = game.card(iid)
      if c then
        for _, s in ipairs(c.subtypes) do
          if s == "goblin" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool > 0 then
      local target = game.choose_card(pool, { optional = false, prompt = "reveal a goblin" })
      if target then
        -- Revealing doesn't actually mutate the card; the reward is the draw.
        game.draw(self.owner, 1)
      end
    end
  end,
}
