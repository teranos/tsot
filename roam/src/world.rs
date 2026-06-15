use crate::teranos::{surface_z, tile_at, TileKind, WORLD_CIRC_X};
use crate::trace::{emit, TraceEvent};

pub const INPUT_W: u32 = 1 << 0;
pub const INPUT_A: u32 = 1 << 1;
pub const INPUT_S: u32 = 1 << 2;
pub const INPUT_D: u32 = 1 << 3;

pub const PIXELS_PER_TILE: u32 = 32;

const SPEED: f32 = 0.2; // pixels per ms
const SHALLOW_WATER_SPEED_MULT: f32 = 0.5;
const MAX_STEP_UP_DOWN: i32 = 1; // max |Δz| between adjacent walkable columns

const DEFAULT_VIEW_W: u32 = 32;
const DEFAULT_VIEW_H: u32 = 24;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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
    fn as_u8(self) -> u8 {
        self as u8
    }
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Facing::N,
            1 => Facing::NE,
            2 => Facing::E,
            3 => Facing::SE,
            4 => Facing::S,
            5 => Facing::SW,
            6 => Facing::W,
            7 => Facing::NW,
            _ => Facing::S,
        }
    }
}

pub struct Player {
    pub x: f32, // world pixels; wraps modulo WORLD_CIRC_X * PIXELS_PER_TILE
    pub y: f32, // world pixels; bounded by polar oceans
    pub z: i32, // voxel z of the tile the player is standing on
    pub facing: Facing,
}

