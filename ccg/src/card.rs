mod loader;

pub use loader::{load_card, load_cards_dir, load_cards_embedded};

use mlua::{Function, Lua, LuaOptions, StdLib, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Owns the long-lived Lua VM and the cards loaded into it.
///
/// The VM outlives the cards because future card fields (event handlers like
/// `on_die`, `static`) will be `mlua::Function` values whose validity is tied
/// to this `Lua`. Built once at startup; not mutated during a game.
pub struct CardRegistry {
    lua: Lua,
    cards: Vec<Card>,
}

impl CardRegistry {
    /// Load every `.lua` file in `dir` into a fresh sandboxed VM.
    ///
    /// `os`, `io`, `package`, and `debug` are not loaded as stdlib. The base
    /// library is always loaded in Lua, so the dangerous loader functions
    /// (`load`, `loadstring`, `loadfile`, `dofile`) are explicitly nil'd in
    /// globals afterward. `math`, `string`, `table`, and `coroutine` remain
    /// (coroutine is required for Phase 2's choice API).
    pub fn load(dir: &Path) -> mlua::Result<Self> {
        let safe_libs = StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::COROUTINE;
        let lua = Lua::new_with(safe_libs, LuaOptions::default())?;
        {
            let globals = lua.globals();
            for forbidden in ["load", "loadstring", "loadfile", "dofile"] {
                globals.set(forbidden, Value::Nil)?;
            }
        }
        let cards = load_cards_dir(&lua, dir)?;
        Ok(Self { lua, cards })
    }

    /// Load every embedded card (compiled into the binary from
    /// `$CARGO_MANIFEST_DIR/cards` at build time). Used by production
    /// entry points so the runtime has no filesystem dependency — works
    /// identically on native and on `wasm32-unknown-emscripten`.
    pub fn load_embedded() -> mlua::Result<Self> {
        let safe_libs = StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::COROUTINE;
        let lua = Lua::new_with(safe_libs, LuaOptions::default())?;
        {
            let globals = lua.globals();
            for forbidden in ["load", "loadstring", "loadfile", "dofile"] {
                globals.set(forbidden, Value::Nil)?;
            }
        }
        let cards = load_cards_embedded(&lua)?;
        Ok(Self { lua, cards })
    }

    pub fn cards(&self) -> &[Card] {
        &self.cards
    }

    pub fn lua(&self) -> &Lua {
        &self.lua
    }

    /// Look up a card by its `id` field. Linear scan — the registry holds
    /// a few dozen cards, and EA-side helpers calling this stay well under
    /// the per-game budget.
    pub fn get(&self, id: &str) -> Option<&Card> {
        self.cards.iter().find(|c| c.id == id)
    }
}

// Colors are open-ended strings stored in `Card.colors: Vec<String>`. The
// canonical wheel today is white/blue/black/red/green/colorless plus any
// custom color a card chooses to introduce (e.g., "purple"). The engine
// doesn't branch on color — it's identity/flavor data passed through to
// handlers via `game.card(iid).colors`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CardType {
    Unspecified,
    Creature,
    /// Non-permanent card that resolves to GRAVEYARD. The timing class
    /// (`Card.timing`) decides whether it can be cast at instant speed
    /// (any priority window) or only in your main phase.
    Spell,
    Artifact,
    Environment,
    /// Aura-style attachment: casts targeting a creature on BOARD,
    /// attaches to that creature, carries on-board static effects via
    /// `scope = "attached_host"`. HAND payments go to GRAVEYARD (like
    /// spells); the mutation itself attaches to the target.
    Mutation,
    /// Per RULES C.17 / P.37: a board-placed permanent keyed by exactly
    /// one printed color and one printed symbol. ETB untapped (P.37),
    /// no summoning sickness (C.17a), unique-in-play by `id` (P.36),
    /// at most one cast per turn (P.35). The cap, uniqueness, and
    /// top-of-deck casting (P.38) are enforced at the cast-validation
    /// site, not by this enum; the enum just identifies the kind.
    Symbol,
}

