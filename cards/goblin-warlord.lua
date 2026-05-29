-- Green goblin of the cycle: 1/1, 1 hand + 2 mill, lord + on-block discard.
-- Both abilities deferred:
--   - The +1/+1 lord effect needs the static system (LUA Phase 2).
--   - "discard a card" on block needs the choice API (LUA Phase 2).
return {
  id = "goblin-warlord",
  name = "Goblin Warlord",
  colors = {"green"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "all other goblins get +1/+1.",
    "whenever this creature blocks, discard a card.",
  },
  stats = {x = 1, y = 1},
}
