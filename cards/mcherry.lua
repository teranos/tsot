-- Pink mutation: +1/+1 to host. The pink counterpart to GFP. mCherry
-- (monomeric Cherry) is the workhorse red/pink fluorescent protein in
-- molecular biology — derived from DsRed in Discosoma, engineered for
-- monomer stability and brightness. Same mechanical role as GFP, color
-- differs.
return {
  id = "mcherry",
  name = "mCherry",
  type = "mutation",
  colors = {"pink", "glow"},
  cost = {{amount = 1, source = "mill"}},
  abilities = {
    "the host creature gets +1/+1 and becomes pink and glow.",
  },
  flavor = "Same as GFP, just pink.",
  static = {
    affects = {
      scope = "attached_host",
    },
    modifier = {x = 1, y = 1, colors = {"pink", "glow"}},
  },
}
