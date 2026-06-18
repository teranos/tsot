-- TNF — red mutation. Tumor Necrosis Factor, an inflammatory cytokine the
-- host releases on contact during combat. Mechanic: whenever the host
-- creature attacks, every opposing creature takes 1 damage.
--
-- Currently dead code until the engine ships #6 (OnAttack iteration over
-- attacker's attached list, parallel to OnDealtDamageToPlayer's existing
-- shape at combat.rs:432). The combat.rs:170 attacker-side fire today
-- targets only the attacker iid; this handler needs the attached-list
-- iteration to receive the trigger.
return {
  id = "TNF",
  name = "TNF",
  type = "mutation",
  colors = {"red"},
  cost = {{amount = 1, source = "mill"}},
  abilities = {
    "the host creature gets: whenever this creature attacks, deal 1 damage to each opposing creature.",
  },
  flavor = "Cytokine storm on contact. Everything nearby burns.",
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
