-- P53 — purple spell. The "guardian of the genome": when cellular stress
-- is detected, P53 selects the response. Mechanic: tutor for one of the
-- three P53-mediated stress-response mutations — APOPTOSIS (programmed
-- death), SENESCENCE (permanent growth arrest), or HYPOXIA (oxygen-stress
-- response). Forward-looking — only APOPTOSIS exists in the corpus
-- today; SENESCENCE and HYPOXIA are designed but not yet authored.
-- The pool resolver scans the deck for any of the three ids, so the
-- card works correctly with whichever subset exists at a given moment.
--
-- on_play handler is wired today — the choose_card replay path
-- (lua_api.rs / play.rs PlayError::ChoicePending → StepEngine HumanPrompt)
-- already handles on_play yields. No engine dependency.
return {
  id = "P53",
  name = "P53",
  type = "spell",
  colors = {"purple"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "mill"},
  },
  abilities = {
    "search your deck for APOPTOSIS, SENESCENCE, or HYPOXIA and put it in your hand.",
  },
  flavor = "Guardian of the genome. Reads the damage, picks the response.",
  on_play = function(game, self)
    local targets = {"APOPTOSIS", "SENESCENCE", "HYPOXIA"}
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c then
        for _, target_id in ipairs(targets) do
          if c.id == target_id then
            table.insert(pool, iid)
            break
          end
        end
      end
    end
    if #pool == 0 then return end
    local picked = game.choose_card(pool, {
      optional = true,
      prompt = "P53: select stress response",
    })
    if picked == nil then return end
    game.move(picked, "hand")
  end,
}
