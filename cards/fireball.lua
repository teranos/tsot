-- Damage-to-player is modeled as mill-to-graveyard from the opponent's deck.
-- B.2 already conflates combat damage with deck attrition; non-combat burn
-- on a player uses the same equivalence (mill, not exile).
return {
  id = "fireball",
  name = "Fireball",
  colors = {"red"},
  type = "spell",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "deal 4 damage to target creature or opponent.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then
      game.mill(opp, 4, "graveyard")
      return
    end
    game.set_intent("remove_threat")
    if game.confirm("aim at opponent? (no = burn a creature)") then
      game.mill(opp, 4, "graveyard")
      return
    end
    local target = game.choose_card(pool, {prompt = "deal 4 damage to"})
    if not target then return end
    game.damage(target, 4)
    local after = game.card(target)
    if after and after.y and 4 >= after.y then
      game.move(target, "graveyard")
    end
  end,
}
