-- Name not yet specified.
return {
  id = "zebra",
  symbol = "⨳",
  colors = {"black", "white"},
  type = "creature",
  subtypes = {"Zebra"},
  cost = {
    {amount = 2, source = "hand"},
    {amount = 2, source = "mill"},
  },
  abilities = {
    "If this card gets attached as a cost to a black or white card, you may reveal it and draw a card.",
  },
  stats = {x = 2, y = 2},
}
