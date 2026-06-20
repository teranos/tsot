-- Black instant: the dark mirror of trustworthy-lender. Where the lender
-- voluntarily returns its attached payment to its owner on death,
-- Foreclosure liquidates someone else's collateral early — and to the
-- wrong party. One attached card on an opposing on-board creature moves
-- to your hand. Owner stays the opponent per T.2 (immutable); controller
-- becomes you. The opp lost a deck card permanently — if you discard or
-- pitch it later, it goes to YOUR graveyard but originated from their
-- pool. You gained a hand card from out of nowhere.
--
-- Composed from two existing primitives:
--   - falter's pool-walk (cards on board with non-empty attached) and the
--     snapshot-attached pattern.
--   - beguile's game.move_to(target, self.owner, "<zone>") for the
--     cross-player transfer with controller update.
--
-- Targets opp's side only (no self-steal — moving your own attached card
-- to your own hand is bizarre and we're not in that business). Trivially
-- no-ops if the opponent has no attached cards on board.
--
-- Cost: 1 graveyard, 0 hand. Conditional on opp having attached cards
-- AND on your graveyard being non-empty, so the cost cannot be paid on
-- turn 1. Cheap because the effect fizzles entirely against decks that
-- don't lean on attached economy — it's an archetype-answer, not
-- universal removal.
--
-- Synergy paths the corpus enables today:
--   - Catches trustworthy-lender's stash before the lender dies — the
--     lender controller paid the hand cost expecting a refund; you pocket
--     it instead. Lender becomes a 2/2 vanilla with no payoff.
--   - Strips an attached card from hydra / reef-phantom; with STATIC
--     Phase 1.5 the effective stats recompute, so the host shrinks by
--     +1/+1 on the spot. Surgical, not the all-strip swing falter does.
--   - Lifts a same-color jewel onto your hand to pitch into your own
--     matching-color creature for the +1/+1 OnAttachedAsCost grant.
--     Their payment, your buff.
--   - Steals a companion-bird mid-attach so the host loses flying.
--
-- Symbol not yet specified.
return {
  id = "foreclosure",
  name = "Foreclosure",
  colors = {"black"},
  type = "instant",
  cost = {
    {amount = 1, source = "graveyard"},
  },
  abilities = {
    "Steal an attached card from a target opposing creature; put it into your hand.",
  },
  flavor = "The borrower is servant to the lender.",
  on_play = function(game, self)
    local opp = game.opponent(self.owner)
    -- Pool: opponent's on-board cards with at least one attached.
    local victims = {}
    for _, iid in ipairs(game.zones(opp).board) do
      local c = game.card(iid)
      if c and c.attached and #c.attached > 0 then
        table.insert(victims, iid)
      end
    end
    if #victims == 0 then return end
    local victim = game.choose_card(victims, {prompt = "foreclose on which creature?"})
    if not victim then return end
    local vcard = game.card(victim)
    if not vcard or not vcard.attached or #vcard.attached == 0 then return end
    -- Snapshot before mutating (game.move_to pops from host's attached list).
    local pool = {}
    for _, aid in ipairs(vcard.attached) do
      table.insert(pool, aid)
    end
    local stolen = game.choose_card(pool, {prompt = "seize which attached card?"})
    if not stolen then return end
    game.move_to(stolen, self.owner, "hand")
  end,
}