/// When a spell can be cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Timing {
    /// Castable at any time, including inside response windows.
    Instant,
    /// Main phase only. Cannot be cast inside a response window.
    Sorcery,
}

/// Slot geometry for symbol and hole placement. 15 positions arranged
/// 5 rows × 3 columns; center `C` is the default and only required
/// slot. Diagram (canonical — does not change):
///
/// ```text
/// TL  T  TR
/// UL  U  UR
/// L   C   R
/// DL  D  DR
/// BL  B  BR
/// ```
///
/// See `SLOTS.md` for the full design (per-slot symbols, holes, and
/// the see-through reveal rule). This enum is infrastructure for the
/// per-slot symbol / hole system; no `Card` field uses it yet.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
    serde::Serialize, serde::Deserialize,
)]
pub enum Slot {
    TL, T, TR,
    UL, U, UR,
    L, C, R,
    DL, D, DR,
    BL, B, BR,
}

impl Slot {
    /// Canonical iteration order — top-to-bottom, left-to-right. Used
    /// wherever the slot loop needs to be deterministic (rendering,
    /// per-slot reveal walks, dashboard grids).
    pub const ALL: [Slot; 15] = [
        Slot::TL, Slot::T, Slot::TR,
        Slot::UL, Slot::U, Slot::UR,
        Slot::L, Slot::C, Slot::R,
        Slot::DL, Slot::D, Slot::DR,
        Slot::BL, Slot::B, Slot::BR,
    ];

    /// Canonical fill order for array-form symbols — spirals out from
    /// `C` clockwise through the inner ring (U → UR → R → DR → D → DL
    /// → L → UL), then clockwise through the outer ring (TL → T → TR →
    /// BR → B → BL). Per SLOTS.md. The loader uses this when a card
    /// declares `symbols = {"X", "Y", "Z"}` without explicit slot keys.
    pub const SPIRAL: [Slot; 15] = [
        Slot::C,
        Slot::U, Slot::UR, Slot::R, Slot::DR, Slot::D, Slot::DL, Slot::L, Slot::UL,
        Slot::TL, Slot::T, Slot::TR, Slot::BR, Slot::B, Slot::BL,
    ];

    /// Short label (1-2 chars) for printing. Matches the labels in
    /// SLOTS.md's diagram and the `FromStr` accepted forms.
    pub fn as_str(self) -> &'static str {
        match self {
            Slot::TL => "TL", Slot::T => "T", Slot::TR => "TR",
            Slot::UL => "UL", Slot::U => "U", Slot::UR => "UR",
            Slot::L => "L", Slot::C => "C", Slot::R => "R",
            Slot::DL => "DL", Slot::D => "D", Slot::DR => "DR",
            Slot::BL => "BL", Slot::B => "B", Slot::BR => "BR",
        }
    }
}

