-- Black artifact creature: 3/3 haste insect for 2 hand + 2 graveyard +
-- 2 mill. The flavor: your opponent can hack the dumb big mutated fly by
-- overwriting its Brain-Computer Interface. Mechanically: every time it
-- attacks (engine approximation of "deals damage to opponent" — see
-- caveat below), the opponent gets the option to sacrifice one of their
-- artifacts to take control of the fly.
--
-- Engine approximation:
--   The corpus lacks an `OnDealtDamageToPlayer` event today. Same gap as
--   cinder-wurm: the trigger uses on_attack, which fires whether the
--   attack was blocked or not. Net effect — the opponent can grab the
--   fly even if they blocked it. Design call: live with this. The
--   opponent only takes the deal if their artifact is worth less than
--   owning the 3/3 haste body; the gap mostly means the opponent gets
--   the option more often, not that the math is wrong.
--
-- Uses the new game.confirm_for / game.choose_card_for primitives so the
-- OPPONENT (not the card's owner) is asked.
--
-- The fly is artifact-typed so it routes through the no-summoning-sickness
-- play path. Haste keyword still goes through has_keyword and lets it
-- attack the turn it ETB. (Belt-and-suspenders: artifacts wouldn't get
-- summoning sickness even without haste, but if a future engine change
-- ties summoning sickness to creature-stats-presence, haste keeps the
-- design intent intact.)
return {
  id = "bci-megafly",
  name = "BCI MegaFly",
  colors = {"black"},
  type = "artifact",
  subtypes = {"insect"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "flying.",
    "haste.",
    "when this creature deals damage to an opponent, that opponent may sacrifice an artifact and gain control of this creature.",
  },
  stats = {x = 3, y = 4},
  on_attack = function(game, self)
    local opp = game.opponent(self.owner)
    -- Build pool of opponent's BOARD artifacts (excluding the fly — it's
    -- on the controller's side anyway, but guard for the edge case where
    -- a future controller-transfer sequence puts it on opp's board mid-trigger).
    local pool = {}
    for _, iid in ipairs(game.zones(opp).board) do
      if iid ~= self.instance_id then
        local c = game.card(iid)
        if c and c.type == "artifact" then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    if not game.confirm_for(opp, "sacrifice an artifact to gain control of bci-megafly?") then
      return
    end
    local victim = game.choose_card_for(opp, pool, {prompt = "sacrifice"})
    if not victim then return end
    game.move(victim, "graveyard")
    -- Transfer control: move the fly to the opponent's BOARD. game.move_to
    -- updates controller as a side effect (same primitive beguile uses).
    game.move_to(self.instance_id, opp, "board")
  end,
}
