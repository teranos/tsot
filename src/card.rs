use include_dir::{include_dir, Dir};
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

static EMBEDDED_CARDS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/cards");

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
}

/// When a spell can be cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Timing {
    /// Castable at any time, including inside response windows.
    Instant,
    /// Main phase only. Cannot be cast inside a response window.
    Sorcery,
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

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub x: i32,
    pub y: i32,
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
    pub affects: StaticAffects,
    /// Phase 1.5: stat modifier values are no longer fixed integers — each
    /// is a `ModifierValue` that resolves to an `i32` against the source's
    /// current state at every read. Lets cards scale with their attached
    /// set (hydra: +1/+1 per attached; reef-phantom: +1/+1 per attached blue)
    /// without the snapshot leak that imperative `add_modifier` had.
    pub modifier_x: ModifierValue,
    pub modifier_y: ModifierValue,
    /// Phase 2: keyword granted to matching candidates. None = no keyword
    /// grant. Lowercase string matching `has_keyword` lookup. Examples:
    /// "flying", "vigilance", "haste", "cannot-block".
    #[serde(default)]
    pub modifier_keyword: Option<String>,
    /// Phase 2: state-reading gate. None = always active when the source
    /// is on board. Some(cond) = the static only fires when the engine's
    /// evaluation of `cond` against game state is true.
    #[serde(default)]
    pub condition: Option<StaticCondition>,
    /// Phase 3: restrictions imposed on matching candidates. Each restriction
    /// is consulted by the engine at the corresponding choke point
    /// (declare_attacker, resolve_hand_payment, etc.). Empty = no
    /// restrictions. One static can carry multiple (flesh-eating-plant:
    /// `cannot_attack` AND `cannot_be_cost_paid`).
    #[serde(default)]
    pub restrictions: Vec<Restriction>,
    /// Phase 3.5: cost reductions applied to matching candidates when they
    /// are cast. The `affects` predicate gates which cards get the discount;
    /// each entry reduces one cost-source by `amount` (clamped to 0 per P.20).
    /// Modern LCD Clock uses this with `affects.kind = creature` and one
    /// entry each for HAND and GRAVEYARD reductions.
    #[serde(default)]
    pub cost_modifiers: Vec<CostModifier>,
    /// Static-granted activated ability. Matching cards (per `affects`)
    /// gain this ability in addition to any printed activations they
    /// already have. Used by the jewel cycle: the jewel's static
    /// (scope = attached_host) grants `T: draw a card, then discard a
    /// card` to its host creature. Not serialized — the Lua handler
    /// references inside are rebound from the live CardRegistry per
    /// the same convention as `Card.handlers` / `Card.activated`.
    #[serde(skip, default)]
    pub granted_activated: Option<ActivatedAbility>,
    /// Colors granted to matching candidates. Empty Vec = no color
    /// grant. Used by fluorescent-protein mutations (GFP grants green +
    /// glow to its host). `GameState::effective_colors(iid)` unions the
    /// candidate's printed colors with every grant from active statics
    /// whose `affects` predicate matches. Identity matching (P.7a) and
    /// jewel pitch validation (P.24) consult effective colors; the
    /// static-affects matcher itself uses printed colors only, to
    /// avoid recursion (same pattern as the keyword-grant cycle guard).
    #[serde(default)]
    pub granted_colors: Vec<String>,
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

/// Phase 1.5 dynamic stat-modifier value. Resolved to an `i32` against the
/// source CardInstance's current state every time `effective_stats` runs,
/// so the value automatically tracks attached-set changes.
///
/// Lua parser accepts either a bare integer (`x = 2` → `Fixed(2)`) or a
/// short string descriptor. `"attached"` maps to `AttachedCount`;
/// `"attached:blue"` maps to `AttachedCountByColor("blue")`;
/// `"attached:type:mutation"` maps to `AttachedCountByKind(Mutation)`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModifierValue {
    Fixed(i32),
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
    /// Count of cards in both players' HAND zones. Parsed from `"hands"`.
    HandCount,
}

