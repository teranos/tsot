-- Purple instant: -3/-3 until end of turn on a target creature, plus a
-- goblin-tribal cantrip. Uses Phase 1.5 temporary stat modifiers — the
-- earlier game.damage proxy is replaced with the real -X/-Y mechanism.
--
-- Behavior:
--   - Apply Modifier::EotStatBoost{x=-3, y=-3} to the chosen target.
--   - Re-read the target's effective stats. If Y <= 0, manually move it
--     to graveyard (tsot has no state-based-action loop, so handlers
--     that drop a creature's toughness to 0 must enforce death directly;
--     same pattern silent-murder uses).
--   - At end of turn, the engine strips EOT modifiers — surviving
--     creatures return to their original stats automatically.
--
-- The goblin rider only fires when you control a goblin on board.
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
        game.add_modifier(target, "stat_boost", -3, -3, "end_of_turn")
        -- Manual death check: tsot has no SBA loop, so we look at the
        -- post-modifier effective Y and kill if it dropped to 0 or below.
        local after = game.card(target)
        if after and after.y <= 0 then
          game.move(target, "graveyard")
        end
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
