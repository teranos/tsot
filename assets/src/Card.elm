module Card exposing
    ( Card(..)
    , CardData
    , Config
    , Kind(..)
    , Slot(..)
    , SlotSymbol
    , Timing(..)
    , decode
    , defaultConfig
    , faceDownConfig
    , isAttachedZone
    , key
    , kindFromString
    , slotKey
    , slotSpiralOrder
    , styles
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

Two render modes only — `Front` and `Back` per RULES C.3 ("A card
has two display states: face-up and face-down"). The card primitive
ALWAYS renders the full card; "pool tile", "compact list row", and
"patience back strip" are NOT separate render modes — they're the
caller's container CSS. A compact row hides information, which is
harmful — every Card render shows the whole card.

  - [x] ~~**Phase 1 — Wire `Card.view Card.Front` into the in-game
        render path.**~~ Replaced `GameScreen.viewCard` + `CardView`
        + `CardOpts` + `decodeCardView`. PlayerCounts.board/hand/
        graveyard thread `List Card` through. Done in commit a2306ca.

  - [x] ~~**Phase 2 — Wire `Card.view Card.Back` into deck-top zones.**~~
        Replaced `Main.viewDeckTop` + `DeckBack` + the back-side
        color tags (C.1 violation in the old impl). Done in commit
        25022fd.

  - [x] ~~**Phase 3 — Wire `Card.view Card.Front` into the deckbuilder.**~~
        Replaced `Main.viewPoolCard` + `viewDeckRow` with full
        `Card.view Card.Front`. Deckbuilder shows the full card every
        time, not a compact pill or row — per user, "always need to
        see the full card", "a compact list is harmful because it
        hides information". The deckbuilder layout (grid for pool,
        list+count for deck) is container CSS, not a render-mode
        decomposition. `Main.poolEntryToCard` bridges the deckbuilder
        envelope (no iid, no tapped, Maybe power/toughness) into Card;
        `Card.viewFront`'s click fallback uses `id` when `iid` is
        Nothing so pool clicks still fire.

  - [ ] **Phase 4 — Patience-style attached stack.** Render each
        entry in `CardData.attached` as `Card.view Card.Back`
        positioned behind the host with negative-margin overlap and
        container overflow clipping (so only the top strip is
        visible). Hover any back → expand to `Card.view Card.Front`
        tooltip (per the dev-tool relaxation: both players hover-look,
        not strict P.18 controller-only). The "strip peek" is parent
        CSS — the card itself is rendered as full Back, the container
        clips.

  - [ ] **Phase 5 — SLOTS-driven per-slot symbols + holes (engine
        wire format verification required first).** Per the user
        2026-06-09, the engine HAS per-slot symbol positions and
        holes; SLOTS.md's "Status: design only" line is outdated.
        Verify the wire format in `src/sim/snapshot.rs` + `src/card.rs`
        + the loader, then switch `Card.decode` from
        `defaultSymbolsToSlots` (spiral-out fallback) to reading the
        wire positions directly. Render symbols at their actual slot
        positions on the 5×3 grid (CSS grid in `Back` mode). Symbol
        cards (C.17b) get their glyph filling the central 3×3. Holes
        render as transparent windows; see-through reveals from below
        per V.8 + the SLOTS.md per-slot generalization.


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

import Html exposing (Html, div, node, span, text)
import Html.Attributes as A exposing (class, style)
import Html.Events as E
import Html.Keyed as Keyed
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


{-| Polymorphic msg config — caller in Main / GameScreen wires in
their concrete Msg. Mirrors SpectatorBar.Config / LogPanel pattern.

Per CARD.md Axiom: a card is ONE DOM element across its lifetime.
There is no `RenderMode = Front | Back` dispatch — the same render
function always emits the full face-up DOM, with `.face-down` class
controlling visual state. CSS hides face-only children when
face-down; `:hover` (scoped to attached-rows) reverts. RULES C.3 is
two display STATES of the same card, not two renders.
-}
type alias Config msg =
    { clickable : Maybe (String -> msg)
    , selected : Bool
    , dim : Bool
    , faceDown : Bool
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
    , faceDown = False
    , uctBadge = Nothing
    , uctChosen = False
    , borderColor = Nothing
    , borderStyle = Nothing
    , overlays = []
    }


{-| Convenience: defaultConfig with `faceDown = True`. Used by
deck-top zones, the attached strip's internal helper, and anywhere
else a caller wants the face-down visual state without unpacking
the record. Per CARD.md Axiom this is just a state flag — same DOM
element, same `view` function.
-}
faceDownConfig : Config msg
faceDownConfig =
    { defaultConfig | faceDown = True }


{-| ATTACHED zone test for the few callsites that need it (e.g., the
patience-stack render checks if a host has any attached). Z.6 has
no associated `iid` slot in the wire shape — attached cards are
their own Card entries in the host's `attached` list.
-}
isAttachedZone : Card -> Bool
isAttachedZone (Card d) =
    not (List.isEmpty d.attached)


{-| Stable per-card identity for Html.Keyed (Slice 2 of CARD.md
Axiom): a card's iid uniquely identifies it across renders, so a
keyed container preserves the same DOM node when the card stays
in the zone (even if it reorders). Pool entries have no iid; fall
back to the card-id (still unique within the pool/deck contexts
they appear in).

Slice 2 closes intra-zone reorder destruction. Cross-zone
reparenting still destroys the node — Elm's vDOM has no
cross-parent identity. Slice 3 (single #card-pool top-level
container) is what closes that.
-}
key : Card -> String
key (Card d) =
    Maybe.withDefault d.id d.iid



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


{-| Entry point per CARD.md Axiom: ONE DOM element per card iid. The
function emits the full face-up DOM always; `cfg.faceDown=True` adds
a `.face-down` class that flips visual state via CSS (front-only
children get `display:none`). Hover-flip on attached cards lives in
CSS scoped to `.attached-row:hover` — same element, never a popover
or a second card node. Hosts with attached children render with an
in-place strip; the strip is OUTSIDE the host's `.card` but inside
the `.card-host` wrapper (same flex column).
-}
view : Config msg -> Card -> Html msg
view cfg (Card d) =
    if List.isEmpty d.attached then
        viewSingle cfg d

    else
        div [ class "card-host" ]
            [ viewSingle cfg d
            , Keyed.node "div"
                [ class "attached-strip" ]
                (List.map keyedAttachedRow d.attached)
            ]


keyedAttachedRow : Card -> ( String, Html msg )
keyedAttachedRow card =
    ( key card, viewAttachedRow card )


viewAttachedRow : Card -> Html msg
viewAttachedRow (Card d) =
    -- ONE element per attached iid. `.attached-row` clips to a thin
    -- visible strip; `:hover` on the row pops it out + reveals face
    -- via CSS on the same element. No second DOM node. Slice 2 of
    -- the Axiom roadmap keys the parent .attached-strip on iid so
    -- the row itself is identity-preserved across renders too.
    div [ class "attached-row" ]
        [ viewSingle faceDownConfig d ]


{-| All card-internal CSS — the visual contract of the primitive.
Returns a `<style>` element that Main mounts once at the top of the
page. Card.elm owns the class names AND their rules in one place;
classes-in-Elm / rules-in-stylesheet split is gone.

Scope: ONLY rules that describe the card itself or its sub-pieces
(`.card`, `.head`, `.name`, `.cost`, `.meta-line`, `.symbol`,
`.color-*`, `.stats`, `.abilities`, `.card.clickable/.selected/.tapped/.sick/.uct-recommended`,
`.uct-badge`, `.card-back`). Container layout (`.cards`, `.pool-grid`),
contextual overrides (`.opponent .card`), and caller-specific
decorations (`.pool-card::after` `+` badge) stay with their callers
in play.html — they're not part of the card's visual contract,
they're how the caller arranges or annotates cards in a surface.

Width is a single value (was 8.5–11rem range — flex would grow each
row's cards differently, and aspect-ratio: 3/5 then propagated that
into different heights). One width + aspect-ratio → every card is
identical w×h.
-}
styles : Html msg
styles =
    node "style" [] [ text cardCss ]


cardCss : String
cardCss =
    """
    .card {
      display: flex; flex-direction: column;
      padding: 0.35rem 0.5rem;
      background: #1c1c20; border: 1px solid #444;
      border-radius: 4px;
      width: 9rem;
      aspect-ratio: 3 / 5;
      font-size: 0.7rem;
      position: relative;
      flex: 0 0 auto;
    }
    .card.clickable { cursor: pointer; border-color: #4af; }
    .card.clickable:hover { background: #234; }
    .card.selected { background: #2a3a4f; border-color: #6cf; }
    .card.tapped { opacity: 0.5; transform: rotate(6deg); }
    .card.sick { border-style: dashed; }
    .head { display: flex; justify-content: space-between; align-items: baseline; gap: 0.4rem; }
    .name { font-weight: bold; color: #eee; }
    .cost { color: #fc6; font-size: 0.65rem; }
    .meta-line { color: #888; font-size: 0.65rem; display: flex; gap: 0.4rem; flex-wrap: wrap; margin-top: 0.1rem; }
    .symbol { color: #6cf; }
    .color-w { color: #ddd; }
    .color-u { color: #4af; }
    .color-b { color: #b6f; }
    .color-r { color: #f66; }
    .color-g { color: #6f6; }
    .color-c { color: #aaa; }
    .stats { color: #fc6; font-weight: bold; }
    .abilities { color: #bbb; font-size: 0.65rem; margin-top: 0.2rem; line-height: 1.25; max-height: 4rem; overflow: hidden; }
    .abilities li { margin-left: 0.7rem; }
    .card.uct-recommended { border-color: #6f9; box-shadow: 0 0 0 1px rgba(102, 255, 153, 0.4) inset; }
    .uct-badge { position: absolute; top: 2px; right: 4px; color: #6f9; font-size: 0.6rem; font-weight: bold; pointer-events: none; }

    /* Face-down state — per CARD.md Axiom: same DOM element, different
       visual state. Hide head, abilities, and every meta-line child
       that isn't a .symbol (stats / color tags / subtypes). Per RULES
       C.1 the back shows ONLY symbols. The .face-down rules apply
       wherever the card sits — deck-top, attached strip, future
       face-down hand cards. */
    .card.face-down .head,
    .card.face-down .abilities,
    .card.face-down .meta-line > :not(.symbol) { display: none; }

    /* Attached-card strip below the host (Phase 4 / Slice 1). Same
       DOM element as a normal card — just rendered with faceDown=True
       and wrapped in an .attached-row that clips to a 1.8rem strip
       (so visually you see the top edge of a card-back tucked under
       the host). On `.attached-row:hover` the row's overflow becomes
       visible AND the descendant `.card.face-down`'s display:none
       rules are reverted — the SAME element flips to face-up, in
       place. No popover, no second .card, no degenerated tooltip
       representation. Z-index lifts on hover so the revealed face
       overlays sibling rows below.

       Note: the `.attached-row > .card` width/aspect-ratio still
       resolves to 9rem × 15rem; the row clip makes only the top
       1.8rem visible by default. The bottom 13.2rem is the
       "tucked-behind" portion that emerges on hover. */
    .card-host { display: flex; flex-direction: column; flex: 0 0 auto; }
    .attached-strip { display: flex; flex-direction: column; width: 9rem; margin-top: 0.15rem; }
    .attached-row {
      position: relative; width: 9rem; height: 1.8rem;
      overflow: hidden;
      margin-top: -1px;
    }
    .attached-row:first-child { margin-top: 0; }
    .attached-row:hover { overflow: visible; z-index: 100; }
    .attached-row:hover .card.face-down .head,
    .attached-row:hover .card.face-down .abilities,
    .attached-row:hover .card.face-down .meta-line > :not(.symbol) { display: revert; }
    """


{-| Single render path per CARD.md Axiom. Always emits the full
face-up DOM (head + meta + abilities + uct-badge + overlays). The
`.face-down` class is the only visual differentiator between states:
CSS hides head + abilities + non-symbol meta children when face-down,
and `.attached-row:hover` reverts those rules so the card flips back
to face-up — same element, no second node, no popover. C.1 (back =
symbols only) and C.5 (front carries color) are enforced via the
CSS, not via separate render branches.

`viewBackSymbols`-style symbols-only rendering doesn't exist anymore
— the meta-line carries everything; CSS picks which children to show
based on `.face-down`.
-}
viewSingle : Config msg -> CardData -> Html msg
viewSingle cfg d =
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
                    , flag cfg.faceDown "face-down"
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
            case cfg.clickable of
                Just toMsg ->
                    -- iid for in-game instances; fall back to card-id
                    -- for deckbuilder pool entries (no instance yet).
                    [ E.onClick (toMsg (Maybe.withDefault d.id d.iid)) ]

                Nothing ->
                    []

        iidAttr =
            -- Per CARD.md Axiom Slice 2/3: stable per-card identity.
            -- Falls back to `id` when iid is Nothing (deckbuilder).
            A.attribute "data-iid" (Maybe.withDefault d.id d.iid)
    in
    div
        ([ class classes
         , iidAttr
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