impl std::fmt::Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Slot {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "TL" => Ok(Slot::TL),
            "T" => Ok(Slot::T),
            "TR" => Ok(Slot::TR),
            "UL" => Ok(Slot::UL),
            "U" => Ok(Slot::U),
            "UR" => Ok(Slot::UR),
            "L" => Ok(Slot::L),
            "C" => Ok(Slot::C),
            "R" => Ok(Slot::R),
            "DL" => Ok(Slot::DL),
            "D" => Ok(Slot::D),
            "DR" => Ok(Slot::DR),
            "BL" => Ok(Slot::BL),
            "B" => Ok(Slot::B),
            "BR" => Ok(Slot::BR),
            other => Err(format!(
                "unknown slot {other:?}; expected one of TL T TR UL U UR L C R DL D DR BL B BR"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CostSource {
    Hand,
    Mill,
    Graveyard,
    Sacrifice,
    SelfExile,
    Attached,
}

/// RULES P.32: declarative target categories for cast-time legality. The
/// engine has a built-in legality predicate per variant. Add a variant
/// when a new category of "target X" emerges in the corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Target {
    /// At least one item exists on the stack (counterspells need this).
    Chain,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CostComponent {
    pub amount: i32,
    pub source: CostSource,
    pub is_x: bool,
    /// For SACRIFICE-source components: restricts the sacrificable pool to
    /// cards of this CardType. None = any board card. Cinder-wurm uses
    /// `kind = Creature` to express "sacrifice a creature."
    #[serde(default)]
    pub kind: Option<CardType>,
}

/// Power (X) and toughness (Y) — both real-valued to allow sub-1
/// power (B.2 floors mill across all unblocked attackers per combat)
/// and fractional damage accumulation (B.7/B.8 compare exactly,
/// no rounding). See `cards/pale-apparition.lua` for the spec card
/// that motivated the f32 type.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub x: f32,
    pub y: f32,
}

/// Predicate side of a static ability: which cards on the BOARD receive
/// the effect. Phase 1 is declarative — engine evaluates against the
/// candidate's Card / CardInstance fields directly, no Lua call needed.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StaticAffects {
    /// Candidate must have at least one of these subtypes (case-insensitive
    /// match). Empty = no subtype filter.
    #[serde(default)]
    pub subtypes: Vec<String>,
    /// Candidate must have at least one of these colors. Empty = no color filter.
    #[serde(default)]
    pub colors: Vec<String>,
    /// "owner" → candidate.controller == source.controller. "opponent" →
    /// candidate.controller != source.controller. None → no controller filter.
    #[serde(default)]
    pub controller: Option<StaticController>,
    /// Candidate must not be the source itself.
    #[serde(default)]
    pub exclude_self: bool,
    /// Phase 2: scope of who the static can affect. Default = any on-board
    /// candidate matching the other predicates. `AttachedHost` = only the
    /// card this source is attached to (requires the source to be in some
    /// other card's `attached` list). When `AttachedHost`, the other
    /// predicates still apply (e.g., subtype filter further narrows).
    #[serde(default)]
    pub scope: StaticScope,
    /// Phase 2: candidate's CardType must match. None = no kind filter.
    /// Lets cards say "creatures you control" without enumerating subtypes.
    #[serde(default)]
    pub kind: Option<CardType>,
    /// Phase 3: candidate must have this (lowercase) keyword, evaluated
    /// via `GameState::has_keyword` (intrinsic OR static-granted). Lets
    /// cards say "creatures with flying you control cannot attack" without
    /// enumerating which creatures fly.
    #[serde(default)]
    pub has_keyword: Option<String>,
}

/// What set of cards a static can target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StaticScope {
    /// Any card on either BOARD passing the other affects predicates.
    #[default]
    Board,
    /// Only the card this source is attached to (host). Source must be in
    /// some host's `attached` list. Companion-bird grants flying to its
    /// host via this scope.
    AttachedHost,
    /// Only the source card itself. Used for "this creature has [keyword]
    /// when [condition]" — wandering-wizard's conditional flying.
    SourceOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StaticController {
    /// Same controller as the source.
    Owner,
    /// Different controller from the source.
    Opponent,
}

/// A static ability declared on a card. Phase 1: stat modifier only.
/// Phase 2: also `modifier_keyword` for keyword-grant statics (flying,
/// vigilance, etc.) and `condition` for state-reading predicates (graveyard
/// count thresholds, etc.). All fields can be set; e.g., ossuary combines
/// stat + keyword + condition. `affects` is the predicate against the
/// candidate; everything applies while the source is on the BOARD, the
/// `condition` (if any) is satisfied, and the `affects` predicate matches.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StaticDef {
    /// Predicate gating which cards on the BOARD this static applies to.
    pub affects: StaticAffects,
    /// State-reading gate. None = always active when the source is on
    /// board. Some(cond) = the static only fires when the engine's
    /// evaluation of `cond` against game state is true.
    #[serde(default)]
    pub condition: Option<StaticCondition>,
    /// Effects this static applies to matching candidates. Each entry is
    /// one continuous effect kind (`StaticEffect` enum). Dispatch code
    /// iterates this list and matches on the variant. Not serialized —
    /// effects carry Lua `ActivatedAbility` references rebound from the
    /// live CardRegistry per the same convention as `Card.handlers`.
    #[serde(skip, default)]
    pub effects: Vec<StaticEffect>,
}

