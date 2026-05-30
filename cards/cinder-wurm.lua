-- Red wurm 2/3 with a ping-on-hit trigger. Cost: 1 hand + 2 deck (= 2 mill
-- in the cost vocabulary; top 2 of own deck → graveyard).
--
-- The card text wants "whenever this creature deals damage to a player,
-- deal 2 damage to any target." In tsot, "damage to a player" = combat
-- damage that mills the defender (B.2). The clean trigger would be an
-- OnDealtDamageToPlayer event firing per attacker after combat resolves,
-- but that event doesn't exist yet.
--
-- Workaround: hook on_attack instead. Fires once per declared attack,
-- regardless of whether blockers come in or not. Semantic mismatch when
-- the wurm gets blocked (the trigger still fires even though no player
-- damage happens). Accepted as Phase 1 approximation — the unblocked case
-- (which matters most) works correctly, and the blocked case overshoots
-- slightly (free 2 damage on a blocked swing).
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
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "whenever this creature deals damage to a player, you may deal 2 damage to any target.",
  },
  stats = {x = 2, y = 3},
  on_attack = function(game, self)
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
