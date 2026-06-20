-- Red instant: cheap, flexible burn at the bottom of the red curve.
-- Cost: 1 graveyard + 1 mill — two sources red doesn't lean on (most
-- red is mill+hand). Pushes the deck toward graveyard recycling shapes.
--
-- Deals 2 damage to a target opposing creature. tsot has no state-
-- based-action loop (combat.rs:392-396 TODO), so handlers that should
-- kill via accumulated damage must do it directly — same pattern bring-
-- down uses for negative-Y stat hits. We approximate: if our 2 damage
-- is enough on its own to drop a fresh creature (effective Y ≤ 2),
-- kill manually. Pre-damaged creatures are slightly under-counted, but
-- damage clears at end of turn anyway so the window is one main phase.
--
-- DESIGN INTENT (deferred): the original card had a "portable" rider —
-- "if attached to a red creature you control, exile this card and deal
-- 3 damage to a creature." Two engine blockers:
--   - Spells resolve to GRAVEYARD before on_play fires (play.rs:569).
--     No "instant becomes attached" transition exists.
--   - Activated abilities (T:) aren't wired (LIMITATIONS.md). A "you
--     may exile" player-choice effect cannot fire today.
-- Shipping v1 as the bare bolt; revisit the portable rider when either
-- the attach-from-spell path or the activated-ability layer lands.

return {
  id = "portable-bolt",
  name = "Portable Bolt",
  colors = {"red"},
  type = "instant",
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "deal 2 damage to a creature.",
  },
  flavor = "Small enough to carry. Loud enough to hear three rooms over.",
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "deal 2 damage to"})
    if not target then return end
    game.damage(target, 2)
    -- Manual SBA: kill if our 2 damage alone is enough.
    local after = game.card(target)
    if after and after.y and 2 >= after.y then
      game.move(target, "graveyard")
    end
  end,
}