/// Phase 3.5 cost reduction component on a static ability. Applied during
/// `play_card` cost computation: each on-board static whose `affects`
/// matches the cast card subtracts `amount` from the matching cost
/// source's requirement (minimum 0 per P.20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CostModifier {
    pub source: CostSource,
    pub amount: i32,
}

/// One kind of continuous effect a static ability can produce. A
/// `StaticDef` carries a `Vec<StaticEffect>` listing every effect the
/// static applies to matching candidates. Adding a new continuous-effect
/// kind means adding a variant here.
#[derive(Debug, Clone)]
pub enum StaticEffect {
    /// Stat (X/Y) modifier. Both axes carried together since a single
    /// static typically grants paired modifications (anthem +1/+1, hydra
    /// per-attached +1/+1). Either axis may be zero for asymmetric grants.
    StatBoost {
        x: ModifierValue,
        y: ModifierValue,
    },
    /// Keyword grant (e.g., flying, vigilance, haste). Lowercase string
    /// matching `GameState::has_keyword`.
    KeywordGrant(String),
    /// Behavior restriction (cannot_attack, cannot_be_cost_paid, etc.)
    /// imposed on matching candidates. Consulted at the corresponding
    /// engine choke point (declare_attacker, resolve_hand_payment, ...).
    Restrict(Restriction),
    /// Cost reduction. Applied during `play_card` cost computation and
    /// read at handler-side via `effective_combined_cost` (A.12).
    CostModify {
        source: CostSource,
        amount: i32,
    },
    /// Activated ability granted to matching candidates. The recipient
    /// pays the cost and resolves the effect as the activation source.
    GrantActivated(ActivatedAbility),
    /// Color granted to matching candidates (e.g., GFP grants green).
    /// Unioned with printed colors by `GameState::effective_colors`.
    GrantColor(String),
    /// Face attribute granted to matching candidates (e.g., GFP grants
    /// `glow`). Read by `GameState::effective_face`.
    GrantFace(String),
    /// Subtractive: while this static is active on the target, the
    /// target's effective color identity is empty. Read by
    /// `GameState::host_loses_colors`. Nonsense Mutation uses this.
    MakesHostColorless,
    /// Subtractive: while this static is active on the target, the
    /// target's own abilities (printed + granted) are suppressed —
    /// its static_def stops applying, its handlers don't fire, its
    /// activated abilities can't be initiated. Read by
    /// `GameState::host_loses_abilities`. Nonsense Mutation uses this.
    SuppressesHostAbilities,
}

/// Phase 1.5 dynamic stat-modifier value. Resolved to an `i32` against the
/// source CardInstance's current state every time `effective_stats` runs,
/// so the value automatically tracks attached-set changes.
///
/// Lua parser accepts either a bare integer (`x = 2` → `Fixed(2)`) or a
/// short string descriptor. `"attached"` maps to `AttachedCount`;
/// `"attached:blue"` maps to `AttachedCountByColor("blue")`;
/// `"attached:type:mutation"` maps to `AttachedCountByKind(Mutation)`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ModifierValue {
    Fixed(f32),
    /// Count of cards in the source's `attached` list.
    AttachedCount,
    /// Count of attached cards whose `colors` contains the given lowercase color.
    AttachedCountByColor(String),
    /// Count of attached cards whose `kind` matches.
    AttachedCountByKind(CardType),
    /// `multiplier × attached_count`. Parsed from `"N*attached"` strings
    /// in card .lua files. `"attached"` is equivalent to `"1*attached"`.
    /// Lets a card scale a per-attached bonus without needing a new
    /// hardcoded multiplier per design.
    AttachedCountScaled(i32),
    /// RULES C.16: count of cards on the BOARD (across both players).
    /// Each BOARD card counts as 1; attached cards do not contribute.
    /// Used by Primal Toad's "+X/+Y where X is the number of cards in
    /// play." Parsed from `"board"` in card .lua files.
    BoardCount,
    /// Count of BOARD cards (across both players) whose `face` contains
    /// the given lowercase face attribute. Used by Missense Mutation
    /// (`+1/-0.25 per shiny card on the board`). Parsed from
    /// `"board:face:shiny"` etc. in card .lua files.
    BoardCountByFace(String),
    /// Sum of nested values. Used to compose a constant offset with one
    /// or more per-X scaled contributions in a single modifier slot.
    /// Missense Mutation Y: `Sum([Fixed(-0.5), Scaled(-0.25,
    /// BoardCountByFace("shiny"))])` → `-0.5 + (-0.25 × shiny_count)`.
    Sum(Vec<ModifierValue>),
    /// Multiplier × inner value. Lets a non-integer multiplier (e.g.
    /// -0.25) scale any other ModifierValue. Mirrors the existing
    /// AttachedCountScaled(i32) pattern but generalized to f32 and to
    /// any inner expression.
    Scaled(f32, Box<ModifierValue>),
    /// Count of cards in both players' HAND zones. Parsed from `"hands"`.
    HandCount,
    /// Count of distinct card *types* (CardType, subtypes excluded) across
    /// every card on the BOARD on both players' sides. Parsed from
    /// `"board_types"` in card .lua files. Used by Primal Toad's
    /// "+X where X is the number of types in play" variant.
    BoardTypeCount,
}

