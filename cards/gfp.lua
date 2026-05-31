-- Green mutation: +1/+1 anthem on a single host. Cheap cantrip-style
-- buff. Real flavor: Green Fluorescent Protein from Aequorea victoria
-- jellyfish — the gene that became biology's universal "did the
-- transfection work" tag because expressing cells literally glow.
return {
  id = "gfp",
  name = "GFP",
  type = "mutation",
  colors = {"green", "glow"},
  cost = {{amount = 1, source = "mill"}},
  abilities = {
    "the host creature gets +1/+1 and becomes green and glow.",
  },
  flavor = "Your creature glows in the dark, whoohoo!",
  static = {
    affects = {
      scope = "attached_host",
    },
    modifier = {x = 1, y = 1, colors = {"green", "glow"}},
  },
}
