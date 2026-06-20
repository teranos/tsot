-- Red wurm 2/3 with a ping-on-hit trigger. Cost: 2 mill + 1 sacrifice creature.
--
-- The card text wants "whenever this creature deals damage to a player,
-- you may deal 2 damage to any target." In tsot, "damage to a player" =
-- combat damage that mills the defender (B.2). Hooked via
-- `on_dealt_damage_to_player`, which fires post-combat on every attacker
-- that successfully milled the defender's deck (combat.rs:444). Migration
-- from the earlier `on_attack` workaround (2026-06-20): the workaround
-- fired on every declared attack including blocked ones, overshooting
-- on blocked swings; the new event only fires on unblocked attacks that
-- actually landed damage.
--
-- "Any target" simplified to "any opposing creature" — game.damage works
-- on creatures. Player damage = mill which is a different API. Player
-- targets can come back when an effect-targeting layer exists.
return {
  id = "cinder-wurm",
  name = "Cinder Wurm",
  colors = {"red"},
  type = "creature",
  subtypes = {"wurm"},
  cost = {
    {amount = 2, source = "mill"},
    {amount = 1, source = "sacrifice", kind = "creature"},
  },
  abilities = {
    "whenever this creature deals damage to a player, you may deal 2 damage to any target.",
  },
  stats = {x = 2, y = 3},
  on_dealt_damage_to_player = function(game, self)
    local opp = game.opponent(self.owner)
    local creatures = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        table.insert(creatures, iid)
      end
    end
    if #creatures == 0 then return end
    if not game.confirm("ping with cinder wurm?") then return end
    local target = game.choose_card(creatures, {prompt = "deal 2 damage to"})
    if target then
      game.damage(target, 2)
    end
  end,
}
