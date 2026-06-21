-- Reach elemental that rearranges what's attached to whom. Works from
-- the board as a turn-begin trigger and from the graveyard as a one-shot
-- activated ability.
return {
  id = "durian-elemental",
  name = "Durian Elemental",
  type = "creature",
  colors = {"green", "cyan"},
  subtypes = {"elemental"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 4, source = "graveyard"},
  },
  stats = {x = 3, y = 4},
  abilities = {
    "reach.",
    "at the beginning of your turn, tap target creature and move one of its attached cards to another creature.",
    "while this card is in your graveyard, 1H + exile this card from your graveyard: tap target creature and move one of its attached cards to another creature.",
  },
}
