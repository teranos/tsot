-- Blue sorcery: take control of an opposing creature. Uses the same
-- cross-player move primitive (game.move_to) that opponent-draw uses —
-- the engine's controller-update + zone-transfer is already there.
--
-- In tsot, "control" means the creature lives on YOUR board: it untaps
-- on your turn, you can attack with it, and any controller-targeting
-- effects see you as the new controller. Owner stays on the original
-- player per RULES T.2 (immutable), so if the stolen creature later
-- moves to a non-board zone like graveyard or exile, the owner's
-- containers receive it.
--
-- Pool: opponent's board, all creatures (no flying/keyword restrictions).
-- 3 graveyard cost gates it past turn ~3-4.
return {
  id = "beguile",
  name = "Beguile",
  colors = {"blue"},
  type = "sorcery",
  cost = {{amount = 3, source = "graveyard"}},
  abilities = {
    "you gain control of target creature.",
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
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "gain control of"})
    if not target then return end
    game.move_to(target, self.owner, "board")
  end,
}
