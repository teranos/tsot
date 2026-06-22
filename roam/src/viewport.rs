//! Binary viewport buffer.
//!
//! Rust writes a fixed-layout `[ViewportHeader, TileCell × N]` byte
//! sequence into a thread-local `Vec<u8>`. JS reads it through a
//! `DataView` over wasm linear memory at the offsets declared here.
//! No JSON, no char-packed enums, no parallel strings — every byte
//! has a typed home.
//!
//! The wire format is *defined by* the `#[repr(C)]` structs in this
//! module. The compile-time `assert!`s on field sizes lock the layout;
//! adding a Flower field is a one-line change here that JS picks up
//! at the matching DataView offset.

use std::cell::RefCell;

use crate::teranos::{card_at, flower_at, surface_z, tile_at, Flower, TileKind, WORLD_CIRC_X};
use crate::trace::count_viewport_read;
use crate::world::{World, PIXELS_PER_TILE};

/// One tile in the viewport. 12 bytes, repr(C), no padding.
///
/// `pickup_kind` discriminates the pickup variant on this tile:
///   0 = none, 1 = flower, 2 = card.
/// The flower fields (offset 3..7) are valid iff `pickup_kind == 1`;
/// `card_seed` (offset 8..11) is valid iff `pickup_kind == 2`. We
/// don't try to pack "no pickup" into an out-of-range value — explicit
/// kind byte is easier to read on both sides and self-documenting.
///
/// `card_seed` is a deterministic u32 hash of the card's string id
/// (`Catalog::seed_at_index`). The renderer hashes this to an RGB
/// color so the visual follows the *card*, not the *index*: catalog
/// reorders keep each card's color stable. The full ccg id never
/// touches the per-frame render path — the variable-length string is
/// looked up via `Catalog` only when a player actually picks the card
/// up.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct TileCell {
    pub tile_kind: u8,      // offset 0: TileKind discriminant
    pub elev_offset: i8,    // offset 1: surface_z - player.z (clamped into i8)
    pub pickup_kind: u8,    // offset 2: 0=none, 1=flower, 2=card
    pub petal_center: u8,   // offset 3: FlowerColor discriminant; 0 if !flower
    pub petal_edge: u8,     // offset 4
    pub core_center: u8,    // offset 5: FlowerCore discriminant
    pub core_edge: u8,      // offset 6: CoreEdge discriminant
    pub petal_count: u8,    // offset 7
    pub card_seed: [u8; 4], // offset 8..11: u32 LE; render color seed; zero if !card
}

const _: () = assert!(core::mem::size_of::<TileCell>() == 12);
const _: () = assert!(core::mem::align_of::<TileCell>() == 1);

/// `pickup_kind` discriminants. Public so consumers (render_gl, JS via
/// the FFI table layout in this module's docstring) speak the same
/// byte values. Renumbering these values is a wire-format change.
pub const PICKUP_KIND_NONE: u8 = 0;
pub const PICKUP_KIND_FLOWER: u8 = 1;
pub const PICKUP_KIND_CARD: u8 = 2;

/// Fixed-layout header. Always 32 bytes; JS skips this many bytes before
/// indexing into the TileCell array.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct ViewportHeader {
    pub view_w: u32,          // offset 0
    pub view_h: u32,          // offset 4
    pub center_tx: i32,       // offset 8
    pub center_ty: i32,       // offset 12
    pub player_z: i32,        // offset 16
    pub pixels_per_tile: u32, // offset 20
    pub day_brightness: u32,  // offset 24, fixed-point Q8.24 (0.0..=1.0 → 0..=2^24)
    pub _reserved: u32,       // offset 28
}

const _: () = assert!(core::mem::size_of::<ViewportHeader>() == 32);

pub const VIEWPORT_HEADER_SIZE: usize = core::mem::size_of::<ViewportHeader>();
pub const VIEWPORT_TILE_SIZE: usize = core::mem::size_of::<TileCell>();

