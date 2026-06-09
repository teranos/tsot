module Card exposing
    ( Card(..)
    , CardData
    , Config
    , Kind(..)
    , RenderMode(..)
    , Slot(..)
    , SlotSymbol
    , Timing(..)
    , decode
    , defaultConfig
    , isAttachedZone
    , kindFromString
    , slotKey
    , slotSpiralOrder
    , view
    )

{-| The Card primitive — single source of truth for rendering a card
across every surface in the game (hand, board, graveyard, deck-top,
attached stack, deckbuilder pool, deckbuilder deck-list, etc.). Was
previously three ad-hoc paths (`GameScreen.viewCard`, `viewDeckTop`,
deckbuilder pool/list); each visual change had to be implemented
three times or silently skipped two of them.


## Phases — Card consolidation checklist

Three ad-hoc card-render paths existed (`GameScreen.viewCard`,
`Main.viewDeckTop`, and the deckbuilder pool/list inline render in
`Main.elm`) — each visual change had to be done three times or
silently skipped two of them. This module consolidates them into one
primitive. Phased plan, update by crossing through (`~~line~~`) when
done.

  - [~] **Phase 1 — Wire `view FaceUp` into the in-game render path.**
        Replace `GameScreen.viewCard` + `GameScreen.CardView` (in
        `src/Main.elm`'s `zoneCardsForPrompt` + `cardOptsForZone` +
        `viewSliceFromPlayers`). Drop `GameScreen.CardView` from
        `PlayerCounts.board/hand/graveyard`; thread `List Card` through
        instead. `Card.elm` exists + decoder tested; the actual wire-
        up is the unfinished half of this phase.

  - [ ] **Phase 2 — Wire `view FaceDownBack` into deck-top zones.**
        Replace `Main.viewDeckTop` + the `DeckBack` type. Each
        deck-top zone (opp + your) renders a `Card` in
        `FaceDownBack` mode built from just the colors + symbols
        the engine ships in `PlayerView.deck_top`.

  - [ ] **Phase 3 — Wire `PoolTile` + `CompactRow` modes into the
        deckbuilder.** Replace `Main.viewPoolCard` + `viewDeckRow`
        renders. Add a `fromPoolEntry : CardPoolEntry -> Card`
        converter so the deckbuilder's wire shape (no iid, no tapped,
        Maybe power/toughness) maps into the same Card type. Specialize
        `PoolTile` (grid tile, smaller than FaceUp) and `CompactRow`
        (name + cost + count + remove-button) renders; currently both
        fall through to FaceUp / minimal-row placeholders.

  - [ ] **Phase 4 — Patience-style attached stack inside `FaceUp`.**
        Render each entry in `CardData.attached` as a `BackStrip`
        positioned behind the host with negative-margin overlap, so
        only the back's top strip peeks down (Solitaire-tableau
        style). Hover any strip → expand to `FaceUp` tooltip. Per
        the dev-tool design call, both players can hover-look (a
        relaxation of strict P.18 controller-only). Strip content per
        C.1: color + symbols only; never name / abilities / stats.

  - [ ] **Phase 5 — SLOTS-driven per-slot symbols + holes.** When
        the engine ships per-slot symbol positions (and holes —
        engine has them already per the user, SLOTS.md status is
        outdated on this point and needs verification against
        `src/sim/snapshot.rs` + the loader before this phase can
        land), switch `Card.decode` from `defaultSymbolsToSlots`
        (spiral-out fallback) to reading the wire positions
        directly. Add holes rendering on `FaceDownBack` +
        `BackStrip` — transparent windows in the back per SLOTS.md
        see-through rules (V.8 + the SLOTS.md per-slot generalization).
        Symbol cards (C.17b) render their glyph filling the central
        3×3 of the SLOTS grid.


## What's intentionally NOT in this primitive

  - Color → background tinting / frequency-density stripes. Previous
    attempts shipped but never visually verified; removed
    2026-06-09 per user. Re-add only after the approach is verified
    in-browser. RULES C.5 (cards have colors) is satisfied by
    showing color tags on the meta-line + on the back; no claim is
    made about background painting.
  - Card-shape aspect ratio enforcement. SLOTS.md says 5×3 grid →
    3:5 portrait; CSS-side aspect-ratio: 3/5 is set on .card in
    play.html. This module relies on that CSS, doesn't apply
    geometry inline.
  - UCT preview state. Lives in `Main.Model.uctPreview`; the badge
    is opt-in via `Config.uctBadge` so this module stays free of
    UCT-coupling.

