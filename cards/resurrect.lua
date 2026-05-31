-- Black sorcery: target a creature card in any graveyard (yours or
-- opponent's) and bring it to your board with full ETB. Self-exile cost
-- means Resurrect itself joins the silence after casting — preventing
-- the Resurrect-into-Resurrect loop and limiting each copy to one use.
-- The reanimated creature lands under your control; owner stays the
-- original per T.2 (so if it dies later it goes to ITS owner's
-- graveyard, not yours).
--
-- First card to exercise the upgraded game.move_to: non-board → board
-- transitions now fire on_enter_board automatically, so "ETB effects
-- apply" is real, not aspirational. Reanimating goblin-scribe draws a
-- card; reanimating jellyfish bounces a creature; reanimating any
-- ETB-handler creature triggers it the same as a hard-cast play.
--
-- Cost: 1 hand + 1 mill + self-exile. Light hand+mill because the
-- dependency on a populated graveyard gates it past turns 1-2 anyway,
-- and self-exile makes every copy a one-shot — no chaining the same
-- card back via recast or another Resurrect.
return {
  id = "resurrect",
  name = "Resurrect",
  colors = {"black"},
  type = "spell",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
    {amount = 1, source = "self"},
  },
  abilities = {
    "Choose a creature card in any graveyard. Put it onto the battlefield under your control. ETB effects apply.",
  },
  on_play = function(game, self)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).graveyard) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "raise which creature?"})
    if not target then return end
    game.move_to(target, self.owner, "board")
  end,
}