thread_local! {
    static VIEWPORT_BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Write the viewport for `world` into the static buffer. Returns the
/// total byte length (header + tiles). The pointer is exposed via
/// `viewport_ptr` for the JS bridge to read.
pub fn write_viewport(world: &World, view_w: u32, view_h: u32) -> u32 {
    VIEWPORT_BUFFER.with(|b| {
        let mut buf = b.borrow_mut();
        let total = VIEWPORT_HEADER_SIZE + VIEWPORT_TILE_SIZE * (view_w * view_h) as usize;
        buf.resize(total, 0);

        let center_tx = (world.player.x / PIXELS_PER_TILE as f32).floor() as i32;
        let center_ty = (world.player.y / PIXELS_PER_TILE as f32).floor() as i32;
        let player_z = world.player.z;

        let header = ViewportHeader {
            view_w,
            view_h,
            center_tx,
            center_ty,
            player_z,
            pixels_per_tile: PIXELS_PER_TILE,
            day_brightness: 0,
            _reserved: 0,
        };
        write_struct(&mut buf, 0, &header);

        let half_w = view_w as i32 / 2;
        let half_h = view_h as i32 / 2;
        let mut i = 0;
        for dy in -half_h..(view_h as i32 - half_h) {
            for dx in -half_w..(view_w as i32 - half_w) {
                let tx = center_tx + dx;
                let ty = center_ty + dy;
                let sz = surface_z(tx, ty);
                let top_z = sz.max(0);
                let cx = tx.rem_euclid(WORLD_CIRC_X);
                let hidden = world.player.picked.contains(&(cx, ty))
                    || world.canonical_picked.contains(&(cx, ty));
                // Render path composes worldgen + catalog directly, NOT
                // via `pickup_at`, because pickup_at clones a `String`
                // per card tile (the ccg slug). Per-frame allocation on
                // every visible card tile would be wasteful; the seed
                // is enough for rendering.
                let cell_pickup = if hidden {
                    CellPickup::None
                } else if let Some(f) = flower_at(tx, ty) {
                    CellPickup::Flower(f)
                } else if let Some(idx) = card_at(tx, ty, world.catalog.len()) {
                    match world.catalog.seed_at_index(idx) {
                        Some(seed) => CellPickup::Card(seed),
                        None => CellPickup::None,
                    }
                } else {
                    CellPickup::None
                };
                let elev_offset =
                    (sz - player_z).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
                let cell = tile_cell(tile_at(tx, ty, top_z), elev_offset, cell_pickup);
                write_struct(
                    &mut buf,
                    VIEWPORT_HEADER_SIZE + i * VIEWPORT_TILE_SIZE,
                    &cell,
                );
                i += 1;
            }
        }
        count_viewport_read();
        total as u32
    })
}

pub fn viewport_ptr() -> u32 {
    VIEWPORT_BUFFER.with(|b| b.borrow().as_ptr() as u32)
}

/// What a tile cell carries for render purposes. Distinct from
/// `teranos::Pickup`: this enum exists per-frame and never allocates
/// (the `Card` variant carries the precomputed color seed, not the
/// owned `CardId` string).
enum CellPickup {
    None,
    Flower(Flower),
    Card(u32),
}

fn tile_cell(kind: TileKind, elev_offset: i8, pickup: CellPickup) -> TileCell {
    let mut cell = TileCell {
        tile_kind: kind as u8,
        elev_offset,
        ..TileCell::default()
    };
    match pickup {
        CellPickup::Flower(f) => {
            cell.pickup_kind = PICKUP_KIND_FLOWER;
            cell.petal_center = f.petal_center as u8;
            cell.petal_edge = f.petal_edge as u8;
            cell.core_center = f.core_center as u8;
            cell.core_edge = f.core_edge as u8;
            cell.petal_count = f.petal_count;
        }
        CellPickup::Card(seed) => {
            cell.pickup_kind = PICKUP_KIND_CARD;
            cell.card_seed = seed.to_le_bytes();
        }
        CellPickup::None => {}
    }
    cell
}

fn write_struct<T: Copy>(buf: &mut [u8], offset: usize, value: &T) {
    let size = core::mem::size_of::<T>();
    let dest = &mut buf[offset..offset + size];
    // SAFETY: T is Copy + repr(C) (guaranteed by callers in this module).
    // `value` points to a valid T; `dest` is `size` bytes inside a
    // freshly-resized Vec<u8> that has no aliasing with `value`.
    unsafe {
        core::ptr::copy_nonoverlapping(
            value as *const T as *const u8,
            dest.as_mut_ptr(),
            size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teranos::{CoreEdge, FlowerColor, FlowerCore};

    #[test]
    fn tile_cell_layout_is_locked() {
        assert_eq!(VIEWPORT_TILE_SIZE, 12);
        assert_eq!(VIEWPORT_HEADER_SIZE, 32);
    }

    /// Card variant writes the precomputed color seed at offset 8..12
    /// as little-endian u32 bytes. JS / render_gl read `pickup_kind == 2`
    /// then the seed. Falsifies the regression where the kind byte
    /// says card but the seed bytes are stale or zeroed.
    #[test]
    fn tile_cell_card_writes_card_seed_bytes() {
        let cell = tile_cell(TileKind::Grass, 0, CellPickup::Card(0x0A0B_0C0D));
        assert_eq!(cell.pickup_kind, PICKUP_KIND_CARD);
        assert_eq!(cell.card_seed, [0x0D, 0x0C, 0x0B, 0x0A]);
        // Flower fields untouched (Default-zeroed).
        assert_eq!(cell.petal_center, 0);
        assert_eq!(cell.petal_count, 0);
    }

    /// Flower variant: pickup_kind == 1, card_seed stays zeroed, the
    /// flower bytes go where they go.
    #[test]
    fn tile_cell_flower_does_not_touch_card_bytes() {
        // Use a known flower (any tile with one) to avoid hand-constructing.
        let mut found = None;
        'outer: for ty in -10..=10 {
            for tx in 0..50 {
                if let Some(f) = flower_at(tx, ty) {
                    found = Some(f);
                    break 'outer;
                }
            }
        }
        let f = found.expect("scan window must contain a flower");
        let cell = tile_cell(TileKind::Grass, 0, CellPickup::Flower(f));
        assert_eq!(cell.pickup_kind, PICKUP_KIND_FLOWER);
        assert_eq!(cell.card_seed, [0, 0, 0, 0]);
        assert_eq!(cell.petal_center, f.petal_center as u8);
        assert_eq!(cell.petal_count, f.petal_count);
    }

    #[test]
    fn write_viewport_fills_buffer_to_expected_length() {
        let w = World::new();
        let len = write_viewport(&w, 4, 4) as usize;
        assert_eq!(len, VIEWPORT_HEADER_SIZE + VIEWPORT_TILE_SIZE * 16);
    }

    #[test]
    fn write_viewport_header_round_trips() {
        let w = World::new();
        let _ = write_viewport(&w, 8, 6);
        VIEWPORT_BUFFER.with(|b| {
            let buf = b.borrow();
            let view_w = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            let view_h = u32::from_le_bytes(buf[4..8].try_into().unwrap());
            let pixels_per_tile = u32::from_le_bytes(buf[20..24].try_into().unwrap());
            assert_eq!(view_w, 8);
            assert_eq!(view_h, 6);
            assert_eq!(pixels_per_tile, PIXELS_PER_TILE);
        });
    }

    #[test]
    fn tile_cell_roundtrip_through_buffer() {
        // Build a TileCell, write it through write_struct, read each
        // field back at the documented offset. Locks the wire format.
        let cell = TileCell {
            tile_kind: TileKind::Grass as u8,
            elev_offset: -3,
            pickup_kind: PICKUP_KIND_FLOWER,
            petal_center: FlowerColor::Pink as u8,
            petal_edge: FlowerColor::Glow as u8,
            core_center: FlowerCore::Black as u8,
            core_edge: CoreEdge::MatchPetalEdge as u8,
            petal_count: 8,
            card_seed: [0, 0, 0, 0],
        };
        let mut buf = vec![0_u8; 12];
        write_struct(&mut buf, 0, &cell);
        assert_eq!(buf[0], TileKind::Grass as u8);
        assert_eq!(buf[1] as i8, -3);
        assert_eq!(buf[2], PICKUP_KIND_FLOWER);
        assert_eq!(buf[3], FlowerColor::Pink as u8);
        assert_eq!(buf[4], FlowerColor::Glow as u8);
        assert_eq!(buf[5], FlowerCore::Black as u8);
        assert_eq!(buf[6], CoreEdge::MatchPetalEdge as u8);
        assert_eq!(buf[7], 8);
        // card_seed bytes at offset 8..12.
        assert_eq!(&buf[8..12], &[0u8, 0, 0, 0]);
    }

    /// Card-variant roundtrip: locks the wire-format card_seed LE bytes.
    #[test]
    fn tile_cell_card_roundtrip_through_buffer() {
        let cell = TileCell {
            tile_kind: TileKind::Grass as u8,
            elev_offset: 0,
            pickup_kind: PICKUP_KIND_CARD,
            petal_center: 0,
            petal_edge: 0,
            core_center: 0,
            core_edge: 0,
            petal_count: 0,
            card_seed: 0x1234_5678_u32.to_le_bytes(),
        };
        let mut buf = vec![0_u8; 12];
        write_struct(&mut buf, 0, &cell);
        assert_eq!(buf[2], PICKUP_KIND_CARD);
        let seed = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(seed, 0x1234_5678);
    }
}
