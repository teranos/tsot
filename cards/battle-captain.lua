-- Lord ability (+1/+1 to other humans) deferred: needs the static system
-- (LUA Phase 2 continuous effects). The on_attack mass-untap is Phase 1.
return {
  id = "battle-captain",
  name = "Battle Captain",
  colors = {"white"},
  type = "creature",
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "all other humans you control get +1/+1.",
    "whenever this creature attacks, untap all other creatures you control that are attacking.",
  },
  stats = {x = 2, y = 2},
  on_attack = function(game, self)
    for _, iid in ipairs(game.attackers()) do
      if iid ~= self.instance_id then
        game.untap(iid)
      end
    end
  end,
}