pub struct World {
    pub player: Player,
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

fn tile_char(k: TileKind) -> char {
    match k {
        TileKind::Air => '0',
        TileKind::Grass => '1',
        TileKind::Rock => '2',
        TileKind::ShallowWater => '3',
        TileKind::DeepWater => '4',
    }
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
            player: Player {
                x: spawn_x,
                y: spawn_y,
                z: spawn_z,
                facing: Facing::S,
            },
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

        let before_x = self.player.x;
        let before_y = self.player.y;
        let before_z = self.player.z;

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

        // Cylindrical wrap in x.
        let circ = world_circ_px();
        if self.player.x < 0.0 || self.player.x >= circ {
            self.player.x = self.player.x.rem_euclid(circ);
        }

        emit(TraceEvent::Tick {
            input_bits: input,
            dt_ms,
            before_x,
            before_y,
            before_z,
            after_x: self.player.x,
            after_y: self.player.y,
            after_z: self.player.z,
            facing: self.player.facing.as_u8(),
            intended_dx: mvx,
            intended_dy: mvy,
            blocked_x,
            blocked_y,
        });
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

    // Restore from a persisted (x, y, facing). Z snaps to the column's
    // surface (or water surface for water columns) — the saved x/y
    // determines location; z is always derived from current terrain
    // so the saved player can never land inside a wall if terrain
    // generation parameters shift between sessions.
    pub fn set_position(&mut self, x: f32, y: f32, facing: u8) {
        let circ = world_circ_px();
        self.player.x = x.rem_euclid(circ);
        self.player.y = y;
        let tx = pixel_to_tile(self.player.x);
        let ty = pixel_to_tile(self.player.y);
        let sz = crate::teranos::surface_z(tx, ty);
        self.player.z = sz.max(0);
        self.player.facing = Facing::from_u8(facing);
        emit(TraceEvent::Note {
            tag: "set_position",
            msg: format!(
                "restored to ({:.1}, {:.1}, z={}) f={}",
                self.player.x, self.player.y, self.player.z, self.player.facing.as_u8()
            ),
        });
    }

    pub fn state_json(&self) -> String {
        emit(TraceEvent::StateRead {
            x: self.player.x,
            y: self.player.y,
            z: self.player.z,
            facing: self.player.facing.as_u8(),
        });
        format!(
            r#"{{"x":{},"y":{},"z":{},"f":{}}}"#,
            self.player.x,
            self.player.y,
            self.player.z,
            self.player.facing.as_u8()
        )
    }

    // Viewport tile data centered on the player. JS calls this per frame
    // to render the visible slice at the player's current z.
    pub fn map_json(&self) -> String {
        self.viewport_json(DEFAULT_VIEW_W, DEFAULT_VIEW_H)
    }

    // Top-down view: render the top of each column, not a flat slice at
    // player.z. Hills above player show as elevated terrain; valleys
    // below show through. JS uses the `elev` parallel string to shade
    // each tile by `surface_z - player.z`.
    //
    // Caves (when underground) will need a different code path — switch
    // to slice-at-player-z view when player.z < surface_z(player). Not
    // yet relevant; player always lives at the surface in v0.3.5.
    pub fn viewport_json(&self, view_w: u32, view_h: u32) -> String {
        let center_tx = pixel_to_tile(self.player.x);
        let center_ty = pixel_to_tile(self.player.y);
        let z = self.player.z;
        let half_w = view_w as i32 / 2;
        let half_h = view_h as i32 / 2;
        let cap = (view_w * view_h) as usize;
        let mut tiles = String::with_capacity(cap);
        let mut elev = String::with_capacity(cap);
        for dy in -half_h..(view_h as i32 - half_h) {
            for dx in -half_w..(view_w as i32 - half_w) {
                let tx = center_tx + dx;
                let ty = center_ty + dy;
                let sz = surface_z(tx, ty);
                let top_z = sz.max(0);
                tiles.push(tile_char(tile_at(tx, ty, top_z)));
                elev.push(elev_char(sz));
            }
        }
        emit(TraceEvent::ViewportRead {
            view_w,
            view_h,
            center_tx,
            center_ty,
            z,
        });
        format!(
            r#"{{"tile":{},"view_w":{},"view_h":{},"center_tx":{},"center_ty":{},"z":{},"tiles":"{}","elev":"{}"}}"#,
            PIXELS_PER_TILE, view_w, view_h, center_tx, center_ty, z, tiles, elev
        )
    }
}

// Encode signed z (-32..62) as one printable ASCII char.
// '!' = -32, 'A' = 0, 'a' = 32. Decodes as code - 33 - 32 (JS side).
fn elev_char(z: i32) -> char {
    let v = (z + 32).clamp(0, 94);
    char::from_u32((v as u32) + 33).unwrap_or('!')
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
    fn x_wraps_at_world_circumference() {
        let mut w = World::new();
        let circ = world_circ_px();
        w.player.x = circ - 1.0;
        // Apply a small eastward push — should wrap to ~0.
        let dt = 10.0;
        let push = SPEED * dt + 2.0; // overshoot the wrap
        w.player.x = circ - 1.0;
        w.step(INPUT_D, push / SPEED);
        assert!(w.player.x < circ);
        assert!(w.player.x >= 0.0);
    }

    #[test]
    fn viewport_json_tiles_and_elev_match() {
        let w = World::new();
        let s = w.viewport_json(8, 6);
        // Extract the "tiles":"..." and "elev":"..." payloads.
        let tile_start = s.find(r#""tiles":""#).unwrap() + r#""tiles":""#.len();
        let after_tiles = s[tile_start..].find('"').unwrap();
        let tile_len = after_tiles;
        let elev_start = s.find(r#""elev":""#).unwrap() + r#""elev":""#.len();
        let after_elev = s[elev_start..].find('"').unwrap();
        let elev_len = after_elev;
        assert_eq!(tile_len, 48);
        assert_eq!(elev_len, 48);
    }

    #[test]
    fn elev_char_roundtrip() {
        for z in -32..=62 {
            let c = elev_char(z);
            let decoded = (c as i32) - 33 - 32;
            assert_eq!(decoded, z, "elev_char({z}) = {c:?} → decoded {decoded}");
        }
    }
}
