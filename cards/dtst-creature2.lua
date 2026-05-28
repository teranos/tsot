-- Debug/test card. Tests more abilities at various costs.
-- Color and symbol not yet specified.
return {
  id = "dtst-creature2",
  name = "DTST-creature2",
  type = "creature",
  subtypes = {"Creature Test"},
  abilities = {
    "1 mill: change a card's type",
    "1 mill, 1 graveyard: put a card from exile into your hand",
    "X hand: draw X cards",
    "Tap: untap target creature",
  },
  stats = {x = 6, y = 6},
}
