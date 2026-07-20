use crate::template::PropKind;

/// Per-kind colour + cube size for static structure props. Chairs and
/// tables read as distinct wooden furniture. The campfire never
/// reaches here (it renders through its own flickering path); it has a
/// fallback only so the match is total.
pub(super) fn prop_appearance(kind: PropKind) -> ([f32; 3], [f32; 3]) {
    match kind {
        PropKind::Chair => ([0.30, 0.20, 0.12], [28.0, 36.0, 28.0]),
        PropKind::Table => ([0.42, 0.28, 0.14], [64.0, 28.0, 64.0]),
        // Walls are one CDDA tile long, thin across the run. NS runs
        // along Z (thin in X); EW runs along X (thin in Z); the plain
        // Wall (corner/junction) fills the tile. Sizes match the
        // colliders in template.rs.
        PropKind::Wall => ([0.48, 0.47, 0.50], [80.0, 220.0, 80.0]),
        PropKind::WallNS => ([0.48, 0.47, 0.50], [24.0, 220.0, 80.0]),
        PropKind::WallEW => ([0.48, 0.47, 0.50], [80.0, 220.0, 24.0]),
        // Flat roof slab, sits at ROOF_HEIGHT (elevation comes from the
        // prop's y position, not this box).
        PropKind::Roof => ([0.33, 0.30, 0.34], [80.0, 20.0, 80.0]),
        PropKind::Furniture => ([0.34, 0.26, 0.20], [50.0, 70.0, 50.0]),
        PropKind::Campfire => ([1.0, 0.45, 0.08], [50.0, 60.0, 50.0]),
        // Glass panes fill their tile like the wall they sit in; the
        // translucency comes from the glass pass, not the colour. These
        // are only reached through `snapshot_to_glass_instances`.
        PropKind::Window => ([0.55, 0.70, 0.85], [80.0, 220.0, 80.0]),
        PropKind::WindowNS => ([0.55, 0.70, 0.85], [24.0, 220.0, 80.0]),
        PropKind::WindowEW => ([0.55, 0.70, 0.85], [80.0, 220.0, 24.0]),
        // Fence — bottom rail (single instance from prop_appearance).
        // The top rail is added by the structure loop as a second
        // instance, so the fence reads as two stacked thin bars with a
        // see-through gap between them (real-fence silhouette).
        PropKind::Fence => (FENCE_COLOR, [8.0, 6.0, 8.0]),
        PropKind::FenceNS => (FENCE_COLOR, [8.0, 6.0, 80.0]),
        PropKind::FenceEW => (FENCE_COLOR, [80.0, 6.0, 8.0]),
        // Toilet — white ceramic block, roughly toilet-sized.
        PropKind::Toilet => ([0.92, 0.94, 0.94], [36.0, 50.0, 44.0]),
    }
}

/// Weathered wood — same value across all three fence kinds so a fence
/// run reads as one continuous piece.
const FENCE_COLOR: [f32; 3] = [0.42, 0.32, 0.20];
/// Bottom rail sits low, top rail near the top of the 60-tall collider.
/// The gap between them (~35 units) is the see-through part.
pub(super) const FENCE_BOTTOM_Y: f32 = 12.0;
pub(super) const FENCE_TOP_Y: f32 = 48.0;

pub(super) fn is_fence(k: PropKind) -> bool {
    matches!(k, PropKind::Fence | PropKind::FenceNS | PropKind::FenceEW)
}
