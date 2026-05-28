-- Debug/test card. Tests core abilities at 0 cost.
-- Color and symbol not yet specified.
return {
  id = "dtst-creature",
  name = "DTST_creature",
  type = "creature",
  subtypes = {"Creature Test"},
  abilities = {
    "Tap: kill target creature",
    "Tap: draw a card",
    "Tap: draw 10 cards",
    "Tap: return target to hand",
    "1 graveyard: change target card's color",
    "1 graveyard: change target card's symbol",
    "Tap: counter target card on the stack",
  },
  stats = {x = 10, y = 10},
}
