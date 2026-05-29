-- Symbol not yet specified.
--
-- The two other abilities ("defender" and the insects-suppression static)
-- are unwired:
--   - The static "insects your opponents control cannot attack or be used
--     as a cost paid" needs the static-effect system (Phase 2 LUA `static`).
--   - The card also has a SACRIFICE cost, so it can't be played through
--     `play_card` yet (costs theme). The on_die handler still fires
--     correctly from any state where the card sits on the BOARD.
return {
  id = "flesh-eating-plant",
  name = "Flesh-eating Plant",
  colors = {"red", "green"},
  type = "creature",
  subtypes = {"plant"},
  cost = {{amount = 1, source = "sacrifice"}},
  abilities = {
    "defender.",
    "insects your opponents control cannot attack or be used as a cost paid.",
    "When this creature dies you may return an insect card from your graveyard to your hand.",
  },
  stats = {x = 1, y = 2},
  on_die = function(game, self)
    if not game.confirm("return an insect from your graveyard?") then
      return
    end
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).graveyard) do
      local c = game.card(iid)
      if c then
        for _, s in ipairs(c.subtypes) do
          if s == "insect" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool > 0 then
      local target = game.choose_card(pool, { optional = false, prompt = "return an insect" })
      if target then
        game.move(target, "hand")
      end
    end
  end,
}
