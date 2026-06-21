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

use crate::teranos::{flower_at, surface_z, tile_at, Flower, TileKind, WORLD_CIRC_X};
use crate::trace::count_viewport_read;
use crate::world::{World, PIXELS_PER_TILE};

/// One tile in the viewport. 8 bytes, repr(C), no padding.
///
/// `has_flower` is 0 or 1; the flower fields are valid iff `has_flower == 1`.
/// We don't try to pack "no flower" into an out-of-range value — explicit
/// presence byte is easier to read on both sides and self-documenting.
#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct TileCell {
    pub tile_kind: u8,    // offset 0: TileKind discriminant
    pub elev_offset: i8,  // offset 1: surface_z - player.z (clamped into i8)
    pub has_flower: u8,   // offset 2
    pub petal_center: u8, // offset 3: FlowerColor discriminant; 0 if !has_flower
    pub petal_edge: u8,   // offset 4
    pub core_center: u8,  // offset 5: FlowerCore discriminant
    pub core_edge: u8,    // offset 6: CoreEdge discriminant
    pub petal_count: u8,  // offset 7
}

const _: () = assert!(core::mem::size_of::<TileCell>() == 8);
const _: () = assert!(core::mem::align_of::<TileCell>() == 1);

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
                let cell_flower = if world.player.picked.contains(&(cx, ty))
                    || world.canonical_picked.contains(&(cx, ty))
                {
                    None
                } else {
                    flower_at(tx, ty)
                };
                let elev_offset =
                    (sz - player_z).clamp(i8::MIN as i32, i8::MAX as i32) as i8;
                let cell = tile_cell(tile_at(tx, ty, top_z), elev_offset, cell_flower);
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

fn tile_cell(kind: TileKind, elev_offset: i8, flower: Option<Flower>) -> TileCell {
    let mut cell = TileCell {
        tile_kind: kind as u8,
        elev_offset,
        ..TileCell::default()
    };
    if let Some(f) = flower {
        cell.has_flower = 1;
        cell.petal_center = f.petal_center as u8;
        cell.petal_edge = f.petal_edge as u8;
        cell.core_center = f.core_center as u8;
        cell.core_edge = f.core_edge as u8;
        cell.petal_count = f.petal_count;
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
        assert_eq!(VIEWPORT_TILE_SIZE, 8);
        assert_eq!(VIEWPORT_HEADER_SIZE, 32);
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
            has_flower: 1,
            petal_center: FlowerColor::Pink as u8,
            petal_edge: FlowerColor::Glow as u8,
            core_center: FlowerCore::Black as u8,
            core_edge: CoreEdge::MatchPetalEdge as u8,
            petal_count: 8,
        };
        let mut buf = vec![0_u8; 8];
        write_struct(&mut buf, 0, &cell);
        assert_eq!(buf[0], TileKind::Grass as u8);
        assert_eq!(buf[1] as i8, -3);
        assert_eq!(buf[2], 1);
        assert_eq!(buf[3], FlowerColor::Pink as u8);
        assert_eq!(buf[4], FlowerColor::Glow as u8);
        assert_eq!(buf[5], FlowerCore::Black as u8);
        assert_eq!(buf[6], CoreEdge::MatchPetalEdge as u8);
        assert_eq!(buf[7], 8);
    }
}
