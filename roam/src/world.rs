use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::error::{emit as emit_error, Severity};
use crate::catalog::Catalog;
use crate::teranos::{
    pickup_at, surface_z, tile_at, CoreEdge, Flower, FlowerColor, FlowerCore, Pickup, TileKind,
    WORLD_CIRC_X,
};
use crate::trace::{count_state_read, count_tick, emit, TraceEvent};

pub const INPUT_W: u32 = 1 << 0;
pub const INPUT_A: u32 = 1 << 1;
pub const INPUT_S: u32 = 1 << 2;
pub const INPUT_D: u32 = 1 << 3;

pub const PIXELS_PER_TILE: u32 = 32;

const SPEED: f32 = 0.2; // pixels per ms
const SHALLOW_WATER_SPEED_MULT: f32 = 0.5;
const MAX_STEP_UP_DOWN: i32 = 1; // max |Δz| between adjacent walkable columns

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Facing {
    N = 0,
    NE = 1,
    E = 2,
    SE = 3,
    S = 4,
    SW = 5,
    W = 6,
    NW = 7,
}

impl Facing {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Strict byte → Facing. Returns None for unknown bytes; callers
    /// must decide what to do with an invalid byte rather than this
    /// function silently mapping it to a default.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Facing::N),
            1 => Some(Facing::NE),
            2 => Some(Facing::E),
            3 => Some(Facing::SE),
            4 => Some(Facing::S),
            5 => Some(Facing::SW),
            6 => Some(Facing::W),
            7 => Some(Facing::NW),
            _ => None,
        }
    }
}

pub struct Player {
    pub x: f32, // world pixels; wraps modulo WORLD_CIRC_X * PIXELS_PER_TILE
    pub y: f32, // world pixels; bounded by polar ocean
    pub z: i32, // voxel z of the tile the player is standing on
    pub facing: Facing,
    /// Tiles this player has already picked a flower from. Personal —
    /// per docs/CANONICAL.md every player has their own picked-set today;
    /// gossip-based first-claim-wins lands later. Key is canonical
    /// `(canonical_x_tile, y_tile)` so the cylinder wrap doesn't
    /// double-record.
    pub picked: BTreeSet<(i32, i32)>,
    /// One entry per picked-up `Pickup`. Generic so cards (v0.4) land
    /// as a new variant without reshaping inventory; until then every
    /// entry is `Pickup::Flower(_)`. Wire format still ships flowers
    /// as `FlowerWire`; the variant tag joins the wire when a second
    /// variant exists.
    pub inventory: Vec<Pickup>,
}

pub struct World {
    pub player: Player,
    /// Application-layer network state, owned by Rust per the seam in
    /// `crate::net`. `None` until the JS bridge constructs a provider
    /// and calls `roam_net_init` — that happens after libp2p is ready,
    /// which is async on the JS side. The hot path (frame loop) must
    /// tolerate `net.is_none()` during the bootstrap window.
    pub net: Option<crate::net::state::Net>,
    /// Canonical layer's claimed flower tiles. Per docs/CANONICAL.md every
    /// identified player's pickup propagates here; a tile in this set
    /// is gone from every identified player's view. `Player.picked` is
    /// distinct — it records what THIS player picked themselves, used
    /// to skip re-pickup attempts. The two sets overlap for our own
    /// canonical pickups and diverge for (a) other peers' canonical
    /// pickups, which land in `canonical_picked` only, and (b) future
    /// non-canonical players' pickups, which land in `Player.picked`
    /// only. Reconstructed from gossipsub on join, never persisted in
    /// the session JSON (the canonical layer is not per-player).
    pub canonical_picked: BTreeSet<(i32, i32)>,
    /// Card catalog as published by the relayer this session is
    /// connected to. Empty until the relayer publishes; worldgen
    /// degrades to "no cards on the ground yet" when empty rather
    /// than inventing IDs locally. Per the v0.4 design: different
    /// relayer = different world = different catalog.
    pub catalog: Catalog,
}

#[inline]
fn world_circ_px() -> f32 {
    (WORLD_CIRC_X * PIXELS_PER_TILE as i32) as f32
}

#[inline]
fn pixel_to_tile(px: f32) -> i32 {
    (px / PIXELS_PER_TILE as f32).floor() as i32
}