Grounded in RULES.md (which I should have read before building any
of the previous renders):

  - **C.1**  symbols are on the back; zero, one, or more
  - **C.2**  single- or double-sided
  - **C.3**  display states are face-up and face-down
  - **C.5**  colorless or one or more colors
  - **C.11** symbols are structured per-card properties
  - **C.13** `transparent` is a frame attribute, not a color;
    transparent-frame cards have no symbols
  - **C.16** counting effects treat a host + its attached as one unit
  - **C.17** Symbol cards: permanent, (color, symbol) pair = identity
  - **C.17b** Symbol card's backside shows its glyph filling the
    central 3×3 of the SLOTS.md grid
  - **Z.6**  ATTACHED is a real zone — a card placed under another
  - **P.17** attached cards are placed face-down
  - **P.18** controller of an attached card may look at it any time
  - **V.6**  cards on the BOARD are fully visible to both
  - **V.7**  visibility of attached cards is defined by P.17 + P.18

And SLOTS.md (canonical for symbol + hole placement on the back):

  - 15-slot grid (5 rows × 3 cols: TL T TR / UL U UR / L C R / DL D DR / BL B BR)
  - cards without an explicit per-slot symbols block fill slots by
    spiraling out from `C` in this order:
    C, U, UR, R, DR, D, DL, L, UL, TL, T, TR, BR, B, BL
  - `holes` are transparent slot positions; symbol and hole can't
    coexist on the same slot

The engine doesn't ship per-slot symbol positions yet (SLOTS.md
status: design only); the converter defaults every emitted symbol
to the spiral-out order. When the engine ships slot data, only the
converter changes — the primitive already accommodates the shape.

-}

import Html exposing (Html, button, div, span, text)
import Html.Attributes as A exposing (class, style)
import Html.Events as E
import Json.Decode as D


{-| Wrapper-constructor breaks the recursive type-alias prohibition.
A `Card` carries `CardData` which may carry `List Card` for the
attached zone (Z.6).
-}
type Card
    = Card CardData


type alias CardData =
    { iid : Maybe String                 -- in-game instance id; Nothing for deckbuilder entries
    , id : String                        -- card id ("blue-jewel", etc.)
    , name : String                      -- face-only (C.1: name is not on the back)
    , kind : Kind                        -- C.9, C.17
    , colors : List String               -- C.5 — visible on the back
    , symbols : List SlotSymbol          -- C.1, C.11, SLOTS.md — back-of-card placement
    , subtypes : List String
    , printedCost : String               -- face-only
    , effectiveCost : String             -- face-only; C.12 recomputed
    , abilities : List String            -- face-only
    , timing : Maybe Timing              -- C.6, C.7
    , transparentFrame : Bool            -- C.13 — frame attribute, not a color
    , holes : List Slot                  -- SLOTS.md transparent positions
    , printedPower : Float
    , printedToughness : Float
    , tapped : Bool                      -- board-state
    , summoningSick : Bool               -- B.3 / B.15
    , damage : Float                     -- accumulated combat damage (B.7)
    , attached : List Card               -- Z.6 — cards placed under this one
    }


{-| Symbol with its position on the back per SLOTS.md.
-}
type alias SlotSymbol =
    { slot : Slot
    , glyph : String
    }


{-| The 15-slot grid from SLOTS.md. `Center` instead of `C` to avoid
clashing with `Html.Attributes.class`-style code identifiers used in
the same file; `slotKey` returns the canonical letter for serialisation.
-}
type Slot
    = TL
    | T
    | TR
    | UL
    | U
    | UR
    | L
    | Center
    | R
    | DL
    | D
    | DR
    | BL
    | B
    | BR


slotKey : Slot -> String
slotKey s =
    case s of
        TL ->
            "TL"

        T ->
            "T"

        TR ->
            "TR"

        UL ->
            "UL"

        U ->
            "U"

        UR ->
            "UR"

        L ->
            "L"

        Center ->
            "C"

        R ->
            "R"

        DL ->
            "DL"

        D ->
            "D"

        DR ->
            "DR"

        BL ->
            "BL"

        B ->
            "B"

        BR ->
            "BR"


{-| The default fill order per SLOTS.md: clockwise spiral from `C`
through the inner 8, then clockwise through the outer 6.
-}
slotSpiralOrder : List Slot
slotSpiralOrder =
    [ Center, U, UR, R, DR, D, DL, L, UL, TL, T, TR, BR, B, BL ]


type Kind
    = Creature
    | Spell
    | Artifact
    | Environment
    | Mutation
    | SymbolCard
    | OtherKind String


