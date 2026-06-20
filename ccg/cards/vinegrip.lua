-- Green spell: 1 graveyard. Counters a non-creature on the stack.
-- Targets any chain item whose card type isn't creature (i.e., spells,
-- artifacts, environments, mutations). Useful against burn / removal /
-- artifact storm while leaving creatures alone (which die to damage).
return {
  id = "vinegrip",
  name = "Vinegrip",
  colors = {"green"},
  type = "instant",
  cost = {{amount = 1, source = "graveyard"}},
  abilities = {
    "counter target non-creature card on the stack.",
  },
  target = "chain",
  on_play = function(game, self)
    local chain = game.chain()
    if #chain == 0 then return end
    local pool = {}
    for _, item in ipairs(chain) do
      local c = game.card(item.card)
      if c and c.type ~= "creature" then
        table.insert(pool, item.card)
      end
    end
    if #pool == 0 then return end
    game.set_intent("remove_threat")
    local target = game.choose_card(pool, {prompt = "counter which non-creature?"})
    if target then
      game.counter(target)
    end
  end,
}