impl Default for ModifierValue {
    fn default() -> Self {
        ModifierValue::Fixed(0)
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
        }
    }

    /// All known event names, for loader iteration.
    pub const ALL: [EventName; 7] = [
        EventName::OnEnterBoard,
        EventName::OnDie,
        EventName::OnAttack,
        EventName::OnBlock,
        EventName::OnBlockedBy,
        EventName::OnPlay,
        EventName::OnAttachedAsCost,
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
    /// no symbols (legal for any card except `transparent`-colored ones
    /// per C.13, which the engine doesn't enforce yet). Lua parser
    /// accepts either `symbol = "X"` (single-shorthand, wrapped into a
    /// one-element Vec) or `symbols = {"X", "Y"}` (explicit array).
    pub symbols: Vec<String>,
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

fn normalize_color(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// Lua-side type strings translate to `(kind, timing)`. "instant" and
/// "sorcery" are both Spell kind with different timing; "spell" (legacy
/// alias) is treated as sorcery timing.
fn parse_type(s: &str) -> Result<(CardType, Option<Timing>), String> {
    match s.to_ascii_lowercase().as_str() {
        "" => Ok((CardType::Unspecified, None)),
        "creature" => Ok((CardType::Creature, None)),
        "instant" => Ok((CardType::Spell, Some(Timing::Instant))),
        "sorcery" | "spell" => Ok((CardType::Spell, Some(Timing::Sorcery))),
        "artifact" => Ok((CardType::Artifact, None)),
        "environment" => Ok((CardType::Environment, None)),
        "mutation" => Ok((CardType::Mutation, None)),
        other => Err(format!("unknown type: {other}")),
    }
}

fn parse_source(s: &str) -> Result<CostSource, String> {
    match s.to_ascii_lowercase().as_str() {
        "hand" => Ok(CostSource::Hand),
        "mill" => Ok(CostSource::Mill),
        "graveyard" => Ok(CostSource::Graveyard),
        "sacrifice" => Ok(CostSource::Sacrifice),
        "self" => Ok(CostSource::SelfExile),
        "attached" => Ok(CostSource::Attached),
        other => Err(format!("unknown cost source: {other}")),
    }
}

fn read_string_vec(t: &Table, key: &str) -> mlua::Result<Vec<String>> {
    match t.get::<Value>(key)? {
        Value::Nil => Ok(Vec::new()),
        Value::Table(tt) => tt.sequence_values::<String>().collect(),
        other => Err(mlua::Error::runtime(format!(
            "field {key:?} must be a list of strings, got {other:?}"
        ))),
    }
}

fn read_color_vec(t: &Table) -> mlua::Result<Vec<String>> {
    Ok(read_string_vec(t, "colors")?
        .into_iter()
        .map(|s| normalize_color(&s))
        .collect())
}

fn read_cost(t: &Table) -> mlua::Result<Vec<CostComponent>> {
    let raw: Table = match t.get::<Value>("cost")? {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(tt) => tt,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `cost` must be a list, got {other:?}"
            )))
        }
    };
    let mut out = Vec::new();
    for item in raw.sequence_values::<Table>() {
        let item = item?;
        let amount = item.get::<Option<i32>>("amount")?.unwrap_or(0);
        let is_x = item.get::<Option<bool>>("is_x")?.unwrap_or(false);
        let source_s = item.get::<String>("source")?;
        let source = parse_source(&source_s).map_err(mlua::Error::runtime)?;
        let kind = match item.get::<Option<String>>("kind")? {
            None => None,
            Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
        };
        out.push(CostComponent {
            amount,
            source,
            is_x,
            kind,
        });
    }
    Ok(out)
}

fn read_activated(t: &Table) -> mlua::Result<Vec<ActivatedAbility>> {
    let raw: Table = match t.get::<Value>("activated")? {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(tt) => tt,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `activated` must be a list, got {other:?}"
            )))
        }
    };
    let mut out = Vec::new();
    for item in raw.sequence_values::<Table>() {
        let item = item?;
        // Two shapes supported for `cost`:
        //   1. String shorthand: `cost = "tap"` → tap-only.
        //   2. List of components: `cost = {{source = "...", amount = N}}` →
        //      one or more cost components, possibly including a tap
        //      pseudo-component `{source = "tap"}` (no amount).
        let cost_value: Value = item.get("cost")?;
        let (cost_tap, cost_components) = match cost_value {
            Value::String(s) => {
                let s = s.to_str()?.to_ascii_lowercase();
                if s == "tap" || s == "t" {
                    (true, Vec::new())
                } else {
                    return Err(mlua::Error::runtime(format!(
                        "activation cost string {s:?} not recognized (expected \"tap\")"
                    )));
                }
            }
            Value::Table(tt) => {
                let mut tap = false;
                let mut comps: Vec<CostComponent> = Vec::new();
                for comp in tt.sequence_values::<Table>() {
                    let comp = comp?;
                    let src_s: String = comp.get("source")?;
                    let lowered = src_s.to_ascii_lowercase();
                    if lowered == "tap" || lowered == "t" {
                        tap = true;
                        continue;
                    }
                    let amount = comp.get::<Option<i32>>("amount")?.unwrap_or(0);
                    let is_x = comp.get::<Option<bool>>("is_x")?.unwrap_or(false);
                    let source = parse_source(&lowered).map_err(mlua::Error::runtime)?;
                    let kind = match comp.get::<Option<String>>("kind")? {
                        None => None,
                        Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
                    };
                    comps.push(CostComponent {
                        amount,
                        source,
                        is_x,
                        kind,
                    });
                }
                (tap, comps)
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "activation cost must be a string or a list, got {other:?}"
                )))
            }
        };
        let text = item.get::<Option<String>>("text")?.unwrap_or_default();
        let timing_s = item
            .get::<Option<String>>("timing")?
            .unwrap_or_else(|| "sorcery".to_string());
        let timing = match timing_s.to_ascii_lowercase().as_str() {
            "instant" => Timing::Instant,
            "sorcery" => Timing::Sorcery,
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown activation timing: {other:?} (must be \"instant\" or \"sorcery\")"
                )))
            }
        };
        let validate: Option<Function> = match item.get::<Value>("validate")? {
            Value::Nil => None,
            Value::Function(f) => Some(f),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "activation `validate` must be a function, got {other:?}"
                )))
            }
        };
        let effect: Function = item.get("effect")?;
        let target: Option<Target> = match item.get::<Option<String>>("target")? {
            None => None,
            Some(s) => match s.to_ascii_lowercase().as_str() {
                "chain" => Some(Target::Chain),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "unknown activation target category: {other:?}"
                    )))
                }
            },
        };
        out.push(ActivatedAbility {
            cost_tap,
            cost_components,
            text,
            timing,
            validate,
            target,
            effect,
        });
    }
    Ok(out)
}