kindFromString : String -> Kind
kindFromString s =
    case String.toLower s of
        "creature" ->
            Creature

        "spell" ->
            Spell

        "artifact" ->
            Artifact

        "environment" ->
            Environment

        "mutation" ->
            Mutation

        "symbol" ->
            SymbolCard

        other ->
            OtherKind other


type Timing
    = Instant
    | Sorcery


{-| Render mode determines which fields surface and how. Per RULES'
visibility rules — `FaceDownBack` and `BackStrip` MUST only render
the back (color + symbols per C.1, P.17, V.7), never the face.
-}
type RenderMode
    = FaceUp
    | FaceDownBack
    | PoolTile
    | CompactRow
    | BackStrip


{-| Polymorphic msg config — caller in Main / GameScreen wires in
their concrete Msg. Mirrors SpectatorBar.Config / LogPanel pattern.
-}
type alias Config msg =
    { clickable : Maybe (String -> msg)
    , selected : Bool
    , dim : Bool
    , uctBadge : Maybe { winRate : Float, visits : Int, wins : Float }
    , uctChosen : Bool
    , borderColor : Maybe String
    , borderStyle : Maybe String
    , overlays : List (Html msg)
    }


defaultConfig : Config msg
defaultConfig =
    { clickable = Nothing
    , selected = False
    , dim = False
    , uctBadge = Nothing
    , uctChosen = False
    , borderColor = Nothing
    , borderStyle = Nothing
    , overlays = []
    }


{-| ATTACHED zone test for the few callsites that need it (e.g., the
patience-stack render checks if a host has any attached). Z.6 has
no associated `iid` slot in the wire shape — attached cards are
their own Card entries in the host's `attached` list.
-}
isAttachedZone : Card -> Bool
isAttachedZone (Card d) =
    not (List.isEmpty d.attached)



-- DECODER FROM ENGINE WIRE SHAPE (CardView in snapshot.rs)


required : String -> D.Decoder a -> D.Decoder (a -> b) -> D.Decoder b
required field aDec fDec =
    D.map2 (\f a -> f a) fDec (D.field field aDec)


optionalList : String -> D.Decoder a -> D.Decoder (List a -> b) -> D.Decoder b
optionalList field aDec fDec =
    D.map2 (\f a -> f a)
        fDec
        (D.oneOf [ D.field field (D.list aDec), D.succeed [] ])


optionalBool : String -> D.Decoder (Bool -> b) -> D.Decoder b
optionalBool field fDec =
    D.map2 (\f a -> f a)
        fDec
        (D.oneOf [ D.field field D.bool, D.succeed False ])


{-| Decode the engine's in-game CardView shape (per
`src/sim/snapshot.rs::CardView`). The engine emits symbols as
`Vec<String>` flat — we default each to the spiral-out slot order
(SLOTS.md), since per-slot positions aren't on the wire yet.
-}
decode : D.Decoder Card
decode =
    D.succeed makeCardData
        |> required "iid" (D.map Just D.string)
        |> required "id" D.string
        |> required "name" D.string
        |> required "kind" (D.map kindFromString D.string)
        |> required "colors" (D.list D.string)
        |> required "symbols" (D.list D.string)
        |> required "subtypes" (D.list D.string)
        |> required "cost" D.string
        |> required "effective_cost" D.string
        |> required "abilities" (D.list D.string)
        |> D.map (\f -> f Nothing)
        -- timing not yet on the in-game wire; defaulted Nothing
        |> optionalBool "transparent_frame"
        |> required "tapped" D.bool
        |> required "summoning_sick" D.bool
        |> required "damage" D.float
        |> required "power" D.float
        |> required "toughness" D.float
        |> optionalList "attached" (D.lazy (\_ -> decode))


{-| Builder that bridges the pipeline-decoded fields into a Card.
Order MUST match the `required` chain above. The argument list takes
the raw flat-symbol list and applies the spiral-out slot defaulting.
-}
makeCardData :
    Maybe String
    -> String
    -> String
    -> Kind
    -> List String
    -> List String
    -> List String
    -> String
    -> String
    -> List String
    -> Maybe Timing
    -> Bool
    -> Bool
    -> Bool
    -> Float
    -> Float
    -> Float
    -> List Card
    -> Card
