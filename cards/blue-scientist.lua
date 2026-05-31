-- Blue 1/1 human scientist. 1 hand + 1 graveyard. ETB trigger lets the
-- scientist reclaim a mutation card on arrival — from a dual source:
-- mutations in your own graveyard (recovery), OR mutations currently
-- attached to opposing creatures (theft + investment-strip in one
-- action). The taken card lands in your hand for redeployment.
--
-- The opp-side path is the spicier one: stealing an attached mutation
-- pulls it off the host, which means the host instantly loses whatever
-- buff that mutation was granting via static (gfp's +1/+1 to host
-- evaporates per Phase 1.5 effective_stats recompute, same dynamic as
-- falter/foreclosure on hydra). You don't just get a card — you also
-- shrink one of their creatures.
--
-- The graveyard-recovery path is narrow today (mutations don't
-- naturally hit graveyard since the usual path is HAND → attached-on-
-- host → exile-on-host-death per P.8), but lives for the cases where a
-- mutation got milled, discarded by pack-rat's recursion, or sent to
-- graveyard by some future effect.
--
-- Uniformly uses game.move_to(target, self.owner, "hand"): handles both
-- "own graveyard → own hand" (same-player) and "opp's attached → my
-- hand" (cross-player + strip-from-host) through the same primitive.
-- ETB doesn't re-fire on a move to "hand" — only on non-board → board
-- transitions per the engine wiring.
--
-- Replaced the previous unwired "draw 2 instead of 1" ability — that
-- depended on STATIC Phase 4 replacement effects which don't exist yet.
-- This shape uses primitives already in the engine.
--
-- Symbol not yet specified.
return {
  id = "blue-scientist",
  name = "Blue Scientist",
  colors = {"blue"},
  type = "creature",
  subtypes = {"human", "scientist"},
  cost = {{amount = 1, source = "hand"}, {amount = 1, source = "graveyard"}},
  abilities = {
    "when this creature enters the board, you may take a mutation card from your graveyard, or one attached to a creature an opponent controls, and put it into your hand.",
  },
  flavor = "First subject: nearest available.",
  stats = {x = 1, y = 1},
  on_enter_board = function(game, self)
    local owner = self.owner
    local opp = game.opponent(owner)

    -- Pool A: mutations in your graveyard.
    local pool = {}
    for _, iid in ipairs(game.zones(owner).graveyard) do
      local c = game.card(iid)
      if c and c.type == "mutation" then
        table.insert(pool, iid)
      end
    end

    -- Pool B: mutations attached to opp's creatures. Face-down per P.17
    -- but the engine's game.card view exposes card data regardless.
    for _, host_iid in ipairs(game.zones(opp).board) do
      local host = game.card(host_iid)
      if host and host.attached then
        for _, att_iid in ipairs(host.attached) do
          local a = game.card(att_iid)
          if a and a.type == "mutation" then
            table.insert(pool, att_iid)
          end
        end
      end
    end

    if #pool == 0 then return end
    if not game.confirm("take a mutation?") then return end
    local target = game.choose_card(pool, {prompt = "take which mutation?"})
    if not target then return end

    -- Single primitive handles both origin cases uniformly.
    game.move_to(target, owner, "hand")
  end,
}
