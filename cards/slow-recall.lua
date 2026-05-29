-- Purple spell. Deferred across multiple infra gaps:
--
--   1. Spell type isn't routable by play_card yet (only Creature + Instant).
--      The card loads but can't be played until SPELL type lands.
--   2. The "6 deck (exiled)" cost is encoded here as MILL (which sends to
--      GRAVEYARD per P.11) until a proper deck→exile cost source exists.
--      When that lands, switch the source to whatever it's named.
--   3. The persistent "each turn, return one" effect needs `on_turn_end`
--      events + a delayed-trigger registry. None of that exists. Handler
--      omitted; abilities text describes the intended effect.
return {
  id = "slow-recall",
  name = "Slow Recall",
  colors = {"purple"},
  type = "spell",
  cost = {
    {amount = 2, source = "hand"},
    {amount = 6, source = "mill"},
  },
  abilities = {
    "exile the top 6 cards of your deck.",
    "each turn, one card exiled by this card returns to its owner's hand.",
  },
}
