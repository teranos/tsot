-- Black artifact creature: 0/1 for 1 hand. Two effects:
--
-- 1. Restriction static (Phase 3): opponent's flying creatures cannot
--    attack. Uses `affects.has_keyword = "flying"` to filter candidates
--    via `GameState::has_keyword` (intrinsic OR static-granted), and
--    `restrictions = {"cannot_attack"}` for the action choke point.
--
-- 2. ETB destroy: when scarecrow lands, you may pick an opponent's
--    creature whose colors share at least one with any card attached to
--    scarecrow, and destroy it. Uses `game.move(target, "graveyard")` —
--    note that this matches silent-murder's path and does NOT fire on_die
--    (a tsot-wide gap with non-combat destroys; same caveat as silent-murder).
--
-- Type is `artifact` so it routes through the artifact play path (no
-- summoning sickness). It still has stats so it can block. Power 0 means
-- it never deals damage in combat — pure defender/utility.
return {
  id = "scarecrow",
  name = "Scarecrow",
  colors = {"black"},
  type = "artifact",
  subtypes = {"scarecrow"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "creatures your opponents control with flying cannot attack.",
    "when this card enters the board, you may destroy target creature that shares a color with an attached card.",
  },
  stats = {x = 0, y = 1},
  static = {
    affects = {
      kind = "creature",
      controller = "opponent",
      has_keyword = "flying",
    },
    restrictions = {"cannot_attack"},
  },
  on_enter_board = function(game, self)
    -- Gather colors of cards attached to self.
    local attached_colors = {}
    for _, aid in ipairs(self.attached) do
      local c = game.card(aid)
      if c and c.colors then
        for _, col in ipairs(c.colors) do
          attached_colors[col] = true
        end
      end
    end
    if next(attached_colors) == nil then return end
    -- Find opponent creatures whose colors intersect.
    local opp = game.opponent(self.owner)
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" and c.colors then
        for _, col in ipairs(c.colors) do
          if attached_colors[col] then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    if not game.confirm("destroy a color-shared creature?") then return end
    local target = game.choose_card(pool, {prompt = "destroy", optional = false})
    if target then
      game.move(target, "graveyard")
    end
  end,
}
