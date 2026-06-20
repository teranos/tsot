-- Reincubator — green+black artifact. Two of three abilities wired.
--
-- Executable:
--   (1) Cost-reduction static — green or black creatures cost 1 hand + 1
--       graveyard less to cast (everywhere, both players). Wired via the
--       Phase 3.5 cost-modification layer that Modern LCD Clock pioneered:
--       `static.cost_modifiers` carries two entries (-1 hand, -1 graveyard),
--       and `affects.colors = {"green", "black"}` gates on "candidate has
--       at least one of these colors." `kind = creature` further narrows
--       to creature casts. The engine's play_card pre-pass calls
--       `cost_reduction(iid, source)` and `effective_combined_cost(iid)`
--       (A.12) for any handler read. P.20 clamps each per-source amount
--       to 0 so over-reduction doesn't bleed credit across sources.
--   (2) ETB tutor (on_enter_board). Reads game.payment_ids().sacrifice[1]
--       — the creature paid for the 1s cost component. Threshold =
--       sacrificed.x + sacrificed.y + 2, read live via A.11's effective-
--       stats path. Pool = every creature in the caster's deck whose
--       effective combined p/t ≤ threshold. game.choose_card picks one,
--       game.move_to drops it on the caster's board. ETB effects apply.
--
-- Printed-only (no handler):
--   (3) Activated — "T, exile this, sacrifice a creature: search your
--       deck for a creature whose combined p/t is up to 2 higher than the
--       sacrificed creature's and put it on the board." Needs SACRIFICE +
--       SELF cost components in activated abilities, both deferred per
--       LIMITATIONS.md ## activated abilities.
return {
  id = "reincubator",
  name = "Reincubator",
  type = "artifact",
  colors = {"green", "black"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 1, source = "sacrifice", kind = "creature"},
    {amount = 2, source = "graveyard"},
  },
  abilities = {
    "static: any creature whose colors include green OR black costs 1 hand and 1 graveyard less to cast (everywhere, both players). examples: mono-green qualifies, mono-black qualifies, green/black qualifies, black/white qualifies, blue/red does NOT. the bonus does not stack — a creature with multiple qualifying colors still gets it once.",
    "when this enters the board: you may search your deck for a creature whose combined power+toughness is up to 2 higher than the sacrificed creature's, and put it on the board. (ETB effects apply.)",
    "T, exile this, sacrifice a creature: you may search your deck for a creature whose combined power+toughness is up to 2 higher than the sacrificed creature's, and put it on the board. (ETB effects apply.)",
  },
  static = {
    affects = {
      kind = "creature",
      colors = {"green", "black"},
    },
    cost_modifiers = {
      {source = "hand", amount = 1},
      {source = "graveyard", amount = 1},
    },
  },
  on_enter_board = function(game, self)
    -- Read the sacrificed creature from the cast's payment context.
    -- 1s in the cost → exactly one iid in payment_ids.sacrifice.
    local pay = game.payment_ids()
    local sac_iids = pay.sacrifice
    if sac_iids == nil or #sac_iids == 0 then return end
    local sac_view = game.card(sac_iids[1])
    if sac_view == nil then return end
    -- Effective combined p/t per A.11 — game.card returns the post-
    -- modifier x/y, so a buffed-then-sacrificed creature uses its
    -- buffed combined value as the threshold base.
    local threshold = (sac_view.x or 0) + (sac_view.y or 0) + 2

    -- Build candidate pool from the caster's deck. Only creatures whose
    -- effective combined p/t is at or below the threshold qualify.
    local pool = {}
    for _, iid in ipairs(game.zones(self.owner).deck) do
      local c = game.card(iid)
      if c ~= nil and c.type == "creature" then
        local pt = (c.x or 0) + (c.y or 0)
        if pt <= threshold then
          table.insert(pool, iid)
        end
      end
    end
    if #pool == 0 then return end

    local picked = game.choose_card(pool, {
      optional = true,
      prompt = "reincubate: combined p+t ≤ " .. threshold,
    })
    if picked == nil then return end
    game.move_to(picked, self.owner, "board")
  end,
}
