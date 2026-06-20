-- Purple sorcery: identical to Resurrect except the reanimated creature
-- can attack the turn it comes back. Same shape, same cost — the only
-- mechanical difference is one game.set_summoning_sick(target, false)
-- call after move_to clears the sickness the engine automatically
-- applied per B.3.
--
-- Color split is intentional: Resurrect (black) is the classic
-- necromancer's slow ritual; Wake Dead (purple) is the chaos /
-- unstable-magic flavor — purple's identity in the corpus skews toward
-- "you get something for cheap but it's risky." Splitting the two
-- cards across colors also means decks have to choose color identity
-- to access either, rather than slotting both into one black shell.
--
-- Designed as the deliberate strength comparison to Resurrect: both
-- are 1H + 1M + self-exile, both pull a creature from any graveyard to
-- your board with full ETB. Resurrect's reanimated creature blocks-
-- only this turn; Wake Dead's hits the ground swinging. Sim/EA over
-- time will price the haste delta — a strong evidence-driven way to
-- learn what "+haste on a reanimate spell" is actually worth in this
-- corpus.
return {
  id = "wake-dead",
  name = "Wake Dead",
  colors = {"purple"},
  type = "spell",
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
    {amount = 1, source = "self"},
  },
  abilities = {
    "Choose a creature card in any graveyard. Put it onto the battlefield under your control with haste. ETB effects apply.",
  },
  on_play = function(game, self)
    local pool = {}
    for _, side in ipairs({self.owner, game.opponent(self.owner)}) do
      for _, iid in ipairs(game.zones(side).graveyard) do
        local c = game.card(iid)
        if c and c.type == "creature" then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end
    local target = game.choose_card(pool, {prompt = "wake which creature?"})
    if not target then return end
    game.move_to(target, self.owner, "board")
    -- Clear the summoning sickness move_to applied — this is the entire
    -- mechanical difference from Resurrect.
    game.set_summoning_sick(target, false)
  end,
}
