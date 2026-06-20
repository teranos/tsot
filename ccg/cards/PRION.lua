-- Black mutation in the protein-name cycle. A prion is a misfolded
-- protein that propagates by converting normal proteins into copies
-- of itself — the biological zombie. Same mechanical hook: turn the
-- host into a zombie, and when it dies, find another zombie to keep
-- the fold spreading.
return {
  id = "PRION",
  name = "PRION",
  type = "mutation",
  colors = {"black"},
  subtypes = {"zombie"},
  cost = {
    {amount = 2, source = "graveyard"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "the host creature gets -0/-2 and becomes a zombie in addition to its other types.",
    "when the host dies, you may search your deck for a card with subtype zombie and put it in your hand.",
  },
  flavor = "A fold that copies itself into every protein it meets.",
  static = {
    affects = {scope = "attached_host"},
    modifier = {x = 0, y = -2, subtypes = {"zombie"}},
  },
  on_die = function(game, self)
    -- Fires on the mutation when its host dies (host → GRAVEYARD per
    -- P.4, attached → EXILE per P.8). Tutor a zombie from this
    -- controller's deck to hand. Optional via choose_card.
    local zombies = {}
    for _, iid in ipairs(game.zones(self.controller).deck) do
      local c = game.card(iid)
      if c and c.subtypes then
        for _, st in ipairs(c.subtypes) do
          if st == "zombie" then
            table.insert(zombies, iid)
            break
          end
        end
      end
    end
    if #zombies == 0 then return end
    local picked = game.choose_card(zombies, {
      prompt = "Search your deck for a zombie",
      optional = true,
    })
    if picked then
      game.move(picked, "hand")
    end
  end,
}