fn read_handlers(t: &Table) -> mlua::Result<BTreeMap<EventName, Function>> {
    let mut out = BTreeMap::new();
    for ev in EventName::ALL {
        match t.get::<Value>(ev.lua_key())? {
            Value::Nil => {}
            Value::Function(f) => {
                out.insert(ev, f);
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "field `{}` must be a function, got {other:?}",
                    ev.lua_key()
                )))
            }
        }
    }
    Ok(out)
}

fn read_stats(t: &Table) -> mlua::Result<Option<Stats>> {
    match t.get::<Value>("stats")? {
        Value::Nil => Ok(None),
        Value::Table(s) => {
            let x = s.get::<Option<i32>>("x")?.unwrap_or(0);
            let y = s.get::<Option<i32>>("y")?.unwrap_or(0);
            Ok(Some(Stats { x, y }))
        }
        other => Err(mlua::Error::runtime(format!(
            "field `stats` must be a table, got {other:?}"
        ))),
    }
}

fn read_static(t: &Table) -> mlua::Result<Option<StaticDef>> {
    let static_val = t.get::<Value>("static")?;
    let static_t = match static_val {
        Value::Nil => return Ok(None),
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "field `static` must be a table, got {other:?}"
            )))
        }
    };
    let affects = match static_t.get::<Value>("affects")? {
        Value::Nil => StaticAffects::default(),
        Value::Table(a) => {
            let subtypes = match a.get::<Value>("subtypes")? {
                Value::Nil => Vec::new(),
                Value::Table(st) => st
                    .sequence_values::<String>()
                    .collect::<mlua::Result<Vec<_>>>()?
                    .into_iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "static.affects.subtypes must be a table, got {other:?}"
                    )))
                }
            };
            let colors = match a.get::<Value>("colors")? {
                Value::Nil => Vec::new(),
                Value::Table(ct) => ct
                    .sequence_values::<String>()
                    .collect::<mlua::Result<Vec<_>>>()?
                    .into_iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "static.affects.colors must be a table, got {other:?}"
                    )))
                }
            };
            let controller = match a.get::<Option<String>>("controller")? {
                None => None,
                Some(s) => match s.to_ascii_lowercase().as_str() {
                    "owner" => Some(StaticController::Owner),
                    "opponent" => Some(StaticController::Opponent),
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.affects.controller must be 'owner' or 'opponent', got '{other}'"
                        )))
                    }
                },
            };
            let exclude_self = a.get::<Option<bool>>("exclude_self")?.unwrap_or(false);
            let scope = match a.get::<Option<String>>("scope")? {
                None => StaticScope::Board,
                Some(s) => match s.to_ascii_lowercase().as_str() {
                    "board" => StaticScope::Board,
                    "attached_host" => StaticScope::AttachedHost,
                    "source_only" => StaticScope::SourceOnly,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.affects.scope must be 'board', 'attached_host', or 'source_only', got '{other}'"
                        )))
                    }
                },
            };
            let kind = match a.get::<Option<String>>("kind")? {
                None => None,
                Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
            };
            let has_keyword = a
                .get::<Option<String>>("has_keyword")?
                .map(|s| s.to_ascii_lowercase());
            StaticAffects {
                subtypes,
                colors,
                controller,
                exclude_self,
                scope,
                kind,
                has_keyword,
            }
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.affects must be a table, got {other:?}"
            )))
        }
    };
    let (modifier_x, modifier_y, modifier_keyword, granted_colors) =
        match static_t.get::<Value>("modifier")? {
            Value::Nil => (
                ModifierValue::Fixed(0),
                ModifierValue::Fixed(0),
                None,
                Vec::new(),
            ),
            Value::Table(m) => {
                let x = read_modifier_value(m.get::<Value>("x")?)?;
                let y = read_modifier_value(m.get::<Value>("y")?)?;
                let keyword = m
                    .get::<Option<String>>("keyword")?
                    .map(|s| s.to_ascii_lowercase());
                let colors: Vec<String> = match m.get::<Option<Value>>("colors")? {
                    Some(Value::Table(t)) => {
                        let mut out = Vec::new();
                        for s in t.sequence_values::<String>() {
                            out.push(s?.to_ascii_lowercase());
                        }
                        out
                    }
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "static.modifier.colors must be a sequence of strings, got {other:?}"
                        )))
                    }
                    None => Vec::new(),
                };
                (x, y, keyword, colors)
            }
            other => {
                return Err(mlua::Error::runtime(format!(
                    "static.modifier must be a table, got {other:?}"
                )))
            }
        };
    let condition = match static_t.get::<Value>("condition")? {
        Value::Nil => None,
        Value::Table(c) => Some(read_condition(&c)?),
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.condition must be a table, got {other:?}"
            )))
        }
    };
    let restrictions = match static_t.get::<Value>("restrictions")? {
        Value::Nil => Vec::new(),
        Value::Table(r) => {
            let mut out = Vec::new();
            for s in r.sequence_values::<String>() {
                let s = s?;
                let restriction = match s.to_ascii_lowercase().as_str() {
                    "cannot_attack" => Restriction::CannotAttack,
                    "cannot_be_cost_paid" => Restriction::CannotBeCostPaid,
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "static.restrictions entry must be 'cannot_attack' or 'cannot_be_cost_paid', got '{other}'"
                        )))
                    }
                };
                out.push(restriction);
            }
            out
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.restrictions must be a sequence of strings, got {other:?}"
            )))
        }
    };
    let cost_modifiers = match static_t.get::<Value>("cost_modifiers")? {
        Value::Nil => Vec::new(),
        Value::Table(t) => {
            let mut out = Vec::new();
            for item in t.sequence_values::<Table>() {
                let item = item?;
                let source_s: String = item.get("source")?;
                let source = parse_source(&source_s).map_err(mlua::Error::runtime)?;
                let amount = item.get::<Option<i32>>("amount")?.unwrap_or(1);
                out.push(CostModifier { source, amount });
            }
            out
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.cost_modifiers must be a sequence of tables, got {other:?}"
            )))
        }
    };
    // Phase 3: optional `granted_activated` field declares a single
    // activated ability that matching candidates gain. Same Lua shape
    // as a card-level `activated[1]` entry: { cost, text, timing,
    // effect, optional validate }.
    let granted_activated = match static_t.get::<Value>("granted_activated")? {
        Value::Nil => None,
        Value::Table(t) => Some(parse_one_activated_entry(t)?),
        other => {
            return Err(mlua::Error::runtime(format!(
                "static.granted_activated must be a table, got {other:?}"
            )))
        }
    };
    Ok(Some(StaticDef {
        affects,
        modifier_x,
        modifier_y,
        modifier_keyword,
        condition,
        restrictions,
        cost_modifiers,
        granted_activated,
        granted_colors,
    }))
}

