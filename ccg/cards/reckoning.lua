return {
  id = "reckoning",
  name = "Reckoning",
  colors = {"black"},
  type = "spell",
  cost = {
    {is_x = true, source = "hand"},
    {is_x = true, source = "graveyard"},
    {is_x = true, source = "sacrifice"},
  },
  abilities = {
    "draw X cards. for every black card used to pay for this spell, target opponent sacrifices a creature.",
  },
  on_play = function(game, self)
    local x = game.x_value() or 0
    if x <= 0 then return end
    game.draw(self.owner, x)
    local pays = game.payment_ids()
    local black_count = 0
    local function count_black(list)
      for _, iid in ipairs(list) do
        local c = game.card(iid)
        if c and c.colors then
          for _, col in ipairs(c.colors) do
            if col == "black" then
              black_count = black_count + 1
              break
            end
          end
        end
      end
    end
    count_black(pays.hand)
    count_black(pays.attached)
    count_black(pays.graveyard)
    count_black(pays.mill)
    count_black(pays.sacrifice)
    if black_count <= 0 then return end
    local opp = game.opponent(self.owner)
    for i = 1, black_count do
      local pool = {}
      for _, iid in ipairs(game.zones(opp).board) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          table.insert(pool, iid)
        end
      end
      if #pool == 0 then break end
      -- Opponent chooses which of their creatures to sac (MTG convention).
      local pick = game.choose_card_for(opp, pool, { optional = false, prompt = "sacrifice a creature (" .. i .. "/" .. black_count .. ")" })
      if pick then
        game.move(pick, "graveyard")
      end
    end
  end,
}
