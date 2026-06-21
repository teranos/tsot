-- Tumor Necrosis Factor — red gene. Inflammatory cytokine: the host
-- releases damage on contact, scorching every opposing creature.
return {
  id = "TNF",
  name = "TNF",
  type = "mutation",
  colors = {"red"},
  cost = {{amount = 1, source = "mill"}},
  abilities = {
    "the host creature gets: whenever this creature attacks, deal 1 damage to each opposing creature.",
  },
  flavor = "Cytokine storm on contact.",
  on_attack = function(game, self)
    local host = game.host_of(self.instance_id)
    if host == nil then return end
    local host_view = game.card(host)
    if host_view == nil then return end
    local opp = game.opponent(host_view.controller)
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        game.damage(iid, 1)
      end
    end
  end,
}