fn parse_one_activated_entry(item: Table) -> mlua::Result<ActivatedAbility> {
    let cost_value: Value = item.get("cost")?;
    let (cost_tap, cost_components) = match cost_value {
        Value::String(s) => {
            let s = s.to_str()?.to_ascii_lowercase();
            if s == "tap" || s == "t" {
                (true, Vec::new())
            } else {
                return Err(mlua::Error::runtime(format!(
                    "granted_activated cost string {s:?} not recognized (expected \"tap\")"
                )));
            }
        }
        Value::Table(tt) => {
            let mut tap = false;
            let mut comps: Vec<CostComponent> = Vec::new();
            for comp in tt.sequence_values::<Table>() {
                let comp = comp?;
                let src_s: String = comp.get("source")?;
                let lowered = src_s.to_ascii_lowercase();
                if lowered == "tap" || lowered == "t" {
                    tap = true;
                    continue;
                }
                let amount = comp.get::<Option<i32>>("amount")?.unwrap_or(0);
                let is_x = comp.get::<Option<bool>>("is_x")?.unwrap_or(false);
                let source = parse_source(&lowered).map_err(mlua::Error::runtime)?;
                let kind = match comp.get::<Option<String>>("kind")? {
                    None => None,
                    Some(k) => Some(parse_type(&k).map_err(mlua::Error::runtime)?.0),
                };
                comps.push(CostComponent {
                    amount,
                    source,
                    is_x,
                    kind,
                });
            }
            (tap, comps)
        }
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated cost must be a string or list, got {other:?}"
            )))
        }
    };
    let text = item.get::<Option<String>>("text")?.unwrap_or_default();
    let timing_s = item
        .get::<Option<String>>("timing")?
        .unwrap_or_else(|| "sorcery".to_string());
    let timing = match timing_s.to_ascii_lowercase().as_str() {
        "instant" => Timing::Instant,
        "sorcery" => Timing::Sorcery,
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated timing {other:?} must be \"instant\" or \"sorcery\""
            )))
        }
    };
    let validate: Option<Function> = match item.get::<Value>("validate")? {
        Value::Nil => None,
        Value::Function(f) => Some(f),
        other => {
            return Err(mlua::Error::runtime(format!(
                "granted_activated validate must be a function, got {other:?}"
            )))
        }
    };
    let effect: Function = item.get("effect")?;
    let target: Option<Target> = match item.get::<Option<String>>("target")? {
        None => None,
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "chain" => Some(Target::Chain),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown granted_activated target category: {other:?}"
                )))
            }
        },
    };
    Ok(ActivatedAbility {
        cost_tap,
        cost_components,
        text,
        timing,
        validate,
        target,
        effect,
    })
}