impl Default for ModifierValue {
    fn default() -> Self {
        ModifierValue::Fixed(0.0)
    }
}

/// Phase 3 action restriction. Each variant maps to one engine choke point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Restriction {
    /// Candidate cannot be declared as an attacker. Checked by
    /// `declare_attacker` before tap-and-attack mutations.
    CannotAttack,
    /// Candidate cannot be chosen as a HAND payment when paying a cost.
    /// Checked by `resolve_hand_payment` (filtered out of the pool) and
    /// by `play_card`'s payment validation.
    CannotBeCostPaid,
    /// Candidate cannot host attached cards. Refuses Mutation casts
    /// (P.26) that try to attach to this creature. The glass-insect
    /// cycle uses this to be unattachable.
    CannotBeAttachedTo,
}

/// Declarative state-reading predicate for STATIC Phase 2. Each variant
/// is evaluated by the engine against game state at static-application
/// time. "Owner" means the source's controller.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StaticCondition {
    /// Owner's GRAVEYARD has at least `min` cards. Ossuary uses this with
    /// `min = 5`.
    OwnerGraveyardSize { min: usize },
    /// Owner's GRAVEYARD contains at least `min` cards whose kind is not
    /// `CardType::Creature`. Wandering Wizard uses this with `min = 4`.
    OwnerGraveyardNonCreatures { min: usize },
    /// True iff the symbols on the effective top card of the owner's
    /// DECK (per V.8 — walk down through any transparent cards to the
    /// first opaque card) share at least one element with the symbols
    /// of any card currently attached to the source. flyer-match uses
    /// this to trigger its conditional +3/+0.
    DeckTopSymbolMatchesAttached,
}

/// Event handler keys recognised on card files. Matches LUA.md Phase 1 taxonomy
/// plus `OnBlockedBy` (the squirrel-overrun canary — fires on the attacker when
/// any blocker is declared against it).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum EventName {
    OnEnterBoard,
    OnDie,
    OnAttack,
    OnBlock,
    OnBlockedBy,
    OnPlay,
    /// Fires on a card the moment it gets attached as a HAND-payment cost
    /// to a played card. Handler receives `(game, self, partner)` where
    /// `partner` is the card being paid for. Powers the zebra / mantis-shrimp
    /// "if attached as cost to matching-color, may reveal & draw" cycle.
    OnAttachedAsCost,
    /// Fires on a creature after it successfully damaged a player via
    /// an unblocked attack (per B.2). Also fires on each of that
    /// creature's attached cards — Klotho-style mutations declare the
    /// handler and receive `self` = the mutation, drawing for
    /// `self.owner`. Closes the gap LIMITATIONS flagged as
    /// `OnDealtDamageToPlayer`.
    OnDealtDamageToPlayer,
    /// Fires at the start of the active player's Untap step (i.e., at
    /// the beginning of their turn). Broadcasts to every BOARD card of
    /// the active player plus every card attached to one of those
    /// boards' cards — mutations that declare the handler receive
    /// `self` = the mutation. "At the beginning of your turn..."
    OnTurnBegin,
    /// Watcher event: fires on every BOARD card whenever any creature
    /// dies (moved BOARD → GRAVEYARD). Handler signature is
    /// `(game, self, dying)` where `self` is the watcher and `dying`
    /// is the creature that just died. Distinct from `OnDie`, which
    /// fires self-only on the dying card itself. The dying card does
    /// NOT receive its own `OnCreatureDies` (it's no longer on BOARD
    /// by the time the broadcast fires). Used by Avatar of Greed and
    /// any other "whenever a creature dies, ..." trigger.
    OnCreatureDies,
}

