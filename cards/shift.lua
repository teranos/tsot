-- Green instant: pay X mill to move X attached cards from one
-- on-board card (source) to another (destination). Strategic uses:
--   - Steal opp's jewels onto your creature (Phase 3 grants follow)
--   - Consolidate your own attached jewels onto a single host
--   - Save attached cards from a dying creature by relocating them
-- The sim AI uses random targeting today; smart heuristics (response-
-- time triggering, prefer-steal scoring) are a future refinement.
--
-- X is read at on_play time via `game.x_value()` (set by play_card
-- before firing OnPlay). Movement uses `game.move_attached(from_host,
-- to_host, attached_iid)`.
return {
  id = "shift",
  name = "Shift",
  symbol = "⊨",
  colors = {"green"},
  type = "instant",
  cost = {{is_x = true, source = "mill"}},
  abilities = {
    "X mill: move X attached cards from target card to another target card.",
  },
  on_play = function(game, self)
    local x = game.x_value() or 0
    if x <= 0 then return end

    -- Pool of source candidates: any on-board card with attached cards.
    local own = self.owner
    local opp = game.opponent(own)
    local source_pool = {}
    for _, iid in ipairs(game.zones(own).board) do
      if #game.attached_of(iid) > 0 then table.insert(source_pool, iid) end
    end
    for _, iid in ipairs(game.zones(opp).board) do
      if #game.attached_of(iid) > 0 then table.insert(source_pool, iid) end
    end
    if #source_pool == 0 then return end

    local source = game.choose_card(source_pool, {prompt = "shift source"})
    if not source then return end

    local attached_list = game.attached_of(source)
    if #attached_list == 0 then return end

    -- Destination pool: any on-board card != source.
    local dest_pool = {}
    for _, iid in ipairs(game.zones(own).board) do
      if iid ~= source then table.insert(dest_pool, iid) end
    end
    for _, iid in ipairs(game.zones(opp).board) do
      if iid ~= source then table.insert(dest_pool, iid) end
    end
    if #dest_pool == 0 then return end

    local dest = game.choose_card(dest_pool, {prompt = "shift destination"})
    if not dest then return end

    -- Move up to X attached, picking each via choose_card so the
    -- oracle can score (or randomize per the active oracle). Each
    -- move shrinks the source's attached pool; refresh each iteration.
    local moved = 0
    while moved < x do
      local remaining = game.attached_of(source)
      if #remaining == 0 then break end
      local pick = game.choose_card(remaining, {prompt = "shift attached"})
      if not pick then break end
      game.move_attached(source, dest, pick)
      moved = moved + 1
    end
  end,
}