#[inline]
fn tile_to_pixel_center(t: i32) -> f32 {
    (t as f32 + 0.5) * PIXELS_PER_TILE as f32
}

// Allowable z for walking on column (tx, ty), and whether it's shallow water.
// Returns None if the column is impassable (deep water, polar ocean).
fn column_target_z(tx: i32, ty: i32) -> Option<(i32, bool)> {
    let sz = surface_z(tx, ty);
    if sz < 0 {
        match tile_at(tx, ty, 0) {
            TileKind::ShallowWater => Some((0, true)),
            TileKind::DeepWater => None,
            _ => None,
        }
    } else {
        Some((sz, false))
    }
}

fn facing_from_input(dx: f32, dy: f32) -> Option<Facing> {
    use Facing::*;
    let sx = dx.signum() as i32 * (dx != 0.0) as i32;
    let sy = dy.signum() as i32 * (dy != 0.0) as i32;
    Some(match (sx, sy) {
        (0, -1) => N,
        (1, -1) => NE,
        (1, 0) => E,
        (1, 1) => SE,
        (0, 1) => S,
        (-1, 1) => SW,
        (-1, 0) => W,
        (-1, -1) => NW,
        _ => return None,
    })
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        // Spawn at (0, 0) tile center. Player z snaps to the surface
        // (or water surface if (0,0) happens to be under sea level).
        let spawn_tx = 0;
        let spawn_ty = 0;
        let spawn_sz = surface_z(spawn_tx, spawn_ty);
        let spawn_z = spawn_sz.max(0);
        let spawn_x = tile_to_pixel_center(spawn_tx);
        let spawn_y = tile_to_pixel_center(spawn_ty);
        emit(TraceEvent::Init {
            spawn_x,
            spawn_y,
            spawn_z,
        });
        World {
            canonical_picked: BTreeSet::new(),
            catalog: Catalog::new(),
            player: Player {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
                facing: Facing::S,
                picked: BTreeSet::new(),
                inventory: Vec::new(),
            },
            net: None,
        }
    }

    /// Pickup check: if the player's current tile has a flower, route
    /// the pickup through the supplied `class` per docs/CANONICAL.md.
    ///
    /// Canonical: the canonical layer's claimed set grows by one;
    /// downstream gossip (next slice) will propagate. NonCanonical:
    /// the pickup lands in the personal sandbox; no propagation.
    /// Either way the player's own `picked` set + `inventory` get
    /// the update — those are this-player state, not class-dependent.
    ///
    /// A tile already in `canonical_picked` is unpickable for anyone
    /// — that's the canonical layer's first-claim-wins rule. A tile
    /// already in `Player.picked` is unpickable for this player only
    /// (prevents re-picking your own claim).
    pub(crate) fn try_pickup(&mut self, class: crate::identity::WorldClass) {
        let tx = pixel_to_tile(self.player.x);
        let ty = pixel_to_tile(self.player.y);
        let cx = tx.rem_euclid(WORLD_CIRC_X);
        let key = (cx, ty);
        if self.canonical_picked.contains(&key) || self.player.picked.contains(&key) {
            return;
        }
        if let Some(pickup) = pickup_at(tx, ty, &self.catalog) {
            self.player.picked.insert(key);
            self.player.inventory.push(pickup);
            match class {
                crate::identity::WorldClass::Canonical => {
                    self.canonical_picked.insert(key);
                    emit(TraceEvent::Note {
                        tag: "flower_picked_canonical",
                        msg: format!("({tx}, {ty}) {pickup:?}"),
                    });
                    if let Some(net) = self.net.as_mut() {
                        if let Err(err) = net.publish_pickup(cx, ty) {
                            emit_error(
                                Severity::Warn,
                                "roam::world::try_pickup",
                                "canonical pickup publish failed",
                                format!("({cx}, {ty}) reason={err:?}"),
                            );
                        }
                    }
                }
                crate::identity::WorldClass::NonCanonical => {
                    emit(TraceEvent::Note {
                        tag: "flower_picked_sandbox",
                        msg: format!("({tx}, {ty}) {pickup:?}"),
                    });
                }
            }
        }
    }

    pub fn step(&mut self, input: u32, dt_ms: f32) {
        let bit = |b: u32| (input & b != 0) as i32;
        let mut dx = (bit(INPUT_D) - bit(INPUT_A)) as f32;
        let mut dy = (bit(INPUT_S) - bit(INPUT_W)) as f32;
        let mag = (dx * dx + dy * dy).sqrt();
        if mag > 0.0 {
            dx /= mag;
            dy /= mag;
        }

        if let Some(f) = facing_from_input(dx, dy) {
            self.player.facing = f;
        }

        // Speed multiplier from the tile the player is CURRENTLY on.
        // Shallow water slows; everything else full speed.
        let speed_mult = match self.current_tile_kind() {
            TileKind::ShallowWater => SHALLOW_WATER_SPEED_MULT,
            _ => 1.0,
        };

        let mvx = dx * SPEED * dt_ms * speed_mult;
        let mvy = dy * SPEED * dt_ms * speed_mult;

        let nx = self.player.x + mvx;
        let blocked_x = !self.try_set_position(nx, self.player.y);
        if !blocked_x {
            self.player.x = nx;
        }
        let ny = self.player.y + mvy;
        let blocked_y = !self.try_set_position(self.player.x, ny);
        if !blocked_y {
            self.player.y = ny;
        }

        // Pickup check on each step — cheap, fires only when the
        // player's current tile has an unpicked flower. M6 routes
        // by identified-status; today's 0.3.2 hard-fail means the
        // net only ever initializes when persistent identity bytes
        // exist, so `net.is_some()` is the runtime proxy for
        // `is_identified_self`. The proxy collapses to the predicate
        // once guest-mode entry lands and the two paths diverge.
        let class = crate::identity::route_for_actor(self.net.is_some());
        self.try_pickup(class);

        // Cylindrical wrap in x.
        let circ = world_circ_px();
        if self.player.x < 0.0 || self.player.x >= circ {
            self.player.x = self.player.x.rem_euclid(circ);
        }

        count_tick(blocked_x || blocked_y);
    }

    fn current_tile_kind(&self) -> TileKind {
        let tx = pixel_to_tile(self.player.x);
        let ty = pixel_to_tile(self.player.y);
        tile_at(tx, ty, self.player.z)
    }

    // Returns true if (x, y) is walkable from the player's current z, and
    // updates player.z to the destination column's surface (or water surface).
    // Returns false if blocked by deep water, cliff (|Δz| > MAX_STEP_UP_DOWN),
    // or polar ocean.
    fn try_set_position(&mut self, x: f32, y: f32) -> bool {
        let tx = pixel_to_tile(x);
        let ty = pixel_to_tile(y);
        let Some((new_z, _is_shallow)) = column_target_z(tx, ty) else {
            return false;
        };
        if (new_z - self.player.z).abs() > MAX_STEP_UP_DOWN {
            return false;
        }
        self.player.z = new_z;
        true
    }

    /// Restore from a persisted (x, y, facing). Z snaps to the column's
    /// surface (or water surface for water columns). An invalid facing
    /// byte falls back to South *and* emits a Note so the desync shows
    /// in the trace bus — silently substituting a default without
    /// surfacing the cause would violate the sacred-error axiom.
    pub fn set_position(&mut self, x: f32, y: f32, facing_byte: u8) {
        let circ = world_circ_px();
        self.player.x = x.rem_euclid(circ);
        self.player.y = y;
        let tx = pixel_to_tile(self.player.x);
        let ty = pixel_to_tile(self.player.y);
        let sz = crate::teranos::surface_z(tx, ty);
        self.player.z = sz.max(0);
        let facing = match Facing::from_u8(facing_byte) {
            Some(f) => f,
            None => {
                emit(TraceEvent::Note {
                    tag: "set_position_bad_facing",
                    msg: format!(
                        "facing byte {facing_byte} out of [0..=7]; falling back to S and surfacing"
                    ),
                });
                Facing::S
            }
        };
        self.player.facing = facing;
        emit(TraceEvent::Note {
            tag: "set_position",
            msg: format!(
                "restored to ({:.1}, {:.1}, z={}) f={}",
                self.player.x,
                self.player.y,
                self.player.z,
                self.player.facing.as_u8()
            ),
        });
    }

    /// Serialize the per-player session state — picked-set + inventory —
    /// for localStorage. Wire shape is defined by the `SessionSnapshot`
    /// struct below: adding a Flower field is one edit there and serde
    /// carries it.
    pub fn session_snapshot_json(&self) -> String {
        let snap = SessionSnapshot {
            picked: self.player.picked.iter().copied().collect(),
            inv: self.player.inventory.iter().map(pickup_to_wire).collect(),
        };
        serde_json::to_string(&snap).unwrap_or_else(|err| {
            emit_error(
                Severity::Error,
                "roam::world::session_snapshot_json",
                "serde encode failed",
                err.to_string(),
            );
            String::from(r#"{"picked":[],"inv":[]}"#)
        })
    }

    /// Restore picked-set + inventory from a previous session snapshot.
    ///
    /// Accepts two inventory entry shapes for one-shot migration: the
    /// canonical named-field object and the legacy positional tuple
    /// `[pc, cc, n, pe, ce]`. Anything else is malformed; the parser
    /// fails loudly via a sacred-error event rather than silently
    /// substituting defaults. The legacy branch is removed in a
    /// later slice once no v1 sessions remain in the wild.
    pub fn restore_session_json(&mut self, raw: &str) {
        match serde_json::from_str::<SessionSnapshot>(raw) {
            Ok(snap) => {
                self.player.picked = snap.picked.into_iter().collect();
                let mut migrated = 0_usize;
                let mut parsed = 0_usize;
                self.player.inventory.clear();
                for entry in snap.inv {
                    match entry {
                        FlowerWire::Named(named) => match named.try_into_flower() {
                            Ok(f) => {
                                self.player.inventory.push(Pickup::Flower(f));
                                parsed += 1;
                            }
                            Err(why) => {
                                emit_error(
                                    Severity::Error,
                                    "roam::world::restore_session_json",
                                    "inventory entry rejected",
                                    why,
                                );
                            }
                        },
                        FlowerWire::Legacy(legacy) => match legacy_tuple_to_flower(&legacy) {
                            Ok(f) => {
                                self.player.inventory.push(Pickup::Flower(f));
                                migrated += 1;
                            }
                            Err(why) => {
                                emit_error(
                                    Severity::Error,
                                    "roam::world::restore_session_json",
                                    "legacy inventory entry rejected",
                                    why,
                                );
                            }
                        },
                    }
                }
                if migrated > 0 {
                    emit(TraceEvent::Note {
                        tag: "session_migrate_inventory",
                        msg: format!(
                            "restored {parsed} named-shape + {migrated} legacy-tuple inventory entries"
                        ),
                    });
                }
            }
            Err(err) => {
                emit_error(
                    Severity::Error,
                    "roam::world::restore_session_json",
                    "session snapshot rejected",
                    format!("serde decode failed: {err}; raw[..120]={:?}", &raw.chars().take(120).collect::<String>()),
                );
            }
        }
    }

    pub fn state_json(&self) -> String {
        count_state_read();
        let state = PlayerStateJson {
            x: self.player.x,
            y: self.player.y,
            z: self.player.z,
            f: self.player.facing.as_u8(),
            inv: self.player.inventory.iter().map(pickup_to_wire).collect(),
        };
        serde_json::to_string(&state).unwrap_or_else(|err| {
            emit_error(
                Severity::Error,
                "roam::world::state_json",
                "serde encode failed",
                err.to_string(),
            );
            String::from(r#"{"x":0,"y":0,"z":0,"f":4,"inv":[]}"#)
        })
    }

}

// ----- wire shapes: serde-derived JSON for state + session -----

/// Session snapshot wire format. Versioning happens by addition: new
/// fields default to None; existing readers ignore unknown fields per
/// serde's default behavior.
#[derive(Serialize, Deserialize)]
struct SessionSnapshot {
    picked: Vec<(i32, i32)>,
    inv: Vec<FlowerWire>,
}

/// State wire format consumed by the JS HUD. Inventory shape matches
/// `SessionSnapshot::inv`; the bridge reads `f.pc / .pe / .cc / .ce / .n`
/// for the inventory panel.
#[derive(Serialize)]
struct PlayerStateJson {
    x: f32,
    y: f32,
    z: i32,
    f: u8,
    inv: Vec<FlowerWire>,
}

/// On-wire flower representation. Two shapes accepted on decode:
/// the canonical named-field object and the legacy positional tuple
/// `[pc, cc, n, pe, ce]`. Encode is always Named (the `From<Flower>`
/// impl produces only `Named`); the legacy branch is a one-shot
/// migration for inbound v1 data.
///
/// **Variant order is load-bearing for decode.** serde_json can
/// deserialize a struct from an array if the lengths match — putting
/// `Legacy([u8; 5])` first means an array `[a,b,c,d,e]` matches
/// `Legacy` before serde tries to fit it into `FlowerNamed` (which
/// would silently accept the array with misaligned field semantics).
#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(untagged)]
enum FlowerWire {
    Legacy([u8; 5]),
    Named(FlowerNamed),
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct FlowerNamed {
    pc: u8,
    pe: u8,
    cc: u8,
    ce: u8,
    n: u8,
}

impl From<Flower> for FlowerWire {
    fn from(f: Flower) -> Self {
        FlowerWire::Named(FlowerNamed {
            pc: f.petal_center as u8,
            pe: f.petal_edge as u8,
            cc: f.core_center as u8,
            ce: f.core_edge as u8,
            n: f.petal_count,
        })
    }
}

/// Project a `Pickup` to the on-wire flower shape. The wire format is
/// still flower-only — Card variant landed in `teranos.rs` but
/// `card_at` returns None for now, so no Card ever reaches inventory
/// and the `todo!()` arm is unreachable until worldgen for cards
/// lands. The next slice extends the wire format to a tagged shape
/// and replaces this projection.
fn pickup_to_wire(p: &Pickup) -> FlowerWire {
    match p {
        Pickup::Flower(f) => FlowerWire::from(*f),
        Pickup::Card(_) => {
            todo!("card wire format pending — card_at returns None this slice")
        }
    }
}

impl FlowerNamed {
    fn try_into_flower(self) -> Result<Flower, String> {
        Ok(Flower {
            petal_center: flower_color_from_u8(self.pc)?,
            petal_edge: flower_color_from_u8(self.pe)?,
            core_center: flower_core_from_u8(self.cc)?,
            core_edge: core_edge_from_u8(self.ce)?,
            petal_count: self.n,
        })
    }
}

/// Legacy positional inventory entry from before the named-shape
/// migration. Order: `[petal_center, core_center, petal_count,
/// petal_edge, core_edge]`. Only accepted on decode; new writes always
/// emit the named shape.
fn legacy_tuple_to_flower(tuple: &[u8; 5]) -> Result<Flower, String> {
    Ok(Flower {
        petal_center: flower_color_from_u8(tuple[0])?,
        petal_edge: flower_color_from_u8(tuple[3])?,
        core_center: flower_core_from_u8(tuple[1])?,
        core_edge: core_edge_from_u8(tuple[4])?,
        petal_count: tuple[2],
    })
}

fn flower_color_from_u8(v: u8) -> Result<FlowerColor, String> {
    match v {
        0 => Ok(FlowerColor::Red),
        1 => Ok(FlowerColor::Yellow),
        2 => Ok(FlowerColor::Blue),
        3 => Ok(FlowerColor::Purple),
        4 => Ok(FlowerColor::Azure),
        5 => Ok(FlowerColor::Pink),
        6 => Ok(FlowerColor::Glow),
        _ => Err(format!("FlowerColor discriminant out of range: {v}")),
    }
}

fn flower_core_from_u8(v: u8) -> Result<FlowerCore, String> {
    match v {
        0 => Ok(FlowerCore::White),
        1 => Ok(FlowerCore::Yellow),
        2 => Ok(FlowerCore::Black),
        _ => Err(format!("FlowerCore discriminant out of range: {v}")),
    }
}

fn core_edge_from_u8(v: u8) -> Result<CoreEdge, String> {
    match v {
        0 => Ok(CoreEdge::White),
        1 => Ok(CoreEdge::MatchPetalCenter),
        2 => Ok(CoreEdge::MatchPetalEdge),
        _ => Err(format!("CoreEdge discriminant out of range: {v}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_z_at_or_above_sea_level() {
        let w = World::new();
        assert!(w.player.z >= 0, "spawn z={} below sea level", w.player.z);
    }

    #[test]
    fn no_movement_keeps_position() {
        let mut w = World::new();
        let (x, y, z) = (w.player.x, w.player.y, w.player.z);
        w.step(0, 16.0);
        assert_eq!(w.player.x, x);
        assert_eq!(w.player.y, y);
        assert_eq!(w.player.z, z);
    }

    #[test]
    fn facing_updates_on_input() {
        let mut w = World::new();
        w.step(INPUT_D, 16.0); // east
        assert_eq!(w.player.facing, Facing::E);
        w.step(INPUT_W | INPUT_A, 16.0); // northwest
        assert_eq!(w.player.facing, Facing::NW);
    }

    #[test]
    fn facing_from_u8_rejects_out_of_range() {
        for v in 0..=7_u8 {
            assert!(Facing::from_u8(v).is_some(), "{v} should map to a Facing");
        }
        for v in 8_u8..=255 {
            assert!(Facing::from_u8(v).is_none(), "{v} must be rejected");
        }
    }

    #[test]
    fn x_wraps_at_world_circumference() {
        let mut w = World::new();
        let circ = world_circ_px();
        w.player.x = circ - 1.0;
        let dt = 10.0;
        let push = SPEED * dt + 2.0;
        w.player.x = circ - 1.0;
        w.step(INPUT_D, push / SPEED);
        assert!(w.player.x < circ);
        assert!(w.player.x >= 0.0);
    }

    #[test]
    fn session_round_trip_preserves_inventory() {
        let mut w = World::new();
        w.player.inventory.push(Pickup::Flower(Flower {
            petal_center: FlowerColor::Red,
            petal_edge: FlowerColor::Glow,
            core_center: FlowerCore::Black,
            core_edge: CoreEdge::MatchPetalEdge,
            petal_count: 8,
        }));
        w.player.picked.insert((10, -20));
        let snap = w.session_snapshot_json();
        let mut w2 = World::new();
        w2.restore_session_json(&snap);
        assert_eq!(w2.player.inventory.len(), 1);
        assert_eq!(w2.player.inventory[0], w.player.inventory[0]);
        assert!(w2.player.picked.contains(&(10, -20)));
    }

    #[test]
    fn restore_session_rejects_malformed_input_and_surfaces_error() {
        crate::error::reset();
        let mut w = World::new();
        w.restore_session_json("not even json");
        assert!(w.player.inventory.is_empty(), "malformed input must not silently load anything");
        let errs = crate::error::drain();
        assert!(!errs.is_empty(), "sacred-error: malformed restore must surface");
        assert_eq!(errs[0].context.surface, "roam::world::restore_session_json");
    }

    #[test]
    fn restore_session_rejects_out_of_range_discriminant() {
        crate::error::reset();
        let mut w = World::new();
        // pc=99 is out of FlowerColor range. The named-shape parse
        // succeeds at the JSON level, but the discriminant check
        // rejects it; the inventory must stay empty and the error
        // must surface to the sacred-error log.
        let bad = r#"{"picked":[],"inv":[{"pc":99,"pe":0,"cc":0,"ce":0,"n":5}]}"#;
        w.restore_session_json(bad);
        assert!(w.player.inventory.is_empty());
        let errs = crate::error::drain();
        assert!(!errs.is_empty());
        assert!(
            errs.iter().any(|e| e.why.contains("FlowerColor discriminant out of range")),
            "sacred-error: out-of-range discriminant must say so"
        );
    }

    /// Finds the first `(tx, ty)` in a small deterministic window
    /// that has a flower. Used to position the player on a known
    /// flower tile without hard-coding a coordinate — if worldgen
    /// changes the flower pattern, the test still finds one.
    fn first_flower_tile() -> (i32, i32) {
        // Probe via flower_at directly: this helper looks for a flower
        // tile specifically (M6 routing tests need a tile with a flower
        // on it), so going through pickup_at + Catalog would add noise.
        for ty in -10..=10 {
            for tx in 0..50 {
                if crate::teranos::flower_at(tx, ty).is_some() {
                    return (tx, ty);
                }
            }
        }
        panic!("no flowers in the test scan window — worldgen broken?");
    }

    fn place_player_at_tile(w: &mut World, tx: i32, ty: i32) {
        w.player.x = (tx as f32 + 0.5) * PIXELS_PER_TILE as f32;
        w.player.y = (ty as f32 + 0.5) * PIXELS_PER_TILE as f32;
    }

    /// M6 — identified actor's pickup routes Canonical: the canonical
    /// layer's picked-set gets the tile, the personal picked-set
    /// gets the tile (so the player can't re-pick), and inventory
    /// grows by one. Falsifies the regression where the Canonical
    /// branch quietly degenerates to NonCanonical behavior (canonical
    /// layer never updates → other identified peers won't see the
    /// tile as claimed → duplicate pickups).
    #[test]
    fn identified_pickup_routes_canonical_layer() {
        let mut w = World::new();
        let (fx, fy) = first_flower_tile();
        place_player_at_tile(&mut w, fx, fy);
        let cx = fx.rem_euclid(WORLD_CIRC_X);
        w.try_pickup(crate::identity::WorldClass::Canonical);
        assert!(
            w.canonical_picked.contains(&(cx, fy)),
            "identified pickup must update canonical layer at ({cx}, {fy})"
        );
        assert!(w.player.picked.contains(&(cx, fy)), "personal picked must also record");
        assert_eq!(w.player.inventory.len(), 1, "inventory grows by one");
    }

    /// M6 — non-canonical actor's pickup stays in the sandbox: personal
    /// picked-set gets the tile, inventory grows, but the canonical
    /// layer does NOT update. Anti-grief by structure: unidentified
    /// players' world mutations are invisible to identified peers.
    /// Falsifies the regression where the NonCanonical branch leaks
    /// into the canonical set (defeats the whole sandbox model).
    #[test]
    fn non_canonical_pickup_does_not_touch_canonical_layer() {
        let mut w = World::new();
        let (fx, fy) = first_flower_tile();
        place_player_at_tile(&mut w, fx, fy);
        let cx = fx.rem_euclid(WORLD_CIRC_X);
        w.try_pickup(crate::identity::WorldClass::NonCanonical);
        assert!(
            w.canonical_picked.is_empty(),
            "non-canonical pickup must NOT update canonical layer"
        );
        assert!(w.player.picked.contains(&(cx, fy)), "sandbox player still remembers");
        assert_eq!(w.player.inventory.len(), 1, "inventory grows by one");
    }

    /// M6 — a tile already claimed in the canonical layer is unpickable
    /// regardless of routing class. Models the "peer X already picked
    /// this canonically" case: I receive their gossipsub message, my
    /// canonical_picked gets the tile, and now my own pickup attempt
    /// must be rejected. Falsifies the regression where canonical
    /// claims don't block local pickup (would let two players pick
    /// the same flower because one ingested the other's claim too
    /// slowly).
    #[test]
    fn canonical_claimed_tile_blocks_local_pickup() {
        let mut w = World::new();
        let (fx, fy) = first_flower_tile();
        let cx = fx.rem_euclid(WORLD_CIRC_X);
        w.canonical_picked.insert((cx, fy));
        place_player_at_tile(&mut w, fx, fy);
        w.try_pickup(crate::identity::WorldClass::Canonical);
        assert_eq!(
            w.player.inventory.len(),
            0,
            "tile already canonically claimed — pickup must be no-op"
        );
    }

    #[test]
    fn restore_session_migrates_legacy_tuple() {
        let mut w = World::new();
        // Legacy snapshot: positional tuple inventory.
        let legacy = r#"{"picked":[[1,2]],"inv":[[0,2,8,6,2]]}"#;
        w.restore_session_json(legacy);
        assert_eq!(w.player.inventory.len(), 1);
        let Pickup::Flower(f) = w.player.inventory[0] else {
            panic!("legacy tuple must restore as Pickup::Flower")
        };
        assert_eq!(f.petal_center, FlowerColor::Red);
        assert_eq!(f.core_center, FlowerCore::Black);
        assert_eq!(f.petal_count, 8);
        assert_eq!(f.petal_edge, FlowerColor::Glow);
        assert_eq!(f.core_edge, CoreEdge::MatchPetalEdge);
    }
}
