-- Black instant: strip the cost-attachments off a creature, exiling them.
-- Doesn't kill the creature — just removes the investment. Hydra-style
-- cards that scale with attached count are the prime victim: a 6/6 hydra
-- with 5 attached cards becomes a base 1/1 instantly.
--
-- Effect path: pick a card on either board with at least one attached,
-- exile every attached card. The host creature stays. Modifiers granted
-- by ETB-based static reads of the attached count (like hydra) DON'T
-- update automatically today — the ETB modifier is sticky once applied.
-- Static-driven recomputation is a STATIC.md Phase 1.5+ concern; for now
-- falter still strips the attached cards (visible in game.zones) but
-- hydra's printed bonus persists. Worth a follow-up retrofit.
--
-- Symbol not yet specified.
return {
  id = "falter",
  name = "Falter",
  colors = {"black"},
  type = "instant",
  abilities = {
    "exile all cards attached to target card.",
  },
  on_play = function(game, self)
    -- Build pool: any on-board card (either side) with at least one attached.
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        local c = game.card(iid)
        if c and c.attached and #c.attached > 0 then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "strip attached from"})
    if not target then return end
    local tcard = game.card(target)
    if not tcard or not tcard.attached then return end
    -- Snapshot the attached list before mutating (game.move pops it).
    local victims = {}
    for _, iid in ipairs(tcard.attached) do
      table.insert(victims, iid)
    end
    for _, iid in ipairs(victims) do
      game.move(iid, "exile")
    end
  end,
}
