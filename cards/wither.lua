return {
  id = "wither",
  name = "Wither",
  colors = {"purple"},
  type = "spell",
  cost = {
    {is_x = true, source = "hand"},
    {is_x = true, source = "graveyard"},
    {is_x = true, source = "attached"},
  },
  abilities = {
    "draw X cards. for every purple card used to pay for this spell, put a -1/-1 counter on any target (each instance can target a different creature).",
  },
  on_play = function(game, self)
    local x = game.x_value() or 0
    if x <= 0 then return end
    game.draw(self.owner, x)
    local pays = game.payment_ids()
    local purple_count = 0
    local function count_purple(list)
      for _, iid in ipairs(list) do
        local c = game.card(iid)
        if c and c.colors then
          for _, col in ipairs(c.colors) do
            if col == "purple" then
              purple_count = purple_count + 1
              break
            end
          end
        end
      end
    end
    count_purple(pays.hand)
    count_purple(pays.attached)
    count_purple(pays.graveyard)
    count_purple(pays.mill)
    if purple_count <= 0 then return end
    -- "any target" simplified to "any opposing creature" — same
    -- restriction as read-the-embers' damage targeting.
    local opp = game.opponent(self.owner)
    for i = 1, purple_count do
      local pool = {}
      for _, iid in ipairs(game.zones(opp).board) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          table.insert(pool, iid)
        end
      end
      if #pool == 0 then return end
      local target = game.choose_card(pool, { optional = false, prompt = "-1/-1 counter (" .. i .. "/" .. purple_count .. ")" })
      if target then
        game.add_modifier(target, "stat_boost", -1, -1)
      end
    end
  end,
}
