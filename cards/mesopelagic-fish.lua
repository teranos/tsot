-- Blue 1/1 fish, 10 mill cost — a sacrificial graveyard-recursion engine.
-- The 10-mill cost is steep (gates it past turn ~5 when deck has milled
-- enough), the 1/1 body is throwaway, but the on_die ability lets you
-- recur a key non-creature spell (silent-murder, falter, opponent-draw,
-- beguile, etc.) from your graveyard.
--
-- on_die fires after the death is recorded and the creature is in the
-- graveyard. The handler pool excludes the fish itself (it's a creature,
-- filtered by `c.type ~= "creature"`).
--
-- Synergy:
--   - U-variant decks (blue-heavy) get the most out of this — they fill
--     graveyard with mill-cost casts (counterspell, draw-two, untap) and
--     recur their value spells.
--   - Excluded from G pool (no fish), so G doesn't benefit. Already-weak
--     G doesn't get pulled up further.
return {
  id = "mesopelagic-fish",
  name = "Mesopelagic fish",
  colors = {"blue"},
  type = "creature",
  subtypes = {"fish"},
  symbol = "⋈",
  cost = {{amount = 10, source = "mill"}},
  abilities = {
    "when this creature dies, you may return a non-creature card from your graveyard to your hand",
  },
  stats = {x = 1, y = 1},
  on_die = function(game, self)
    -- Build the pool BEFORE the may-confirm so we only prompt when there's
    -- actually something to recur. This is the pattern all "may" cards
    -- should follow — the oracle can't read the prompt string to decide
    -- whether confirming is meaningful, so cards have to be defensive.
    local gy = game.zones(self.owner).graveyard
    local pool = {}
    for _, iid in ipairs(gy) do
      local c = game.card(iid)
      if c and c.type ~= "creature" then
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then return end
    if not game.confirm("return a non-creature card from your graveyard?") then
      return
    end
    game.set_intent("recur")
    local target = game.choose_card(pool, {prompt = "return from graveyard"})
    if not target then return end
    game.move(target, "hand")
  end,
}