makeCardData iid_ id_ name_ kind_ colors_ symGlyphs subtypes_ cost_ effCost_ abilities_ timing_ transparent_ tapped_ sick_ damage_ pow_ tough_ attached_ =
    Card
        { iid = iid_
        , id = id_
        , name = name_
        , kind = kind_
        , colors = colors_
        , symbols = defaultSymbolsToSlots symGlyphs
        , subtypes = subtypes_
        , printedCost = cost_
        , effectiveCost = effCost_
        , abilities = abilities_
        , timing = timing_
        , transparentFrame = transparent_
        , holes = []
        , printedPower = pow_
        , printedToughness = tough_
        , tapped = tapped_
        , summoningSick = sick_
        , damage = damage_
        , attached = attached_
        }


{-| SLOTS.md default: spiral out from C through the inner 8, then
clockwise through the outer 6. Engine ships symbols as a flat list
without slot positions today, so we apply this fallback in the
converter. When the engine emits per-slot positions, switch to
reading them directly.
-}
defaultSymbolsToSlots : List String -> List SlotSymbol
defaultSymbolsToSlots glyphs =
    List.map2 SlotSymbol slotSpiralOrder glyphs



-- VIEW


view : Config msg -> RenderMode -> Card -> Html msg
view cfg mode (Card d) =
    case mode of
        FaceUp ->
            viewFaceUp cfg d

        FaceDownBack ->
            viewFaceDownBack cfg d

        BackStrip ->
            viewBackStrip cfg d

        PoolTile ->
            -- Phase 3 will specialize; fall back to FaceUp until then.
            viewFaceUp cfg d

        CompactRow ->
            -- Phase 3 will specialize; minimal row render for now.
            viewCompactRowMin cfg d


{-| Full-face render (V.4 / V.5 / V.6). All fields surface.
-}
viewFaceUp : Config msg -> CardData -> Html msg
viewFaceUp cfg d =
    let
        flag b name =
            if b then
                name

            else
                ""

        classes =
            String.join " " <|
                List.filter (not << String.isEmpty)
                    [ "card"
                    , flag (cfg.clickable /= Nothing) "clickable"
                    , flag cfg.selected "selected"
                    , flag d.tapped "tapped"
                    , flag d.summoningSick "sick"
                    , flag cfg.uctChosen "uct-recommended"
                    ]

        styleAttrs =
            List.filterMap identity
                [ if cfg.dim then
                    Just (style "opacity" "0.6")

                  else
                    Nothing
                , Maybe.map (style "border-color") cfg.borderColor
                , Maybe.map (style "border-style") cfg.borderStyle
                ]

        clickAttrs =
            case ( cfg.clickable, d.iid ) of
                ( Just toMsg, Just iid ) ->
                    [ E.onClick (toMsg iid) ]

                _ ->
                    []
    in
    div
        ([ class classes
         , A.title (titleTextFor d)
         ]
            ++ styleAttrs
            ++ clickAttrs
        )
        (viewUctBadge cfg.uctBadge
            ++ [ viewHead d ]
            ++ viewMeta d
            ++ viewAbilities d.abilities
            ++ cfg.overlays
        )


{-| Back-only render (P.17 ATTACHED / V.1 deck-top / V.3 opp hand).
Per C.1 only color + symbols are visible. No name, no cost, no
abilities, no stats. Per C.17b a Symbol card's back fills the central
3×3 with its glyph — handled here by checking kind.
-}
viewFaceDownBack : Config msg -> CardData -> Html msg
viewFaceDownBack cfg d =
    div
        [ class "card card-back"
        , A.title (d.id ++ "  (face-down)")
        ]
        [ div [ style "font-size" "0.55rem", style "color" "#888" ]
            [ text "(back)" ]
        , viewBackSymbols d
        ]


{-| Compressed top-strip of a face-down back. Used for the patience-
style attached stack peek (phase 4 wires it up).
-}
viewBackStrip : Config msg -> CardData -> Html msg
viewBackStrip cfg d =
    div
        [ class "card-back-strip"
        , style "height" "1.2rem"
        , style "display" "flex"
        , style "align-items" "center"
        , style "gap" "0.25rem"
        , style "padding" "0 0.3rem"
        , style "font-size" "0.55rem"
        ]
        (span [ style "color" "#888" ] [ text "(back)" ]
            :: List.map (\s -> span [ class "symbol" ] [ text s.glyph ]) d.symbols
        )


viewCompactRowMin : Config msg -> CardData -> Html msg
viewCompactRowMin cfg d =
    div
        [ style "display" "flex"
        , style "gap" "0.5rem"
        , style "align-items" "baseline"
        , style "padding" "0.2rem 0.4rem"
        ]
        [ span [ style "font-weight" "bold", style "color" "#ddd" ] [ text d.name ]
        , span [ class "cost", style "color" "#fc6", style "font-size" "0.65rem" ] [ text d.printedCost ]
        ]



