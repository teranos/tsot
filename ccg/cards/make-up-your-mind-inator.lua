-- Colorless artifact: a tap-activated retarget effect on cards already on
-- the stack. Currently flavor-text only — no handler — because the card
-- depends on three engine pieces that don't exist yet:
--
--   1. Activated abilities (`T: ...`). LIMITATIONS notes this as not
--      started: no Lua declaration syntax, no activation flow that puts
--      the ability on the stack, no sim AI decision hook.
--
--   2. Targeting layer. tsot has no engine concept of "what is legal to
--      target" — every targeting card today builds its own pool. To pick
--      a NEW target for an existing stack spell, the engine needs to know
--      what the original spell's target pool was, recompute it, and let
--      the activator choose from it.
--
--   3. Stack-item target mutation. StackItem::PlayedCard captures the
--      cast but resolution-time targeting lives inside the handler's
--      Lua call. The engine has no mechanism to alter a spell's target
--      between cast and resolution.
--
-- Until all three land, this card sits in deck pools as pitch fuel and
-- a cheap inert board drop. Cost 1 hand. Symbol not yet specified.
return {
  id = "make-up-your-mind-inator",
  name = "Make Up Your Mind -inator",
  colors = {},
  type = "artifact",
  cost = {{amount = 1, source = "hand"}},
  abilities = {
    "T: change the target of target spell.",
  },
}
