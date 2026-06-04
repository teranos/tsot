return {
  id = "read-the-embers",
  name = "Read the Embers",
  colors = {"red"},
  type = "spell",
  cost = {
    {is_x = true, source = "hand"},
    {is_x = true, source = "graveyard"},
    {is_x = true, source = "mill"},
  },
  abilities = {
    "draw X cards. for every red card used to pay for this spell, deal 1 damage to any target (each instance can target a different creature).",
  },
  on_play = function(game, self)
    local x = game.x_value() or 0
    if x <= 0 then return end
    game.draw(self.owner, x)
    -- Count red cards across every category that paid for this spell.
    -- game.payment_ids() returns { hand, attached, graveyard, mill } iids
    -- snapshotted at cost-resolution time, before this handler ran.
    local pays = game.payment_ids()
    local red_count = 0
    local function count_red(list)
      for _, iid in ipairs(list) do
        local c = game.card(iid)
        if c and c.colors then
          for _, col in ipairs(c.colors) do
            if col == "red" then
              red_count = red_count + 1
              break
            end
          end
        end
      end
    end
    count_red(pays.hand)
    count_red(pays.attached)
    count_red(pays.graveyard)
    count_red(pays.mill)
    if red_count <= 0 then return end
    -- "any target" simplified to "any opposing creature" — game.damage
    -- works on creatures; player targeting needs a different API shape
    -- (see pyre-spirit for the same pattern).
    local opp = game.opponent(self.owner)
    for i = 1, red_count do
      local pool = {}
      for _, iid in ipairs(game.zones(opp).board) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          table.insert(pool, iid)
        end
      end
      if #pool == 0 then return end
      local target = game.choose_card(pool, { optional = false, prompt = "deal 1 damage (" .. i .. "/" .. red_count .. ")" })
      if target then
        game.damage(target, 1)
      end
    end
  end,
}
