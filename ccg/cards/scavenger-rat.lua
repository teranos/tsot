-- Colorless rat. Smaller body than sewer-rat (2/1 vs 3/2) for the same
-- 1H+2M cost, trading raw stats for a tribal-pitch upside: when you
-- cast scavenger-rat, the card you pitched as the hand-payment may be
-- revealed; if it's a rat, you may draw. Pairs with the rat-recursion
-- pack-rat ability — pitching another rat into scavenger-rat refunds
-- the cantrip immediately AND keeps the rat alive as attached-substrate
-- for pack-rat to bring home on death.
--
-- "Can't block cats" stays on-template with the rest of the rat tribe.
-- Colorless makes scavenger-rat splashable into any deck — it doesn't
-- conflict with on-color cost-discount cards (methylene-blue / LCD-Clock)
-- and it doesn't ask any zebra/jewel synergies to care about a color
-- match for the pitch.
return {
  id = "scavenger-rat",
  name = "Scavenger Rat",
  symbol = "⋈",
  type = "creature",
  colors = {},
  subtypes = {"rat"},
  cannot_block_subtypes = {"cat"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  stats = {x = 2, y = 1},
  abilities = {
    "can't block cats.",
    "when this creature enters the board, you may reveal an attached card. if it is a rat, you may draw a card.",
  },
  on_enter_board = function(game, self)
    if #self.attached == 0 then return end
    if not game.confirm("reveal the attached card?") then return end
    -- 1H+2M cost: only the hand payment attaches (mill goes to graveyard
    -- per P.11). So self.attached[1] is the single attached card.
    local aid = self.attached[1]
    local a = game.card(aid)
    if not a or not a.subtypes then return end
    local is_rat = false
    for _, s in ipairs(a.subtypes) do
      if s == "rat" then
        is_rat = true
        break
      end
    end
    if not is_rat then return end
    if game.confirm("revealed a rat — draw a card?") then
      game.draw(self.owner, 1)
    end
  end,
}