/// Parse a `ModifierValue` from a Lua value. Accepts either:
/// - Nil → `Fixed(0)` (back-compat for omitted entries)
/// - Integer N → `Fixed(N)`
/// - String "attached" → `AttachedCount`
/// - String "N*attached" (e.g., "2*attached") → `AttachedCountScaled(N)`
/// - String "attached:type:<kind>" → `AttachedCountByKind(kind)`
/// - String "attached:<color>" → `AttachedCountByColor(color)` (fallback)
fn read_modifier_value(v: Value) -> mlua::Result<ModifierValue> {
    match v {
        Value::Nil => Ok(ModifierValue::Fixed(0)),
        Value::Integer(n) => Ok(ModifierValue::Fixed(n as i32)),
        Value::Number(n) => Ok(ModifierValue::Fixed(n as i32)),
        Value::String(s) => {
            let raw = s.to_str()?.to_string();
            let lower = raw.to_ascii_lowercase().replace(' ', "");
            if lower == "attached" {
                return Ok(ModifierValue::AttachedCount);
            }
            if lower == "board" {
                return Ok(ModifierValue::BoardCount);
            }
            if lower == "hands" || lower == "hand" {
                return Ok(ModifierValue::HandCount);
            }
            // `N*attached` form (e.g., "2*attached", "3*attached").
            if let Some((mul_str, tail)) = lower.split_once('*') {
                if tail == "attached" {
                    let n: i32 = mul_str.parse().map_err(|_| {
                        mlua::Error::runtime(format!(
                            "modifier value 'N*attached' multiplier must be an integer, got {mul_str:?}"
                        ))
                    })?;
                    return Ok(ModifierValue::AttachedCountScaled(n));
                }
            }
            if let Some(kind_str) = lower.strip_prefix("attached:type:") {
                let (kind, _) = parse_type(kind_str).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "modifier value 'attached:type:<kind>' has unknown kind: {e}"
                    ))
                })?;
                return Ok(ModifierValue::AttachedCountByKind(kind));
            }
            if let Some(rest) = lower.strip_prefix("attached:") {
                return Ok(ModifierValue::AttachedCountByColor(rest.to_string()));
            }
            Err(mlua::Error::runtime(format!(
                "modifier value string must be 'attached', 'N*attached', 'attached:<color>', or 'attached:type:<kind>', got {raw:?}"
            )))
        }
        other => Err(mlua::Error::runtime(format!(
            "modifier value must be integer or string, got {other:?}"
        ))),
    }
}

fn read_condition(c: &Table) -> mlua::Result<StaticCondition> {
    let kind: String = c.get("kind")?;
    match kind.to_ascii_lowercase().as_str() {
        "owner_graveyard_size" => {
            let min = c.get::<i64>("min")?.max(0) as usize;
            Ok(StaticCondition::OwnerGraveyardSize { min })
        }
        "owner_graveyard_non_creatures" => {
            let min = c.get::<i64>("min")?.max(0) as usize;
            Ok(StaticCondition::OwnerGraveyardNonCreatures { min })
        }
        other => Err(mlua::Error::runtime(format!(
            "static.condition.kind must be 'owner_graveyard_size' or 'owner_graveyard_non_creatures', got '{other}'"
        ))),
    }
}

