-- Black artifact: graveyard-powered anthem. While your graveyard has 5
-- or more cards, creatures you control get +1/+1 and gain flying.
--
-- Phase 2 motivator. Keyword-grant + combined-stat-and-keyword landed;
-- the remaining gap is the state-reading predicate ("graveyard_count >= 5")
-- — once that's wired this gets a `static` block. Until then, ossuary is
-- a 4-cost (2 hand + 2 mill) artifact that does nothing on BOARD.
--
-- Cost 2 hand + 2 mill: an investment that ironically adds 2 cards to
-- the graveyard you're stocking. Symbol not yet specified.
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
    "while your graveyard has 5 or more cards, creatures you control get +1/+1 and have flying.",
  },
}
