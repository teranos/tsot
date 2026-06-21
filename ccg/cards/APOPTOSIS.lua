-- Programmed cell death. Strips one attached component per turn; when
-- nothing is left attached, the host dies. `same_sleeve` keeps this
-- mutation off the strippable list.
return {
  id = "APOPTOSIS",
  name = "APOPTOSIS",
  type = "mutation",
  colors = {"yellow", "purple"},
  same_sleeve = true,
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "the host creature gets: at the beginning of your turn, move one of this creature's attached cards to your graveyard. if no cards are attached to this creature anymore, sacrifice it.",
  },
  flavor = "P53 calls it. The cell agrees, then tidies itself out one organelle at a time.",
}
