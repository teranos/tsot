-- Pink ghost. Two zoned activations: from attached → tutor to board,
-- from graveyard → tutor to hand.
return {
  id = "pink-ghost",
  name = "Pink Ghost",
  colors = {"pink"},
  symbol = "≡",
  type = "creature",
  subtypes = {"ghost"},
  holes = {"UL", "TL", "B", "BR"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  stats = {x = 1, y = 1},
  abilities = {
    "while attached, exile this: search your deck for a pink symbol card and put it on the board.",
    "while in your graveyard, exile this: search your deck for a pink symbol card and put it in your hand.",
  },
  activated = {
    {
      cost = {{source = "self", amount = 1}},
      text = "while attached, exile this: tutor a pink symbol to the board.",
      timing = "instant",
      from_zones = {"attached"},
      effect = function(game, self)
        local pool = {}
        for _, iid in ipairs(game.zones(self.owner).deck) do
          local c = game.card(iid)
          if c and c.type == "symbol" and c.colors then
            for _, col in ipairs(c.colors) do
              if col == "pink" then
                table.insert(pool, iid)
                break
              end
            end
          end
        end
        if #pool == 0 then return end
        local picked = game.choose_card(pool, {optional = true, prompt = "pink symbol → board"})
        if picked == nil then return end
        game.move_to(picked, self.owner, "board")
      end,
    },
    {
      cost = {{source = "self", amount = 1}},
      text = "while in your graveyard, exile this: tutor a pink symbol to hand.",
      timing = "instant",
      from_zones = {"graveyard"},
      effect = function(game, self)
        local pool = {}
        for _, iid in ipairs(game.zones(self.owner).deck) do
          local c = game.card(iid)
          if c and c.type == "symbol" and c.colors then
            for _, col in ipairs(c.colors) do
              if col == "pink" then
                table.insert(pool, iid)
                break
              end
            end
          end
        end
        if #pool == 0 then return end
        local picked = game.choose_card(pool, {optional = true, prompt = "pink symbol → hand"})
        if picked == nil then return end
        game.move(picked, "hand")
      end,
    },
  },
}
