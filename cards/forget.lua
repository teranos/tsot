return {
  id = "forget",
  name = "Forget",
  colors = {"purple"},
  type = "instant",
  cost = {
    {amount = 1, source = "mill"},
    {amount = 1, source = "attached"},
  },
  abilities = {
    "draw a card. exile target card from any graveyard.",
  },
  on_play = function(game, self)
    game.draw(self.owner, 1)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).graveyard) do
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "exile which card from graveyard?"})
    if target then
      game.move(target, "exile")
    end
  end,
}
