-- 156 — 0-cost artifact tutor for Amsterdam City. T + self-sacrifice
-- fetches the environment straight to BOARD, bypassing its mill+graveyard
-- cost. SACRIFICE cost components inside activated abilities are not yet
-- supported (see LIMITATIONS.md "Deferred"), so the self-sac is folded
-- into the effect body via game.move(self, "graveyard") rather than
-- declared on the cost line.
return {
  id = "156",
  name = "156",
  type = "artifact",
  cost = {},
  abilities = {
    "T, sacrifice this: search your deck for Amsterdam City and put it on the board.",
  },
  on_enter_board = function(game, self)
    game.tap(self.instance_id)
  end,
  activated = {
    {
      cost = "tap",
      text = "T, sacrifice this: search your deck for Amsterdam City and put it on the board.",
      timing = "instant",
      effect = function(game, self)
        for _, iid in ipairs(game.zones(self.owner).deck) do
          local c = game.card(iid)
          if c and c.id == "amsterdam-city" then
            game.move_to(iid, self.owner, "board")
            break
          end
        end
        game.move(self.instance_id, "graveyard")
      end,
    },
  },
}