/// Parse a single Lua table into a `Card`. Handles every field except
/// `variants` (which lives at the file level — see `load_card`). Reused
/// by both the base-card path and the per-variant merged-table path.
fn parse_card_table(table: &Table) -> mlua::Result<Card> {
    let id: String = table.get("id")?;
    let name = table.get::<Option<String>>("name")?.unwrap_or_default();
    let symbols: Vec<String> = match table.get::<Option<Value>>("symbols")? {
        Some(Value::Table(t)) => {
            let mut out = Vec::new();
            for pair in t.pairs::<i64, String>() {
                let (_, s) = pair?;
                out.push(s);
            }
            out
        }
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "card.symbols must be a sequence of strings, got {other:?}"
            )))
        }
        None => match table.get::<Option<String>>("symbol")? {
            Some(s) if !s.is_empty() => vec![s],
            _ => Vec::new(),
        },
    };
    let kind_s = table.get::<Option<String>>("type")?.unwrap_or_default();
    let (kind, timing) = parse_type(&kind_s).map_err(mlua::Error::runtime)?;
    let subtypes = read_string_vec(table, "subtypes")?;
    let cannot_block_subtypes = read_string_vec(table, "cannot_block_subtypes")?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let can_block_subtypes = read_string_vec(table, "can_block_subtypes")?
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let abilities = read_string_vec(table, "abilities")?;
    let flavor = table.get::<Option<String>>("flavor")?.unwrap_or_default();
    let colors = read_color_vec(table)?;
    let cost = read_cost(table)?;
    let stats = read_stats(table)?;
    let static_def = read_static(table)?;
    let handlers = read_handlers(table)?;
    let activated = read_activated(table)?;
    let gy_hand_substitute = table
        .get::<Option<bool>>("gy_hand_substitute")?
        .unwrap_or(false);
    let allow_x_zero = table
        .get::<Option<bool>>("allow_x_zero")?
        .unwrap_or(false);
    let target = match table.get::<Option<String>>("target")? {
        None => None,
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "chain" => Some(Target::Chain),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "unknown target category: {other:?}"
                )));
            }
        },
    };
    Ok(Card {
        id,
        name,
        colors,
        kind,
        timing,
        subtypes,
        cannot_block_subtypes,
        can_block_subtypes,
        symbols,
        cost,
        abilities,
        flavor,
        stats,
        static_def,
        handlers,
        gy_hand_substitute,
        allow_x_zero,
        activated,
        target,
        is_variant: false,
        variant_of: None,
    })
}

/// Load a card .lua file. Returns the base card followed by any
/// variant cards declared in the file's `variants = { [key] = { ... }
/// }` table. Each variant id is `{base_id}-{key}`. The variant table
/// REPLACES top-level fields wholesale (no deep merge) — to tweak a
/// single ability, copy the whole `activated` array into the variant
/// with the tweak. Variants get `is_variant = true` and
/// `variant_of = Some(base_id)` so `main.rs::playable_pool` can
/// exclude them and `tsot balance-probe` can pick them up.
pub fn load_card(lua: &Lua, path: &Path) -> mlua::Result<Vec<Card>> {
    let source = fs::read_to_string(path).map_err(mlua::Error::external)?;
    let chunk_name = path.display().to_string();
    load_card_from_source(lua, &source, &chunk_name)
}

fn load_card_from_source(lua: &Lua, source: &str, chunk_name: &str) -> mlua::Result<Vec<Card>> {
    let value: Value = lua.load(source).set_name(chunk_name.to_string()).eval()?;
    let table = match value {
        Value::Table(t) => t,
        other => {
            return Err(mlua::Error::runtime(format!(
                "card file must return a table, got {other:?}"
            )))
        }
    };

    let base = parse_card_table(&table)?;
    let base_id = base.id.clone();

    let variants_table: Option<Table> = table.get("variants")?;
    let mut out: Vec<Card> = vec![base];
    if let Some(vt) = variants_table {
        // Snapshot the base table's keys ONCE so we can replay them
        // into a merged table per variant. We skip `variants` itself
        // to avoid recursion.
        let mut base_pairs: Vec<(Value, Value)> = Vec::new();
        for pair in table.pairs::<Value, Value>() {
            let (k, v) = pair?;
            if let Value::String(ks) = &k {
                if ks.to_str()? == "variants" {
                    continue;
                }
            }
            base_pairs.push((k, v));
        }
        for pair in vt.pairs::<String, Table>() {
            let (key, override_table) = pair?;
            // Build a merged Lua table: base keys, then variant
            // overrides on top. Top-level fields are replaced
            // wholesale; nested fields are not deep-merged.
            let merged = lua.create_table()?;
            for (k, v) in &base_pairs {
                merged.set(k.clone(), v.clone())?;
            }
            for p in override_table.pairs::<Value, Value>() {
                let (k, v) = p?;
                merged.set(k, v)?;
            }
            // Force the variant id; the base's `id` field carried
            // through the base_pairs copy is the wrong one to keep.
            let variant_id = format!("{base_id}-{key}");
            merged.set("id", variant_id.clone())?;
            let mut variant = parse_card_table(&merged)?;
            variant.is_variant = true;
            variant.variant_of = Some(base_id.clone());
            out.push(variant);
        }
    }
    Ok(out)
}

pub fn load_cards_dir(lua: &Lua, dir: &Path) -> mlua::Result<Vec<Card>> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(mlua::Error::external)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    entries.sort();
    let mut all: Vec<Card> = Vec::new();
    for p in &entries {
        all.extend(load_card(lua, p)?);
    }
    Ok(all)
}

