-- δ-subunit polymerase. Each turn the host's owner draws cards equal to
-- the host's toughness. Reads host live so the polymerase tracks owners
-- across donate/steal — the original caster keeps drawing.
return {
  id = "DNA-DIRECTED-DNA-POLYMERASE",
  name = "DNA-DIRECTED DNA POLYMERASE",
  type = "mutation",
  colors = {"blue", "green"},
  symbol = "δ",
  cost = {},
  abilities = {
    "the host creature gets: at the beginning of your turn, draw cards equal to this creature's toughness.",
  },
  on_turn_begin = function(game, self)
    local host = game.host_of(self.instance_id)
    if host == nil then return end
    local host_view = game.card(host)
    if host_view == nil then return end
    local n = math.floor(host_view.y or 0)
    if n > 0 then
      game.draw(host_view.owner, n)
    end
  end,
}
