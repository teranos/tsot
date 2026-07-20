// Shared scene types + snapshot logic. Used by both the native
// wgpu render (render.rs) and the wasm hand-wired render
// (render_web.rs) so the WGSL and buffer layouts stay one source.

mod camera;
mod drape;
mod emit;
mod instance;
mod props;
mod snapshot;
mod terrain_surface;

pub use camera::SceneCamera;
pub use drape::{drape, drape_mesh};
pub use emit::{
    snapshot_to_ghost_instances, snapshot_to_glass_instances, snapshot_to_instances,
};
pub use instance::{
    INSTANCE_ATTRS, INSTANCE_STRIDE, GpuVertex, InstanceAttr, MeshInstance, SceneInstance,
    as_bytes, cube_geometry,
};
pub use snapshot::{RemotePeerDot, SceneSnapshot, StructureSnap, snapshot_scene};
pub use terrain_surface::{TerrainSurface, surface_snap, terrain_surface_mesh};