pub fn load_cards_embedded(lua: &Lua) -> mlua::Result<Vec<Card>> {
    let mut files: Vec<_> = EMBEDDED_CARDS
        .files()
        .filter(|f| f.path().extension().and_then(|s| s.to_str()) == Some("lua"))
        .collect();
    files.sort_by_key(|f| f.path().to_path_buf());
    let mut all: Vec<Card> = Vec::new();
    for f in &files {
        let source = f
            .contents_utf8()
            .ok_or_else(|| mlua::Error::runtime(format!("non-utf8 card: {}", f.path().display())))?;
        let chunk_name = f.path().display().to_string();
        all.extend(load_card_from_source(lua, source, &chunk_name)?);
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handlers_from(lua: &Lua, src: &str) -> BTreeMap<EventName, Function> {
        let value: Value = lua.load(src).eval().unwrap();
        let table = match value {
            Value::Table(t) => t,
            _ => panic!("expected table"),
        };
        read_handlers(&table).unwrap()
    }

    #[test]
    fn handler_field_captures_lua_function() {
        let lua = Lua::new();
        let handlers = handlers_from(
            &lua,
            r#"
            return {
                id = "fixture",
                on_blocked_by = function(game, self, blocker)
                    return "ran"
                end,
            }
        "#,
        );
        let handler = handlers.get(&EventName::OnBlockedBy).unwrap();
        let result: String = handler.call((Value::Nil, Value::Nil, Value::Nil)).unwrap();
        assert_eq!(result, "ran");
    }

    #[test]
    fn missing_handler_keys_are_absent() {
        let lua = Lua::new();
        let handlers = handlers_from(&lua, r#"return { id = "fixture" }"#);
        assert!(handlers.is_empty());
    }

    #[test]
    fn non_function_handler_value_errors() {
        let lua = Lua::new();
        let value: Value = lua
            .load(r#"return { id = "x", on_die = 5 }"#)
            .eval()
            .unwrap();
        let table = match value {
            Value::Table(t) => t,
            _ => panic!(),
        };
        assert!(read_handlers(&table).is_err());
    }

    #[test]
    fn registry_keeps_handlers_callable() {
        // The whole reason CardRegistry owns the Lua: handlers stay valid
        // as long as the registry lives.
        let tmp = std::env::temp_dir().join("tsot_card_handlers_test");
        std::fs::create_dir_all(&tmp).unwrap();
        let card_path = tmp.join("test-handler.lua");
        std::fs::write(
            &card_path,
            r#"return {
                id = "test-handler",
                on_die = function(game, self) return "fired" end,
            }"#,
        )
        .unwrap();

        let registry = CardRegistry::load(&tmp).unwrap();
        let card = registry
            .cards()
            .iter()
            .find(|c| c.id == "test-handler")
            .unwrap();
        let handler = card.handlers.get(&EventName::OnDie).unwrap();
        let result: String = handler.call((Value::Nil, Value::Nil)).unwrap();
        assert_eq!(result, "fired");

        std::fs::remove_file(&card_path).ok();
    }

    fn load_card_from_lua(src: &str) -> Card {
        // Unique temp-dir name per call without going through rand::random
        // (which is disallowed project-wide for determinism reasons — see
        // clippy.toml). A monotonic counter per process is enough for
        // test uniqueness; tests don't need randomness here.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_card_test_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("under-test.lua");
        std::fs::write(&path, src).unwrap();
        let registry = CardRegistry::load(&tmp).unwrap();
        let card = registry
            .cards()
            .iter()
            .find(|c| c.id == "under-test")
            .expect("card loaded")
            .clone();
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&tmp).ok();
        card
    }

    #[test]
    fn symbol_shorthand_parses_to_one_element_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test", symbol = "꩜" }"#);
        assert_eq!(card.symbols, vec!["꩜".to_string()]);
    }

    #[test]
    fn symbols_array_parses_in_order() {
        let card = load_card_from_lua(
            r#"return { id = "under-test", symbols = {"꩜", "⨳", "⋈"} }"#,
        );
        assert_eq!(
            card.symbols,
            vec!["꩜".to_string(), "⨳".to_string(), "⋈".to_string()]
        );
    }

    #[test]
    fn no_symbol_fields_yields_empty_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test" }"#);
        assert!(card.symbols.is_empty());
    }

    #[test]
    fn symbols_array_takes_priority_when_both_fields_present() {
        let card = load_card_from_lua(
            r#"return { id = "under-test", symbol = "X", symbols = {"꩜", "⨳"} }"#,
        );
        assert_eq!(card.symbols, vec!["꩜".to_string(), "⨳".to_string()]);
    }

    #[test]
    fn empty_symbol_shorthand_yields_empty_symbols_vec() {
        let card = load_card_from_lua(r#"return { id = "under-test", symbol = "" }"#);
        assert!(card.symbols.is_empty());
    }

    /// Load a directory of cards instead of looking one up. Used by
    /// the `variants` tests below which need to see BOTH the base and
    /// the synthesized variants in the registry.
    fn load_dir_cards(src: &str) -> Vec<Card> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!(
            "tsot_variants_test_{}_{}",
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("under-test.lua");
        std::fs::write(&path, src).unwrap();
        let registry = CardRegistry::load(&tmp).unwrap();
        let cards: Vec<Card> = registry.cards().to_vec();
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&tmp).ok();
        cards
    }

    #[test]
    fn card_without_variants_loads_as_single_card() {
        let cards = load_dir_cards(
            r#"return { id = "under-test", type = "creature", stats = {x = 1, y = 1} }"#,
        );
        assert_eq!(cards.len(), 1, "base only — no variants table present");
        assert_eq!(cards[0].id, "under-test");
        assert!(!cards[0].is_variant);
        assert!(cards[0].variant_of.is_none());
    }

    #[test]
    fn variants_emit_one_card_per_entry_with_suffixed_ids() {
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                type = "creature",
                stats = {x = 1, y = 1},
                variants = {
                    ["small"] = { stats = {x = 1, y = 2} },
                    ["big"]   = { stats = {x = 4, y = 4} },
                },
            }"#,
        );
        // Order in `cards` is implementation-defined (Lua pairs() over
        // string keys). Check by id rather than index.
        let by_id: std::collections::BTreeMap<&str, &Card> =
            cards.iter().map(|c| (c.id.as_str(), c)).collect();
        assert!(by_id.contains_key("under-test"), "base id present");
        assert!(by_id.contains_key("under-test-small"), "variant id present");
        assert!(by_id.contains_key("under-test-big"), "variant id present");
        assert_eq!(cards.len(), 3);
        let base = by_id["under-test"];
        assert!(!base.is_variant, "base is_variant = false");
        let small = by_id["under-test-small"];
        assert!(small.is_variant, "variant is_variant = true");
        assert_eq!(small.variant_of.as_deref(), Some("under-test"));
        assert_eq!(small.stats.unwrap().y, 2, "variant stats override applied");
        let big = by_id["under-test-big"];
        assert_eq!(big.stats.unwrap().x, 4);
        assert_eq!(big.stats.unwrap().y, 4);
    }

    #[test]
    fn modifier_value_scaled_attached_parses() {
        // `"2*attached"` → AttachedCountScaled(2) via the static block.
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                type = "creature",
                stats = {x = 0, y = 0},
                static = {
                    affects = { scope = "source_only" },
                    modifier = {x = "2*attached", y = "3*attached"},
                },
            }"#,
        );
        let s = cards[0].static_def.as_ref().expect("static set");
        assert_eq!(s.modifier_x, super::ModifierValue::AttachedCountScaled(2));
        assert_eq!(s.modifier_y, super::ModifierValue::AttachedCountScaled(3));
    }

    #[test]
    fn variant_keys_not_declared_inherit_from_base() {
        let cards = load_dir_cards(
            r#"return {
                id = "under-test",
                name = "Base Name",
                type = "creature",
                colors = {"green"},
                stats = {x = 2, y = 2},
                variants = {
                    ["v1"] = { stats = {x = 5, y = 5} },  -- only stats overridden
                },
            }"#,
        );
        let by_id: std::collections::BTreeMap<&str, &Card> =
            cards.iter().map(|c| (c.id.as_str(), c)).collect();
        let v1 = by_id["under-test-v1"];
        // Inherited fields:
        assert_eq!(v1.name, "Base Name", "name inherited");
        assert_eq!(v1.colors, vec!["green"], "colors inherited");
        // Overridden:
        assert_eq!(v1.stats.unwrap().x, 5);
        assert_eq!(v1.stats.unwrap().y, 5);
    }

    #[test]
    fn cost_source_attached_parses() {
        let card = load_card_from_lua(
            r#"return {
                id = "under-test",
                type = "creature",
                colors = {"green"},
                stats = {x = 1, y = 1},
                cost = {{amount = 2, source = "attached"}},
            }"#,
        );
        assert_eq!(card.cost.len(), 1);
        assert_eq!(card.cost[0].amount, 2);
        assert!(matches!(card.cost[0].source, CostSource::Attached));
    }

    #[test]
    fn sandbox_denies_dangerous_stdlib() {
        // Empty registry — just inspect the VM's globals.
        let tmp = std::env::temp_dir().join("tsot_sandbox_probe");
        std::fs::create_dir_all(&tmp).unwrap();
        if let Ok(rd) = std::fs::read_dir(&tmp) {
            for entry in rd.flatten() {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        let registry = CardRegistry::load(&tmp).unwrap();
        let globals = registry.lua().globals();

        for forbidden in ["os", "io", "package", "debug", "loadstring", "dofile", "loadfile", "require"] {
            let v: Value = globals.get(forbidden).unwrap();
            assert!(
                matches!(v, Value::Nil),
                "expected `{forbidden}` to be nil in sandboxed VM, got {v:?}"
            );
        }
        for allowed in ["math", "string", "table"] {
            let v: Value = globals.get(allowed).unwrap();
            assert!(
                matches!(v, Value::Table(_)),
                "expected `{allowed}` to be present in sandboxed VM"
            );
        }
    }
}
