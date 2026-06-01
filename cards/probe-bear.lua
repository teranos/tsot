-- Reference example for the `variants` schema used by `tsot balance-probe`.
-- The base card and each variant get loaded as separate cards:
--   probe-bear           (the base — same as if there were no `variants`)
--   probe-bear-defensive (variant — overrides only the `stats` field)
--   probe-bear-aggressive
--   probe-bear-balanced
-- Variants are marked `is_variant = true` in the engine and excluded
-- from `make evolve` / champions / gauntlets. Only `make probe` picks
-- them up. Override semantics: each top-level field declared in a
-- variant entry REPLACES the base wholesale (no deep merge). To tweak
-- one ability, copy the full `activated` array into the variant with
-- the tweak.
return {
  id = "probe-bear",
  name = "Probe Bear",
  symbol = "꩜",
  colors = {"green"},
  type = "creature",
  subtypes = {"bear"},
  cost = {{amount = 2, source = "hand"}},
  stats = {x = 3, y = 3},
  abilities = {},
  variants = {
    ["defensive"] = {
      name = "Probe Bear (Defensive 2/4)",
      stats = {x = 2, y = 4},
    },
    ["balanced"] = {
      name = "Probe Bear (Balanced 3/3)",
      stats = {x = 3, y = 3},
    },
    ["aggressive"] = {
      name = "Probe Bear (Aggressive 4/2)",
      stats = {x = 4, y = 2},
    },
  },
}
