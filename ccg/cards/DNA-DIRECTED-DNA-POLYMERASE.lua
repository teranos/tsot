-- DNA-directed DNA polymerase — δ mutation. On the host's OWNER's
-- turn-begin, that owner draws cards equal to the host's effective
-- toughness. Reads the host live via game.host_of, then takes host.owner
-- (not host.controller). The split matters once host-stealing exists: if
-- a player steals a creature carrying a polymerase, draws still go to
-- the ORIGINAL caster of the host, not the new controller.
--
-- Toughness read uses the engine's effective stat (y after modifiers).
-- Per the corpus stat-modifier rule, P/T modifications resolve BEFORE
-- other effects, so by the time on_turn_begin fires the host's y
-- already reflects any active buffs/debuffs.
--
-- OnTurnBegin fires for cards on the active player's BOARD plus their
-- attached cards (src/game/turn.rs), so the trigger naturally gates to
-- "a turn when the host is on someone's board." Cast on your own
-- creature → fires on your next turn (you draw). Cast on an opponent's
-- creature → fires on their next on_turn_begin and THEY draw, because
-- the host's owner is them.
return {
  id = "DNA-DIRECTED-DNA-POLYMERASE",
  name = "DNA-DIRECTED DNA POLYMERASE",
  type = "mutation",
  colors = {"blue", "green"},
  symbol = "δ",
  cost = {},
  abilities = {
    "the host creature gets: at the beginning of your turn, draw cards equal to this creature's toughness.",
  },
  on_turn_begin = function(game, self)
    local host = game.host_of(self.instance_id)
    if host == nil then return end
    local host_view = game.card(host)
    if host_view == nil then return end
    local n = math.floor(host_view.y or 0)
    if n > 0 then
      game.draw(host_view.owner, n)
    end
  end,
}
