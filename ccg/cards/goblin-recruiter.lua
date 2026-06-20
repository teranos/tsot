-- Purple goblin: 2 hand cost, 2/3, on-play reveal-top-6-and-grab-goblins.
-- Functions as a hand-refill engine for goblin-heavy decks. The reveal step
-- doesn't mutate state (no engine "revealed" zone); the reward is the
-- subset moved to hand.
return {
  id = "goblin-recruiter",
  name = "Goblin Recruiter",
  colors = {"purple"},
  type = "creature",
  subtypes = {"goblin"},
  cost = {
    {amount = 2, source = "hand"},
  },
  abilities = {
    "when you play this card, reveal the top 6 cards of your deck and put all goblin cards among them into your hand.",
  },
  stats = {x = 2, y = 2},
  on_play = function(game, self)
    local deck = game.zones(self.owner).deck
    local n = math.min(6, #deck)
    for i = 1, n do
      local iid = deck[i]
      local c = game.card(iid)
      if c and c.subtypes then
        for _, s in ipairs(c.subtypes) do
          if s == "goblin" then
            game.move_to(iid, self.owner, "hand")
            break
          end
        end
      end
    end
  end,
}
