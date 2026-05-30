-- Green goblin of the cycle: 1/1, 1 hand + 2 mill, lord + on-block discard.
-- Lord wired via STATIC.md Phase 1. on-block discard still deferred (needs
-- discard-choice integration).
--
-- Reading the literal text "all other goblins" (no controller qualifier),
-- this is a GLOBAL anthem — also boosts opponent's goblins. That's
-- intentional per the literal text and creates a "goblin mirror" dynamic
-- where dropping a warlord buffs both sides equally. If unintended, change
-- controller = "owner" to scope to the caster's side only.
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
  static = {
    affects = {
      subtypes = {"goblin"},
      exclude_self = true,
    },
    modifier = {x = 1, y = 1},
  },
}