impl EventName {
    /// The Lua field name used to declare this handler on a card table.
    pub fn lua_key(self) -> &'static str {
        match self {
            EventName::OnEnterBoard => "on_enter_board",
            EventName::OnDie => "on_die",
            EventName::OnAttack => "on_attack",
            EventName::OnBlock => "on_block",
            EventName::OnBlockedBy => "on_blocked_by",
            EventName::OnPlay => "on_play",
            EventName::OnAttachedAsCost => "on_attached_as_cost",
            EventName::OnDealtDamageToPlayer => "on_dealt_damage_to_player",
            EventName::OnTurnBegin => "on_turn_begin",
            EventName::OnCreatureDies => "on_creature_dies",
        }
    }

    /// All known event names, for loader iteration.
    pub const ALL: [EventName; 10] = [
        EventName::OnEnterBoard,
        EventName::OnDie,
        EventName::OnAttack,
        EventName::OnBlock,
        EventName::OnBlockedBy,
        EventName::OnPlay,
        EventName::OnAttachedAsCost,
        EventName::OnDealtDamageToPlayer,
        EventName::OnTurnBegin,
        EventName::OnCreatureDies,
    ];
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub colors: Vec<String>,
    pub kind: CardType,
    /// `Some(Timing::Instant)` for instants, `Some(Timing::Sorcery)` for
    /// main-phase-only spells, `None` for permanents (Creature, Artifact,
    /// Environment). Authors write `type = "instant"` or `type = "sorcery"`
    /// in Lua; the parser translates both to `kind = Spell` with the
    /// appropriate timing.
    #[serde(default)]
    pub timing: Option<Timing>,
    pub subtypes: Vec<String>,
    /// Subtypes this creature cannot block. Combat rejects a declared
    /// block when `attacker.subtypes ∩ blocker.cannot_block_subtypes`
    /// is non-empty (case-insensitive). Empty for most cards. Used by
    /// rats ("can't block cats.") and any future "<X> can't block <Y>"
    /// flavor pair.
    #[serde(default)]
    pub cannot_block_subtypes: Vec<String>,
    /// Subtypes this creature CAN block as an exception to flying.
    /// When `attacker.subtypes ∩ blocker.can_block_subtypes` is non-
    /// empty, the flying-blocker requirement is waived for that pair.
    /// Used by cats ("can block birds.") — the predator-prey override.
    /// Does NOT bypass `unblockable` or other non-flying restrictions.
    #[serde(default)]
    pub can_block_subtypes: Vec<String>,
    /// Per RULES.md C.1 / C.11 / P.7a: a card's symbols form a set
    /// participating in identity matching alongside colors. Empty Vec =
    /// no symbols (legal for any card except transparent-frame ones
    /// per C.13, which the engine doesn't enforce yet). Lua parser
    /// accepts either `symbol = "X"` (single-shorthand, wrapped into a
    /// one-element Vec) or `symbols = {"X", "Y"}` (explicit array).
    pub symbols: Vec<String>,
    /// Visual frame attribute, distinct from color identity. Currently
    /// only `Some("transparent")` is meaningful — a transparent card is
    /// see-through (the symbol-search routine looks past it to the next
    /// opaque card down). Frame is NOT a color: color-matching rules
    /// (P.12a graveyard payment, static affects.colors) ignore it.
    ///
    /// Whole-card legacy hack predating the per-slot `holes` field. New
    /// cards declare positional holes via `holes`; cards still using
    /// `frame = "transparent"` are treated as having a single hole at
    /// slot `C` until migrated.
    #[serde(default)]
    pub frame: Option<String>,
    /// Positional hole geometry — slots on the card that are see-through.
    /// See `SLOTS.md`. Empty = no holes (fully opaque card). A hole at
    /// slot S aligns with a symbol slot S on the card beneath when this
    /// card sits above it (the see-through reveal). Cards can declare
    /// any subset of the 15 slots. No engine site reads this yet — the
    /// per-slot reveal walk is the next slice; for now `holes` is data
    /// captured for forward compatibility.
    #[serde(default)]
    pub holes: Vec<Slot>,
    /// Positional symbol placement — `Slot → glyph`. Opt-in. When set,
    /// the card's symbols live at the named slots; when empty, the
    /// engine falls back to the `symbols` array, spiraling out from C
    /// per the SLOTS.md canonical fill order. A card may not declare
    /// a symbol at a slot that also appears in `holes` (loader-enforced
    /// once V.8 ships per-slot).
    #[serde(default)]
    pub symbol_slots: std::collections::BTreeMap<Slot, String>,
    /// Positional color placement — `Slot → color name`. Opt-in. When
    /// set, the card's color identity is also drawn on the back as a
    /// circle at the named slot(s); when empty, the card carries color
    /// identity (`colors`) but does not display it on the back. Each
    /// entry's value must already appear in `colors`. The loader
    /// populates this from slot-form `colors = { C = "green", T = "red" }`
    /// in Lua; list-form `colors = {"green"}` leaves this empty. No
    /// engine site reads this yet — captured for the per-slot reveal
    /// engine that supersedes the V.8/V.9 deck-top channel.
    #[serde(default)]
    pub color_slots: std::collections::BTreeMap<Slot, String>,
    /// Cosmetic surface treatments (e.g. `"shiny"`, `"holo"`). Pure
    /// metadata — no engine rule reads this. Stacks, so a card can be
    /// both shiny and holo. Distinct from `frame` (geometry / hole) and
    /// from `colors` (identity for matching rules).
    #[serde(default)]
    pub face: Vec<String>,
    pub cost: Vec<CostComponent>,
    pub abilities: Vec<String>,
    /// Flavor text. Non-mechanical. Optional. Displayed under abilities in
    /// the report tooltip and (eventually) UIs.
    #[serde(default)]
    pub flavor: String,
    pub stats: Option<Stats>,
    /// Phase 1 static ability declaration. `None` for most cards. When set,
    /// the engine evaluates `affects` against every on-board candidate at
    /// `effective_stats` read time and applies `modifier_x/y` to matches.
    /// Source must be on the BOARD for the effect to apply.
    #[serde(default)]
    pub static_def: Option<StaticDef>,
    /// Lua event handlers loaded from `on_*` fields. Empty for data-only cards.
    /// Handles are refcounted into the owning `CardRegistry`'s VM and must not
    /// outlive it. **Not serialized** — on load, the deserialized `Card` has
    /// an empty handler map; callers must rebind handlers from a live
    /// `CardRegistry` (see `replay::rebind_handlers`).
    #[serde(skip, default)]
    pub handlers: BTreeMap<EventName, Function>,
    /// When true, this card may be exiled from its controller's
    /// GRAVEYARD to fill one HAND-source slot of a spell they cast,
    /// per Clear View. The substituted slot bypasses the P.7a
    /// identity check on the substitute itself, but the cast's
    /// other HAND payments must still satisfy P.7a — substitution
    /// adds slot capacity, not identity coverage.
    #[serde(default)]
    pub gy_hand_substitute: bool,
    /// RULES P.30: variable-X cost components default to a minimum
    /// X of 1. A card may opt into accepting X = 0 by setting this
    /// to true — used by designs where X = 0 has a real strategic
    /// purpose (e.g., dark-salamander cast for max mill efficiency
    /// at the cost of a 0/0 body). Default false: the engine rejects
    /// `x_value = Some(0)` with `PlayError::XBelowMinimum`.
    #[serde(default)]
    pub allow_x_zero: bool,
    /// Activated abilities the controller may fire on their initiative.
    /// Resolves immediately (no stack, no response window per the design
    /// decision recorded in RULES A.5). Each entry has a cost, a text
    /// snippet for tooltips, a timing class, and the Lua effect handler.
    /// Like `handlers`, not serialized — the `Function` is bound to the
    /// owning `CardRegistry` VM and must be re-bound after deserialize.
    #[serde(skip, default)]
    pub activated: Vec<ActivatedAbility>,
    /// RULES P.32: declarative target category. When set, the engine
    /// refuses the cast if no legal target for the category exists. Pure
    /// state read — no Lua handler required. Common categories like
    /// "chain" (counterspells), "creature" (removal), "graveyard_card"
    /// (recursion) get one definition each; cards just declare which.
    #[serde(default)]
    pub target: Option<Target>,
    /// Balance-probe variant marker. True for cards loaded from a
    /// `variants = { [key] = { overrides } }` block in another card's
    /// .lua file. Variants are excluded from `main.rs::playable_pool`
    /// so they don't pollute `make evolve` / champions / gauntlets;
    /// `tsot balance-probe` is the only consumer that picks them up.
    #[serde(default)]
    pub is_variant: bool,
    /// If this card was loaded as a variant, the base card's id (the
    /// id of the .lua file's outer `id = ...`). Used by `tsot balance-
    /// probe` to expand `probe BASE_ID` into the base + all its
    /// variants without forcing the user to list them.
    #[serde(default)]
    pub variant_of: Option<String>,
}

