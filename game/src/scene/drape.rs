use super::instance::{MeshInstance, SceneInstance};

/// Sit a batch of world instances ON the terrain — lift each by the
/// ground height under its XZ.
///
/// **This is a browser-tab survival pattern, not a nicety.** Every
/// renderable — buildings, trees, campfires, obstacles, pins, the
/// player — lives in one of a small fixed set of shared instance
/// streams (opaque, mesh, glass, ghost). Each stream goes to the GPU
/// as ONE buffer write per frame. `drape` runs once per stream, after
/// gather, before submit. Total GPU buffer writes stay O(streams), not
/// O(entity-types). Undo this — let each entity type hold its own
/// buffer and drape itself — and you fragment into a per-type
/// buffer-write storm that the browser tab does NOT survive under real
/// load (see the ~10K-segment draped-grid retirement in TERRAIN.md's
/// *Superseded* note).
///
/// The "new entity types drape for free" ergonomic is a side effect of
/// this invariant, not the reason for it. Reading these functions in
/// isolation makes the pattern look trivial (a Y-lift loop); the real
/// contract is upstream — everything lands in a shared stream.
///
/// Inside a stamp footprint `height` is the flat pad level, so a
/// building rises rigidly onto its (flat) pad; loose props drape onto
/// the rolling surface.
pub fn drape(instances: &mut [SceneInstance]) {
    for i in instances {
        i.pos[1] += crate::terrain::height(i.pos[0], i.pos[2]);
    }
}

/// `drape` for the mesh pipeline's instances. Same contract, same
/// browser-tab-survival reasoning — see `drape`.
pub fn drape_mesh(instances: &mut [MeshInstance]) {
    for i in instances {
        i.pos[1] += crate::terrain::height(i.pos[0], i.pos[2]);
    }
}
