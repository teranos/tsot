-- Azure + white artifact. A vitrification vessel: when it enters the
-- board it pulls one creature off the field, freezes it in stasis
-- inside the chamber, and holds it there as long as the chamber sits.
-- When the chamber leaves play, the held creature thaws at the next
-- main phase and returns to the board.
--
-- TDD slice 1: card data only — colors / type / cost / holes. The ETB
-- exile-and-attach handler, the on-leave-play scheduler, and the
-- delayed-return primitive land in subsequent slices.
return {
  id = "cryogenic-chamber",
  name = "Cryogenic Chamber",
  type = "artifact",
  colors = {"white", "azure"},
  holes = {"L", "R", "T", "TR", "B", "BL"},
  cost = {
    {amount = 1, source = "graveyard"},
  },
  abilities = {
    "when this card enters the board, choose target creature on either board and attach it face-down to this card.",
    "when this card leaves play, return the attached card to its owner's board at the start of the next main phase (Main1 or Main2 of any player's turn, whichever comes first).",
  },
  flavor = "Vitrified. The clock waits with it.",
  on_enter_board = function(game, self)
    -- Build the pool: every creature on either player's board, excluding
    -- the chamber itself. Self-targeting is silently skipped — the
    -- chamber isn't a creature anyway, but we belt-and-suspenders the
    -- check for forward-compat (granted-type statics might lie).
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).board) do
        if iid ~= self.instance_id then
          local c = game.card(iid)
          if c and c.type == "creature" then
            table.insert(pool, iid)
          end
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {
      prompt = "Freeze a creature inside Cryogenic Chamber",
      optional = false,
    })
    if target then
      -- game.attach moves the target from its BOARD slot into the
      -- chamber's `attached` list, face-down per P.17. The chamber
      -- remembers the held card through its attached list.
      game.attach(self.instance_id, target)
    end
  end,
  on_die = function(game, self)
    -- The chamber is leaving the board (combat or otherwise). Each
    -- card it was holding gets queued for return at the next main
    -- phase (Main1 OR Main2 of any player's turn, whichever comes
    -- first). After this handler runs, P.8 cascades any remaining
    -- attached cards into EXILE — that's where the queued iids live
    -- until the turn loop flushes them back to their owner's board.
    for _, iid in ipairs(self.attached) do
      game.schedule_return_at_next_main(iid)
    end
  end,
}
