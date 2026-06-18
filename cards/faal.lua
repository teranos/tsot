-- Faal — chaos-engineering probe.
--
-- A deliberately-buggy creature whose activated abilities each
-- trigger a different sacred-error pipeline branch. Used to verify
-- ERROR.md items end-to-end in the dev tool: tap an ability, watch
-- for the typed-Error overlay with the expected shape.
--
-- Failure modes covered:
--   T (ability 1): Lua nil-index             → mlua chain walker (RuntimeError)
--   T (ability 2): explicit error() call     → mlua chain walker (RuntimeError)
--   T (ability 3): assert(false, msg)        → mlua chain walker (RuntimeError)
--   T (ability 4): call a nil function       → mlua chain walker (RuntimeError)
--   T (ability 5): game.move bogus iid       → move_card_or_emit (engine surface)
--   T (ability 6): deep recursion            → mlua chain walker (Lua stack overflow)
--
-- Pass criteria:
--   1. Each tap surfaces a typed Error overlay AT the cursor.
--   2. The `why` field contains the inner Lua line:message — not
--      just "callback error: ..." (chain walker regression check).
--   3. The game continues — no wasm trap, no frozen UI.
--
-- THIS IS A DEVELOPER PROBE. Don't ship in production decks. The
-- `id = "faal"` is the recognised handle; a future EA-pool filter
-- can exclude `subtypes` containing "debug".

return {
    id = "faal",
    name = "Faal",
    -- The engine accepts a single CardType; the artifact dimension
    -- rides as a subtype so the card is BOTH (creature for combat
    -- semantics + targeting; artifact for printed-type display).
    type = "creature",
    subtypes = {"artifact", "debug"},

    -- Slot-form colors: L and R are omitted so the renderer leaves
    -- those two positions visually empty (= transparent). The
    -- loader enforces unique colors per slot, so omission is the
    -- correct semantic for "this slot has no color." Every other
    -- slot carries a DISTINCT color (13 slots, 13 distinct colors
    -- including azure-list duplicates merged to single occurrences).
    --
    -- Picking distinct colors per slot is a hard constraint (loader
    -- rejects duplicates); 13 named colors exhaust the corpus's
    -- known palette so the card prints as "many colors" — chaos
    -- flag for the dev tool's color-coded selectors.
    colors = {
        TL = "red",     T  = "blue",     TR = "green",
        UL = "black",   U  = "orange",   UR = "pink",
                        C  = "purple",
        DL = "brown",   D  = "azure",    DR = "white",
        BL = "yellow",  B  = "magenta",  BR = "cyan",
    },

    -- All five Teranos symbols. The card displays as a symbol-mash:
    --   ax = ⋈, ix = ⨳, am = ≡, pulse = ꩜, sem = ⊨
    symbols = {"⋈", "⨳", "≡", "꩜", "⊨"},

    cost = {{source = "hand", amount = 1}},
    stats = {x = 5, y = 5},

    -- Each activated ability is one chaos probe.
    activated = {
        {
            cost = "tap",
            text = "T: nil-index — chain walker probe A",
            timing = "sorcery",
            effect = function(game, self)
                -- mlua::Error::CallbackError → RuntimeError
                -- ("attempt to index a nil value").
                local x = nil
                return x.boom
            end,
        },

        {
            cost = "tap",
            text = "T: explicit error() — chain walker probe B",
            timing = "sorcery",
            effect = function(game, self)
                error("faal: explicit error() from Lua")
            end,
        },

        {
            cost = "tap",
            text = "T: assert(false) — chain walker probe C",
            timing = "sorcery",
            effect = function(game, self)
                assert(false, "faal: assert(false) with message")
            end,
        },

        {
            cost = "tap",
            text = "T: call nil function — chain walker probe D",
            timing = "sorcery",
            effect = function(game, self)
                -- mlua::Error::RuntimeError ("attempt to call a nil value").
                local f = nil
                f()
            end,
        },

        {
            cost = "tap",
            text = "T: zone corruption — move bogus iid",
            timing = "sorcery",
            effect = function(game, self)
                -- Goes through move_card_or_emit on the Rust side;
                -- the helper pushes a typed Error with
                -- surface="engine", region="lua-game-move" (or
                -- wherever the FFI binding routes it).
                game.move("faal-not-a-real-iid", "graveyard")
            end,
        },

        {
            cost = "tap",
            text = "T: stack overflow via recursion",
            timing = "sorcery",
            effect = function(game, self)
                local function rec(n) return rec(n + 1) end
                rec(0)
            end,
        },
    },
}
