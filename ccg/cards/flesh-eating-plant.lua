-- Symbol not yet specified.
--
-- Defender + the insects-suppression static + the SACRIFICE cost.
-- Cost is still un-routable through `play_card` (SACRIFICE source not
-- supported), but the static fires whenever the card is on BOARD, however
-- it got there (e.g., a future "return from graveyard" path).
--
-- The static uses STATIC Phase 3 restrictions: `cannot_attack` blocks
-- declare_attacker for opponent insects, `cannot_be_cost_paid` filters
-- them out of HAND-payment pools in resolve_hand_payment.
return {
  id = "flesh-eating-plant",
  name = "Flesh-eating Plant",
  colors = {"red", "green"},
  type = "creature",
  subtypes = {"plant"},
  cost = {{amount = 1, source = "sacrifice"}},
  abilities = {
    "defender.",
    "insects your opponents control cannot attack or be used as a cost paid.",
    "When this creature dies you may return an insect card from your graveyard to your hand.",
  },
  static = {
    affects = {
      subtypes = {"insect"},
      controller = "opponent",
    },
    restrictions = {"cannot_attack", "cannot_be_cost_paid"},
  },
  stats = {x = 1, y = 2},
  on_die = function(game, self)
    if not game.confirm("return an insect from your graveyard?") then
      return
    end
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).graveyard) do
      local c = game.card(iid)
      if c then
        for _, s in ipairs(c.subtypes) do
          if s == "insect" then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool > 0 then
      local target = game.choose_card(pool, { optional = false, prompt = "return an insect" })
      if target then
        game.move(target, "hand")
      end
    end
  end,
}
