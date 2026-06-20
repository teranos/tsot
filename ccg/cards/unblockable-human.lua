-- 0/1 unblockable human. Design intent: "always correct to attack." The
-- body normally deals 0 damage (X = 0) but the pump line takes that to
-- 2 per swing by exiling a graveyard card. Combat is risk-free because
-- nothing can block unblockable.
--
-- Two wired on_attack effects:
--   1. Pump: if you have a graveyard card to spend, exile one and pump
--      +2/+0 until end of turn (Phase 1.5 temporary-modifier system).
--      Mills 2 from the defender this turn instead of 0.
--   2. Loot: discard one + draw one. Free hand filtering.
--
-- The "may" wording on both is courtesy. The AI takes both whenever
-- conditions allow because there's never a line where passing is better.
--
-- AI's `is_attack_worth_declaring` short-circuits to TRUE on unblockable
-- so this creature attacks every turn it's legal.
--
-- Symbol not yet specified.
return {
  id = "unblockable-human",
  name = "Unblockable Human",
  type = "creature",
  colors = {"blue"},
  subtypes = {"human"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "unblockable.",
    "when this creature attacks you may exile a card from your graveyard; if you do, this creature gets +2/+0 until end of turn.",
    "when this creature attacks a player you may discard a card and draw a card.",
  },
  stats = {x = 0, y = 1},
  on_attack = function(game, self)
    -- Pump: exile a graveyard card → +2/+0 UEOT. Front-of-graveyard
    -- pick (no smart-pick API today; the cards being exiled here are
    -- in the OWNER's graveyard so the AI picks whatever's most accessible).
    local gy = game.zones(self.owner).graveyard
    if #gy > 0 then
      game.move(gy[1], "exile")
      game.add_modifier(self.instance_id, "stat_boost", 2, 0, "end_of_turn")
    end
    -- Loot: always take it. Strictly free filtering.
    game.discard(self.owner, 1)
    game.draw(self.owner, 1)
  end,
}
