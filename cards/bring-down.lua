-- Purple instant: -3/-3 until end of turn on a target creature, with a
-- goblin-tribal rider that cantrips when you control a goblin.
--
-- Implementation note on "-3/-3 until end of turn":
--   tsot has no time-bound modifier system today — `Modifier::StatBoost`
--   is permanent. The closest mechanical proxy is `game.damage(target, 3)`:
--     - Damage clears at end of turn naturally (B.10).
--     - If the creature's effective Y <= 3, it dies via the death check.
--     - Power reduction (-3 X) is NOT modeled — damage doesn't touch X.
--   So this handler captures the lethal toughness case but not the
--   "make a 4/4 into a 1/1 for blocking math" case. When temporary
--   modifiers land (STATIC Phase 2-ish), revisit.
--
-- The goblin rider only fires when you control a goblin on board. Uses
-- the defensive-may pattern: check the condition before the confirm.
return {
  id = "bring-down",
  name = "Bring Down",
  colors = {"purple"},
  type = "instant",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "target creature gets -3/-3 until end of turn.",
    "if you control a goblin you may draw a card.",
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
    if #pool > 0 then
      local target = game.choose_card(pool, {prompt = "bring down"})
      if target then
        game.damage(target, 3)
      end
    end
    -- Goblin rider: check own board for any goblin
    local has_goblin = false
    for _, iid in ipairs(game.zones(self.owner).board) do
      local c = game.card(iid)
      if c and c.subtypes then
        for _, st in ipairs(c.subtypes) do
          if st == "goblin" then
            has_goblin = true
            break
          end
        end
      end
      if has_goblin then break end
    end
    if has_goblin and game.confirm("draw a card for goblin tribute?") then
      game.draw(self.owner, 1)
    end
  end,
}
