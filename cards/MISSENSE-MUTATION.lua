-- Cyan mutation in the protein-name cycle. A missense mutation swaps
-- one amino acid for another: the protein is still functional, just
-- shifted — its surface chemistry is now off-spec and it interacts
-- differently with everything reflective nearby. Mechanical hook:
--   - Host gets -0/-0.5 base.
--   - Host becomes cyan (granted color).
--   - Host gets +1/-0.25 for each shiny card on the board (both sides).
return {
  id = "missense-mutation",
  name = "Missense Mutation",
  type = "mutation",
  colors = {"cyan"},
  cost = {
    {amount = 1, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "the host creature gets -0/-0.5.",
    "the host creature becomes cyan in addition to its other colors.",
    "the host creature gets +1/-0.25 for each shiny card on the board.",
  },
  flavor = "One letter swapped. The whole fold rotates.",
  static = {
    affects = {scope = "attached_host"},
    modifier = {
      x = { 0, {scale = 1, count = "board:face:shiny"} },
      y = { -0.5, {scale = -0.25, count = "board:face:shiny"} },
      colors = {"cyan"},
    },
  },
}
