-- Blue Ghost — 1/1 colored ghost. Two printed activated abilities,
-- both currently non-executable: SELF cost in activated abilities and
-- activations from non-BOARD zones (ATTACHED, GRAVEYARD) are deferred
-- per LIMITATIONS.md "Deferred:". The ability text lives in the corpus
-- as design intent until the engine catches up.
return {
  id = "blue-ghost",
  name = "Blue Ghost",
  colors = {"blue"},
  symbol = "≡",
  type = "creature",
  subtypes = {"ghost"},
  holes = {"UL", "TL", "B", "BR"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 5, source = "mill"},
  },
  stats = {x = 1, y = 1},
  abilities = {
    "while attached, you may exile this card: search your deck for a blue symbol card and put it on the board.",
    "while this card is in your graveyard, you may exile this card: search your deck for a blue symbol card and put it in your hand.",
  },
}
