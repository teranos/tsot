-- Black artifact: graveyard-powered anthem. While your graveyard has 5
-- or more cards, creatures you control get +1/+1 and gain flying.
--
-- The card exists in the corpus as the motivator for STATIC.md Phase 2.
-- The handler is intentionally absent — Ossuary needs three Phase 2
-- capabilities the engine doesn't have yet:
--
--   1. State-reading predicate. The "graveyard has 5 or more" condition
--      depends on game state, not card data. Phase 1's declarative
--      `affects` struct (subtypes / colors / controller / exclude_self)
--      can't express it. Phase 2 needs a `condition` field on the
--      static that the engine evaluates against state — at minimum, an
--      enum of common predicates ("graveyard_count >= N",
--      "board_count >= N", "source_tapped", etc.) and probably an
--      escape hatch for arbitrary lookups.
--
--   2. Keyword-grant modifier. Phase 1 modifier is `{x, y}` stat boost
--      only. Phase 2 extends to `{keyword = "flying"}` or similar, and
--      `has_keyword` consults on-board static sources the same way
--      `effective_stats` already does.
--
--   3. Combined stat-and-keyword on one static. Ossuary's effect is
--      both +1/+1 AND flying. Either the modifier shape grows to a
--      list (each entry one stat OR one keyword) OR a single modifier
--      carries multiple components. Design call.
--
-- Why Ossuary in particular as the test card: self-referential thematics
-- (mill-damage economy fills the graveyard, which powers up your board),
-- lazy-eval friendly (predicate re-checks on every `effective_stats` /
-- `has_keyword` call, no invalidation logic), and an all-or-nothing flip
-- at threshold makes telemetry obvious.
--
-- Cost 2 hand + 2 mill: a real investment that ironically adds 2 cards
-- to the graveyard you're stocking. Symbol not yet specified.
--
-- Until Phase 2 lands: Ossuary sits in A/B/H pools as dead weight (the
-- artifact type isn't `play_card`-routable, so it never lands on board
-- and can only be pitched). Its presence will lower A/B/H slightly
-- until Phase 2 turns it from drag to engine.
return {
  id = "ossuary",
  name = "Ossuary",
  colors = {"black"},
  type = "artifact",
  subtypes = {"relic"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "you cannot cast this card.",
    "while your graveyard has 5 or more cards, creatures you control get +1/+1 and have flying.",
  },
}
