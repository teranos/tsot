-- 0-cost artifact. Tap and sacrifice it to drop Amsterdam City into play
-- without paying its cost.
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