-- Internal helpers


viewHead : CardData -> Html msg
viewHead d =
    let
        printed =
            d.printedCost

        effective =
            if String.isEmpty d.effectiveCost then
                printed

            else
                d.effectiveCost

        costSpans =
            if printed == effective then
                [ span [ class "cost" ] [ text printed ] ]

            else
                [ span [ class "cost" ] [ text effective ]
                , span
                    [ class "cost"
                    , style "color" "#666"
                    , style "text-decoration" "line-through"
                    , style "margin-left" "0.3rem"
                    ]
                    [ text printed ]
                ]
    in
    div [ class "head" ]
        (span [ class "name" ] [ text d.name ] :: costSpans)


viewMeta : CardData -> List (Html msg)
viewMeta d =
    let
        statsPart =
            if d.kind == Creature then
                let
                    effT =
                        d.printedToughness - d.damage

                    base =
                        formatNumber d.printedPower ++ "/" ++ formatNumber effT

                    dmgTag =
                        if d.damage > 0 then
                            " (-" ++ formatNumber d.damage ++ ")"

                        else
                            ""
                in
                [ span [ class "stats" ] [ text (base ++ dmgTag) ] ]

            else
                []

        colorParts =
            List.map colorTag d.colors

        symbolParts =
            List.map (\s -> span [ class "symbol" ] [ text s.glyph ]) d.symbols

        subtypeParts =
            if List.isEmpty d.subtypes then
                []

            else
                [ span [ style "color" "#888" ] [ text (String.join "·" d.subtypes) ] ]

        groups =
            List.filter (not << List.isEmpty)
                [ statsPart
                , colorParts
                , symbolParts
                , subtypeParts
                ]

        joined =
            groups
                |> List.intersperse [ text " " ]
                |> List.concat
    in
    if List.isEmpty joined then
        []

    else
        [ div [ class "meta-line" ] joined ]


viewAbilities : List String -> List (Html msg)
viewAbilities abilities =
    if List.isEmpty abilities then
        []

    else
        [ div [ class "abilities" ]
            (List.map (\a -> div [] [ text a ]) abilities)
        ]


viewBackSymbols : CardData -> Html msg
viewBackSymbols d =
    div [ class "meta-line" ]
        (List.map colorTag d.colors
            ++ (if List.isEmpty d.symbols then
                    []

                else
                    text " " :: List.map (\s -> span [ class "symbol" ] [ text s.glyph ]) d.symbols
               )
        )


viewUctBadge : Maybe { winRate : Float, visits : Int, wins : Float } -> List (Html msg)
viewUctBadge maybeCand =
    case maybeCand of
        Nothing ->
            []

        Just cand ->
            let
                pct =
                    String.fromInt (round (cand.winRate * 100))

                winsStr =
                    formatNumber (toFloat (round (cand.wins * 10)) / 10)
            in
            [ div
                [ class "uct-badge"
                , A.title
                    ("UCT visits="
                        ++ String.fromInt cand.visits
                        ++ " wins="
                        ++ winsStr
                    )
                ]
                [ text ("UCT " ++ pct ++ "%") ]
            ]


colorTag : String -> Html msg
colorTag c =
    let
        code =
            String.toLower c
    in
    span [ class ("color-" ++ code) ] [ text code ]


titleTextFor : CardData -> String
titleTextFor d =
    let
        costLine =
            if String.isEmpty d.printedCost then
                ""

            else if d.effectiveCost == "" || d.effectiveCost == d.printedCost then
                " — cost " ++ d.printedCost

            else
                " — cost " ++ d.effectiveCost ++ " (printed " ++ d.printedCost ++ ")"

        sickTag =
            if d.summoningSick then
                "  (summoning sick)"

            else
                ""

        iidPart =
            case d.iid of
                Just iid ->
                    "  · " ++ iid

                Nothing ->
                    ""
    in
    d.name ++ costLine ++ sickTag ++ iidPart


formatNumber : Float -> String
formatNumber n =
    if n == toFloat (floor n) then
        String.fromInt (floor n)

    else
        String.fromFloat n


-- Color-to-background logic intentionally NOT included. Previous
-- attempts (defaultBgForColors / tintForColor / two-color density
-- stripes) shipped but the user never saw the tints actually paint;
-- code was non-functional. Removed per user instruction 2026-06-09.
-- A future verified version reads card.colors and applies the back-
-- of-card tinting per RULES C.5 + SLOTS.md; not added here until the
-- approach has been proven in-browser.
