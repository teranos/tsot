-- Orange mutation in the protein-name cycle (Klotho/FST/GFP/mCherry).
-- MYC is the canonical proto-oncogene — when dysregulated it drives
-- uncontrolled proliferation. Mechanical hook:
--
--   - Static: host gets +0/+X where X is the number of cards attached
--     to the host (every other mutation + every card MYC has pulled
--     in from the deck contributes).
--   - At the beginning of MYC's controller's turn: the top two cards
--     of the controller's DECK become attached to the host (mill into
--     the attached zone, face-down per P.17). Compounds with the
--     static — each turn the host's toughness grows by 2 from the
--     newly-attached deck cards.
--
-- Free to cast — the payoff is in the proliferation loop.
return {
  id = "myc",
  name = "MYC",
  type = "mutation",
  colors = {"orange"},
  cost = {},
  abilities = {
    "the host creature gets +0/+1 for each card attached to it.",
    "at the beginning of your turn, attach the top two cards from your deck to the host.",
  },
  flavor = "Dial broken at full gain.",
  static = {
    affects = {scope = "attached_host"},
    modifier = {x = 0, y = "attached"},
  },
  on_turn_begin = function(game, self)
    -- Only fire when it's the mutation's controller's turn. The engine
    -- broadcasts OnTurnBegin to every BOARD card of the active player
    -- plus its attached — that already gates to the right side.
    local host = game.host_of(self.instance_id)
    if host then
      game.attach_from_deck(host, self.controller, 2)
    end
  end,
}
