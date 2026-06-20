-- Azure sorcery. Target player takes an extra turn after this one.
-- Heavy cost (2 hand + 4 attached + 6 graveyard). Caster picks the
-- target via choose_player; can target self or opponent.
return {
  id = "azure-recursion",
  name = "Azure Recursion",
  colors = {"azure"},
  type = "spell",
  cost = {
    {amount = 2, source = "hand"},
    {amount = 4, source = "attached"},
    {amount = 6, source = "graveyard"},
  },
  abilities = {
    "target player takes an extra turn after this one.",
  },
  on_play = function(game, self)
    local target = game.choose_player({
      prompt = "who takes the extra turn?",
    })
    if target then
      game.grant_extra_turn(target)
    end
  end,
}
