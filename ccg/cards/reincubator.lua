-- Green/black artifact. Cuts hand + graveyard cost by 1 for any creature
-- with green or black in its colors. On ETB, scales up the sacrifice
-- into a bigger creature from your deck.
return {
  id = "reincubator",
  name = "Reincubator",
  type = "artifact",
  colors = {"green", "black"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "sacrifice", kind = "creature"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "static: any creature whose colors include green or black costs 1 hand and 1 graveyard less to cast (everywhere, both players). the bonus does not stack across colors.",
    "when this enters the board: you may search your deck for a creature whose combined power+toughness is up to 2 higher than the sacrificed creature's, and put it on the board.",
    "T, exile this, sacrifice a creature: you may search your deck for a creature whose combined power+toughness is up to 2 higher than the sacrificed creature's, and put it on the board.",
  },
  static = {
    affects = {
      kind = "creature",
      colors = {"green", "black"},
    },
    cost_modifiers = {
      {source = "hand", amount = 1},
      {source = "graveyard", amount = 1},
    },
  },
  on_enter_board = function(game, self)
    local pay = game.payment_ids()
    local sac_iids = pay.sacrifice
    if sac_iids == nil or #sac_iids == 0 then return end
    local sac_view = game.card(sac_iids[1])
    if sac_view == nil then return end
    local threshold = (sac_view.x or 0) + (sac_view.y or 0) + 2

    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c ~= nil and c.type == "creature" then
        local pt = (c.x or 0) + (c.y or 0)
        if pt <= threshold then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end

    local picked = game.choose_card(pool, {
      optional = true,
      prompt = "reincubate: combined p+t ≤ " .. threshold,
    })
    if picked == nil then return end
    game.move_to(picked, self.owner, "board")
  end,
}