/// One activated ability declared on a card. Cost has two parts:
/// `cost_tap` (source must be untapped; B.3 sickness applies to
/// creature sources; source becomes tapped on activate) and
/// `cost_components` (a list of `CostComponent`s in the same shape
/// play-card costs use: HAND, MILL, GRAVEYARD, SACRIFICE, SelfExile).
/// Either, both, or neither can be present. `cost_tap = false` with
/// empty components is a free activation; `cost_tap = true` with
/// `[Hand{amount=1}]` is `T, 1 hand: …`.
#[derive(Clone)]
pub struct ActivatedAbility {
    pub cost_tap: bool,
    pub cost_components: Vec<CostComponent>,
    pub text: String,
    pub timing: Timing,
    /// Optional pre-payment gate. Runs in the same Lua context as
    /// `effect` but is expected to be read-only. Returns truthy if the
    /// effect would do something useful (e.g., a legal target exists).
    /// If absent, no pre-check beyond cost affordability runs. When
    /// present and falsy/errors, the activation aborts with
    /// `ActivateError::NoLegalTarget` and **no cost is paid** — this
    /// is the whole point of the hook.
    pub validate: Option<Function>,
    /// RULES P.32 (extends to activations per A.9): declarative target
    /// category. When set, the engine refuses activation if no legal
    /// target exists. Pure state read. Cards can use this instead of (or
    /// alongside) a `validate` function for the common "needs a target
    /// of category X" pre-check.
    pub target: Option<Target>,
    pub effect: Function,
}

impl std::fmt::Debug for ActivatedAbility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActivatedAbility")
            .field("cost_tap", &self.cost_tap)
            .field("cost_components", &self.cost_components)
            .field("text", &self.text)
            .field("timing", &self.timing)
            .field("has_validate", &self.validate.is_some())
            .finish()
    }
}

impl std::fmt::Debug for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let handler_keys: Vec<&'static str> =
            self.handlers.keys().map(|e| e.lua_key()).collect();
        f.debug_struct("Card")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("colors", &self.colors)
            .field("kind", &self.kind)
            .field("timing", &self.timing)
            .field("subtypes", &self.subtypes)
            .field("symbols", &self.symbols)
            .field("cost", &self.cost)
            .field("abilities", &self.abilities)
            .field("stats", &self.stats)
            .field("static_def", &self.static_def)
            .field("handlers", &handler_keys)
            .field("activated", &self.activated)
            .finish()
    }
}
