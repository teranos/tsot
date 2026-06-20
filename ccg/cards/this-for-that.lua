-- Pink sorcery: take control of an opposing creature in exchange for
-- giving up one of your non-creature board cards (a jewel, artifact, or
-- environment — anything on your BOARD that isn't a creature). Permanent
-- control on both sides: you keep the creature, opp keeps the non-
-- creature. The asymmetry is the design — you give the opp something
-- whose value depends on color/synergy they may not have, in exchange
-- for whatever creature they were committing to.
--
-- Pink-ness vs beguile: beguile is single-currency theft with no give-
-- back. "This for That" requires a real board commitment as the price —
-- you give up an artifact's ongoing value (a jewel's tap-engine, LCD
-- Clock's discount, methylene-blue's blue-card cost reduction) in
-- exchange for the creature. Costs 1 hand on top, matching blue's
-- economy bracket.
--
-- Subtle interaction worth flagging: the activated abilities on jewels
-- (and similar artifacts) read self.owner per the engine convention
-- (T.2 — owner is immutable). So if opp taps a jewel you gave them, the
-- draw + discard fire on YOU as the original owner — they wasted a tap
-- to help you cycle. That's the "Thanks for nothing" flavor literally
-- realized at the engine level. The give-away is a near-empty gift.
--
-- Fizzles cleanly if either side of the swap is impossible (no non-
-- creature on your board to give, or no creature on opp's board to
-- take). Both checks happen in the handler before any movement.
--
-- Symbol not yet specified.
return {
  id = "this-for-that",
  name = "This for That",
  symbol = "꩜",
  colors = {"pink"},
  type = "sorcery",
  cost = {
    {amount = 1, source = "hand"},
  },
  abilities = {
    "Give an opponent a non-creature card from your board. Gain control of target creature they control.",
  },
  flavor = "Thanks for nothing.",
  on_play = function(game, self)
    local owner = self.owner
    local opp = game.opponent(owner)

    -- Pool 1: non-creatures on owner's board.
    local givables = {}
    for _, iid in ipairs(game.zones(owner).board) do
      local c = game.card(iid)
      if c and c.type ~= "creature" then
        table.insert(givables, iid)
      end
    end
    if #givables == 0 then return end

    -- Pool 2: creatures on opp's board.
    local creatures = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.type == "creature" then
        table.insert(creatures, iid)
      end
    end
    if #creatures == 0 then return end

    game.set_intent("low_value_own")
    local give = game.choose_card(givables, {prompt = "give which non-creature to opponent?"})
    if not give then return end
    game.set_intent("remove_threat")
    local take = game.choose_card(creatures, {prompt = "take which creature?"})
    if not take then return end

    -- Execute the swap. Both are board→board moves, so move_to does NOT
    -- fire ETB and does NOT re-apply summoning sickness — same semantic
    -- as beguile's controller-only swap.
    game.move_to(give, opp, "board")
    game.move_to(take, owner, "board")
  end,
}
