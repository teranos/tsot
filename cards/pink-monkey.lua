-- Pink Monkey — 1h, 2/2. First playable pink card.
-- Cost cycle: 1 hand to cast, 2 hand to activate. Activation bounces
-- one opposing creature back to its owner's hand. Bounce is a tempo
-- play — the opposing player has to spend resources to re-cast.
return {
  id = "pink-monkey",
  name = "Pink Monkey",
  colors = {"pink"},
  type = "creature",
  subtypes = {"monkey"},
  symbol = "am",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "2 hand: return target opposing creature to its owner's hand.",
  },
  stats = {x = 2, y = 2},
  activated = {
    {
      cost = {{amount = 2, source = "hand"}},
      text = "2 hand: return target creature to its owner's hand.",
      timing = "instant",
      validate = function(game, self)
        -- RULES A.9: needs an opposing creature on the board.
        local opp = game.opponent(self.owner)
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then return true end
        end
        return false
      end,
      effect = function(game, self)
        local opp = game.opponent(self.owner)
        local pool = {}
        for _, iid in ipairs(game.zones(opp).board) do
          local c = game.card(iid)
          if c and c.type == "creature" then table.insert(pool, iid) end
        end
        if #pool == 0 then return end
        local target = game.choose_card(pool, {prompt = "bounce to owner's hand"})
        if not target then return end
        local info = game.card(target)
        if info and info.owner then
          game.move_to(target, info.owner, "hand")
        end
      end,
    },
  },
}
