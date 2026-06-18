-- Purple mutation in the protein-name cycle. A nonsense mutation
-- introduces a premature stop codon: the protein gets cut short and
-- arrives at the cell denuded — no signaling, no binding, no function.
-- Mechanically: the host loses its color identity and all of its
-- abilities (printed AND granted) while this mutation is attached,
-- with a small bump to attack at a cost to toughness.
return {
  id = "nonsense-mutation",
  name = "Nonsense Mutation",
  type = "mutation",
  colors = {"purple"},
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "the host creature gets +1/-1.",
    "the host creature loses all colors.",
    "the host creature loses all abilities (printed and granted).",
  },
  flavor = "A premature stop. The protein arrives short.",
  static = {
    affects = {scope = "attached_host"},
    modifier = {
      x = 1,
      y = -1,
      colorless = true,
      suppresses_abilities = true,
    },
  },
}
