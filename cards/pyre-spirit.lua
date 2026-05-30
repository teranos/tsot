-- Red/white spirit. Cheap 2/3 with a conditional death payoff: when it
-- dies, IF the HAND-payment card attached to it was red, deal 4 damage
-- to any target. The rider rewards casting it with a red pitch — pair
-- with a red-coded hand or a red-jewel for the death sniper.
--
-- "Any target" simplified to "any opposing creature" — game.damage
-- works on creatures. Player targeting needs the mill API (different
-- shape) and isn't generalized yet.
--
-- on_die fires after the Board → Graveyard move. At that point,
-- self.attached still holds the attached payment iids (P.8 attached →
-- EXILE on host's death isn't yet implemented). So we can read the
-- attached cards' colors directly.
return {
  id = "pyre-spirit",
  name = "Pyre Spirit",
  colors = {"red", "white"},
  type = "creature",
  subtypes = {"spirit"},
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "when this creature dies, if the attached card was red, deal 4 damage to any target.",
  },
  stats = {x = 2, y = 3},
  on_die = function(game, self)
    if #self.attached == 0 then return end
    -- 1 hand cost = exactly 1 attached payment card at index 1.
    local attached_iid = self.attached[1]
    local c = game.card(attached_iid)
    if not c or not c.colors then return end
    local is_red = false
    for _, col in ipairs(c.colors) do
      if col == "red" then
        is_red = true
        break
      end
    end
    if not is_red then return end
    -- Build target pool: opposing creatures.
    local opp = game.opponent(self.owner)
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local card = game.card(iid)
      if card and card.type == "creature" then
        table.insert(pool, iid)
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "deal 4 damage to"})
    if target then
      game.damage(target, 4)
    end
  end,
}
