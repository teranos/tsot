-- Red devil 1/3 — sweep-the-board ETB. Sacrifice cost forces tribute
-- on play (one of your own creatures); on entry, pings every opposing
-- creature for 1. Useful against wide low-toughness boards (clears
-- 1-toughness flyers, midnight-raven, mortal-bee's relatives, etc.)
-- but doesn't touch your own side.
return {
  id = "red-devil",
  name = "Red Devil",
  type = "creature",
  colors = {"red"},
  subtypes = {"devil"},
  cost = {
    {amount = 1, source = "sacrifice", kind = "creature"},
  },
  stats = {x = 1, y = 3},
  abilities = {
    "when this creature enters the board, deal 1 damage to each opposing creature.",
  },
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        game.damage(iid, 1)
      end
    end
  end,
}
