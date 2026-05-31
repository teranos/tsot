-- Green spider that scales with mutations attached to it. Same dynamic-
-- stat shape as hydra, but filtered to mutation-type attachments only
-- via STATIC Phase 1.5 ModifierValue::AttachedCountByKind(Mutation). If
-- a mutation is removed (host moves to graveyard, mutation exiled, etc.)
-- the spider's stats shrink immediately on the next effective_stats().
return {
  id = "big-freaking-spider",
  name = "Big Freaking Spider",
  type = "creature",
  colors = {"green"},
  subtypes = {"spider"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "reach.",
    "this creature gets +1/+1 for each mutation attached to it.",
  },
  flavor = "What!?",
  stats = {x = 2, y = 4},
  static = {
    affects = {
      scope = "source_only",
    },
    modifier = {x = "attached:type:mutation", y = "attached:type:mutation"},
  },
}
