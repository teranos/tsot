-- Reach elemental that rearranges what's attached to whom. Works from
-- the board as a turn-begin trigger and from the graveyard as a one-shot
-- activated ability.

-- Pick any creature on either side, tap it, then move one of its
-- attached cards onto a different creature.
local function rearrange(game, self)
  local me = self.controller
  local opp = game.opponent(me)
  local creatures = {}
  for _, iid in ipairs(game.zones(me).board) do
    local c = game.card(iid)
    if c and c.type == "creature" then table.insert(creatures, iid) end
  end
  for _, iid in ipairs(game.zones(opp).board) do
    local c = game.card(iid)
    if c and c.type == "creature" then table.insert(creatures, iid) end
  end
  if #creatures == 0 then return end
  local target = game.choose_card(creatures, {prompt = "tap and rearrange"})
  if target == nil then return end
  game.tap(target)
  local attached = {}
  local target_view = game.card(target)
  if target_view and target_view.attached then
    for _, aid in ipairs(target_view.attached) do
      table.insert(attached, aid)
    end
  end
  if #attached == 0 then return end
  local picked = game.choose_card(attached, {prompt = "attached card to move"})
  if picked == nil then return end
  local hosts = {}
  for _, iid in ipairs(creatures) do
    if iid ~= target then table.insert(hosts, iid) end
  end
  if #hosts == 0 then return end
  local new_host = game.choose_card(hosts, {prompt = "new host"})
  if new_host == nil then return end
  game.move_attached(target, new_host, picked)
end

return {
  id = "durian-elemental",
  name = "Durian Elemental",
  type = "creature",
  colors = {"green", "cyan"},
  subtypes = {"elemental"},
  cost = {
    {amount = 1, source = "hand"},
    {amount = 4, source = "graveyard"},
  },
  stats = {x = 3, y = 4},
  abilities = {
    "reach.",
    "at the beginning of your turn, tap target creature and move one of its attached cards to another creature.",
    "while this card is in your graveyard, 1H + exile this card from your graveyard: tap target creature and move one of its attached cards to another creature.",
  },
  on_turn_begin = function(game, self)
    return rearrange(game, self)
  end,
  activated = {
    {
      cost = {
        {source = "hand", amount = 1},
        {source = "self", amount = 1},
      },
      text = "while in your graveyard, 1H + exile this: rearrange an attached card.",
      timing = "instant",
      from_zones = {"graveyard"},
      effect = function(game, self)
        return rearrange(game, self)
      end,
    },
  },
}
