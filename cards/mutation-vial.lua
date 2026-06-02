-- Transparent artifact (Vial subtype). Tap + self-sacrifice tutors a
-- mutation card from your deck to your hand.
--
-- A.8 reserves SACRIFICE / SELF as activation cost components, so the
-- "sacrifice" half lives inside the effect handler: the activation
-- pays tap only, then the handler moves the vial from BOARD to
-- GRAVEYARD and tutors. Functionally identical, just doesn't lean on
-- an unwired cost component.
--
-- Reading "1yg" as 1 graveyard + 1 mill. The graveyard pitch must
-- color-match (P.12a) — a transparent-color-only vial requires a
-- transparent pitch in GY (clear-* in graveyard satisfies this).
return {
  id = "mutation-vial",
  name = "Mutation Vial",
  colors = {"transparent"},
  type = "artifact",
  subtypes = {"vial"},
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "T: sacrifice this card; search your deck for a mutation card and put it in your hand.",
  },
  flavor = "Specimen jar, freshly emptied.",
  activated = {
    {
      cost = "tap",
      text = "T: sacrifice this card; search for a mutation in your deck.",
      timing = "instant",
      effect = function(game, self)
        -- Sacrifice: BOARD → GRAVEYARD. The activation already paid
        -- the tap, but the source is still on its controller's BOARD;
        -- move it out before the tutor so it doesn't pollute the
        -- attached/zone state.
        game.move(self.instance_id, "graveyard")
        -- Tutor: find the first mutation in controller's DECK and
        -- move it to HAND. Stops at the first hit; engine has no
        -- shuffle primitive, so subsequent calls deterministically
        -- return the next mutation in deck order.
        for _, iid in ipairs(game.zones(self.owner).deck) do
          local c = game.card(iid)
          if c and c.type == "mutation" then
            game.move(iid, "hand")
            return
          end
        end
      end,
    },
  },
}
