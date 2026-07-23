//! Walls-on-mesh slice 2: tessellate a `cdda::WallGraph` into mesh
//! geometry (see `game/docs/RENDER.md`). Rectilinear only — runs are
//! quad bands, corners/T-junctions/crossings are resolved ONCE in the
//! junction node's thickness×thickness plan square (the miter). No
//! faces are ever emitted inside a miter square: that is the machine
//! proxy for "one wall turning" instead of "two boxes meeting".
//!
//! Positions weld, normals crease: a 90° corner keeps a hard edge with
//! two normals — that's what a real wall turning looks like. The
//! doubled-edge artifact came from separate boxes shading their own
//! end-caps inside the joint, not from the crease.
//!
//! Openings (slice 3): every vertical face is split on the canonical
//! sill/lintel lines (`Y_BANDS`) so band edges always meet band edges.
//! Window cells occupy only the outer bands — the glass band stays
//! open for the alpha pass — with sill top, lintel underside, and
//! reveals closing the solid neighbours. Door nodes carry no lateral
//! offsets and tessellate as gaps; a run ending beside a door caps at
//! the cell boundary — which IS the door jamb.

use cdda::{WallGraph, WallNode};

use crate::tree_mesh::MeshVertex;

/// Wall height — matches the prop path's wall size and `ROOF_HEIGHT`.
pub const WALL_HEIGHT: f32 = 220.0;
/// Window sill top — the tessellation constant (RENDER.md decision 1:
/// sill/lintel are not authored data). Matches the prop path's layers.
pub const SILL_TOP: f32 = 60.0;
/// Window lintel underside — glass band is SILL_TOP..HEAD_BOTTOM.
pub const HEAD_BOTTOM: f32 = 180.0;
/// Half thickness — matches `placement.rs`'s `WALL_HALF_THICKNESS`.
const HT: f32 = 12.0;
const CELL: f32 = cdda::CDDA_TILE;
const HC: f32 = CDDA_HALF;
const CDDA_HALF: f32 = 40.0;

/// The three canonical vertical bands. EVERY vertical face is split on
/// these lines — solid walls occupy all three, windows only the outer
/// two — so band edges always meet band edges and the mesh stays
/// manifold wherever solid meets window.
const Y_BANDS: [(f32, f32); 3] = [
    (0.0, SILL_TOP),
    (SILL_TOP, HEAD_BOTTOM),
    (HEAD_BOTTOM, WALL_HEIGHT),
];

/// Tessellate the whole graph. Offsets are template-local (same space
/// as `Prop.offset`); the caller instances the result at the building
/// anchor. Returns indexed triangles in the mesh pipeline's vertex
/// format.
pub fn wall_graph_mesh(g: &WallGraph) -> (Vec<MeshVertex>, Vec<u32>) {
    let mut out = MeshOut::default();

    // Neighbor lookups by exact cell offset — the graph's edges are the
    // source of adjacency; positions identify direction.
    let key = |x: f32, z: f32| -> (i64, i64) { (x.round() as i64, z.round() as i64) };
    let node_at = |x: f32, z: f32| -> Option<&WallNode> {
        g.nodes
            .iter()
            .find(|n| key(n.offset.x, n.offset.z) == key(x, z))
    };
    let is_junction = |n: &WallNode| n.ew.is_some() && n.ns.is_some();

    // Which junction-square sides have a run abutting them — those
    // sides emit no face (the interface is interior to the wall solid).
    use std::collections::HashSet;
    let mut attached: HashSet<((i64, i64), u8)> = HashSet::new();
    const W: u8 = 0;
    const E: u8 = 1;
    const N: u8 = 2;
    const S: u8 = 3;

    // ---- EW runs ------------------------------------------------------
    // Group nodes carrying an EW piece by (row z, lateral offset,
    // colour), sort by x, split on gaps. Door cells carry no offset so
    // runs split at doors automatically.
    let mut ew_nodes: Vec<&WallNode> = g.nodes.iter().filter(|n| n.ew.is_some()).collect();
    ew_nodes.sort_by(|a, b| {
        a.offset
            .z
            .total_cmp(&b.offset.z)
            .then(a.ew.unwrap().total_cmp(&b.ew.unwrap()))
            .then(a.offset.x.total_cmp(&b.offset.x))
    });
    let mut i = 0;
    while i < ew_nodes.len() {
        let start = i;
        while i + 1 < ew_nodes.len()
            && ew_nodes[i + 1].offset.z == ew_nodes[start].offset.z
            && ew_nodes[i + 1].ew == ew_nodes[start].ew
            && (ew_nodes[i + 1].offset.x - ew_nodes[i].offset.x - CELL).abs() < 1e-3
        {
            i += 1;
        }
        let run = &ew_nodes[start..=i];
        emit_axis_run(&mut out, run, Axis::Ew, &node_at, &is_junction, &mut attached);
        i += 1;
    }

    // ---- NS runs ------------------------------------------------------
    let mut ns_nodes: Vec<&WallNode> = g.nodes.iter().filter(|n| n.ns.is_some()).collect();
    ns_nodes.sort_by(|a, b| {
        a.offset
            .x
            .total_cmp(&b.offset.x)
            .then(a.ns.unwrap().total_cmp(&b.ns.unwrap()))
            .then(a.offset.z.total_cmp(&b.offset.z))
    });
    let mut i = 0;
    while i < ns_nodes.len() {
        let start = i;
        while i + 1 < ns_nodes.len()
            && ns_nodes[i + 1].offset.x == ns_nodes[start].offset.x
            && ns_nodes[i + 1].ns == ns_nodes[start].ns
            && (ns_nodes[i + 1].offset.z - ns_nodes[i].offset.z - CELL).abs() < 1e-3
        {
            i += 1;
        }
        let run = &ns_nodes[start..=i];
        emit_axis_run(&mut out, run, Axis::Ns, &node_at, &is_junction, &mut attached);
        i += 1;
    }

    // ---- Junction squares --------------------------------------------
    for n in g.nodes.iter().filter(|n| is_junction(n)) {
        let (sx, sz) = square_center(n);
        let k = key(n.offset.x, n.offset.z);
        // Top cap.
        out.quad(
            [sx - HT, WALL_HEIGHT, sz - HT],
            [sx + HT, WALL_HEIGHT, sz - HT],
            [sx + HT, WALL_HEIGHT, sz + HT],
            [sx - HT, WALL_HEIGHT, sz + HT],
            [0.0, 1.0, 0.0],
        );
        // Side faces only where no run abuts — banded on the canonical
        // Y lines so square edges always meet banded run edges.
        for (y0, y1) in Y_BANDS {
            if !attached.contains(&(k, W)) {
                out.quad(
                    [sx - HT, y0, sz + HT],
                    [sx - HT, y0, sz - HT],
                    [sx - HT, y1, sz - HT],
                    [sx - HT, y1, sz + HT],
                    [-1.0, 0.0, 0.0],
                );
            }
            if !attached.contains(&(k, E)) {
                out.quad(
                    [sx + HT, y0, sz - HT],
                    [sx + HT, y0, sz + HT],
                    [sx + HT, y1, sz + HT],
                    [sx + HT, y1, sz - HT],
                    [1.0, 0.0, 0.0],
                );
            }
            if !attached.contains(&(k, N)) {
                out.quad(
                    [sx - HT, y0, sz - HT],
                    [sx + HT, y0, sz - HT],
                    [sx + HT, y1, sz - HT],
                    [sx - HT, y1, sz - HT],
                    [0.0, 0.0, -1.0],
                );
            }
            if !attached.contains(&(k, S)) {
                out.quad(
                    [sx + HT, y0, sz + HT],
                    [sx - HT, y0, sz + HT],
                    [sx - HT, y1, sz + HT],
                    [sx + HT, y1, sz + HT],
                    [0.0, 0.0, 1.0],
                );
            }
        }
    }

    (out.verts, out.idx)
}

/// A junction node's miter square centre in template-local space.
pub fn square_center(n: &WallNode) -> (f32, f32) {
    (n.offset.x + n.ns.unwrap_or(0.0), n.offset.z + n.ew.unwrap_or(0.0))
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Ew,
    Ns,
}

/// Emit one run's band segments: split around junction squares, cap at
/// free ends (building ends and door jambs), and record which square
/// sides the segments abut.
fn emit_axis_run<'a>(
    out: &mut MeshOut,
    run: &[&WallNode],
    axis: Axis,
    node_at: &dyn Fn(f32, f32) -> Option<&'a WallNode>,
    is_junction: &dyn Fn(&WallNode) -> bool,
    attached: &mut std::collections::HashSet<((i64, i64), u8)>,
) {
    let key = |x: f32, z: f32| -> (i64, i64) { (x.round() as i64, z.round() as i64) };
    // Along-axis coordinate of a node.
    let along = |n: &WallNode| match axis {
        Axis::Ew => n.offset.x,
        Axis::Ns => n.offset.z,
    };
    // Square centre along the run axis.
    let sq_along = |n: &WallNode| match axis {
        Axis::Ew => n.offset.x + n.ns.unwrap_or(0.0),
        Axis::Ns => n.offset.z + n.ew.unwrap_or(0.0),
    };
    // The run's lateral centreline.
    let lateral = match axis {
        Axis::Ew => run[0].offset.z + run[0].ew.unwrap(),
        Axis::Ns => run[0].offset.x + run[0].ns.unwrap(),
    };
    // Does the run's line continue past this end node as a DOOR (a
    // wall-line cell with no same-axis offset)? Then a jamb segment
    // extends from the square to the cell edge. A neighbor carrying a
    // *different* lateral offset is a separate run that closes itself
    // (a jog); no jamb, and the square keeps its own face there.
    let jamb_past = |n: &WallNode, dir: f32| -> bool {
        let (nx, nz) = match axis {
            Axis::Ew => (n.offset.x + dir * CELL, n.offset.z),
            Axis::Ns => (n.offset.x, n.offset.z + dir * CELL),
        };
        match node_at(nx, nz) {
            Some(nb) => match axis {
                Axis::Ew => nb.ew.is_none(),
                Axis::Ns => nb.ns.is_none(),
            },
            None => false,
        }
    };
    // Attachment side codes for this axis: a segment west of a square
    // abuts the square's W side, etc.
    let (side_lo, side_hi) = match axis {
        Axis::Ew => (0u8, 1u8), // W, E
        Axis::Ns => (2u8, 3u8), // N, S
    };

    // Build segment boundaries: [lo, hi] intervals between squares.
    struct Seg {
        lo: f32,
        hi: f32,
        cap_lo: bool,
        cap_hi: bool,
    }
    let mut segs: Vec<Seg> = Vec::new();
    let first = run[0];
    let last = run[run.len() - 1];
    let mut cursor: f32;
    let mut cursor_cap: bool;
    if is_junction(first) {
        let k = key(first.offset.x, first.offset.z);
        // Jamb stub beyond the square toward a continuing door.
        if jamb_past(first, -1.0) {
            segs.push(Seg {
                lo: along(first) - HC,
                hi: sq_along(first) - HT,
                cap_lo: true,
                cap_hi: false,
            });
            attached.insert((k, side_lo));
        }
        cursor = sq_along(first) + HT;
        cursor_cap = false;
        attached.insert((k, side_hi));
    } else {
        cursor = along(first) - HC;
        cursor_cap = true;
    }
    for n in run.iter().skip(1).take(run.len().saturating_sub(2)) {
        if is_junction(n) {
            let k = key(n.offset.x, n.offset.z);
            segs.push(Seg { lo: cursor, hi: sq_along(n) - HT, cap_lo: cursor_cap, cap_hi: false });
            attached.insert((k, side_lo));
            attached.insert((k, side_hi));
            cursor = sq_along(n) + HT;
            cursor_cap = false;
        }
    }
    if run.len() > 1 && is_junction(last) {
        let k = key(last.offset.x, last.offset.z);
        segs.push(Seg { lo: cursor, hi: sq_along(last) - HT, cap_lo: cursor_cap, cap_hi: false });
        attached.insert((k, side_lo));
        if jamb_past(last, 1.0) {
            segs.push(Seg {
                lo: sq_along(last) + HT,
                hi: along(last) + HC,
                cap_lo: false,
                cap_hi: true,
            });
            attached.insert((k, side_hi));
        }
    } else if run.len() == 1 && is_junction(first) {
        // Single junction node (a stub): the forward interval from the
        // square to the cell edge, if the line continues (jamb), was
        // handled for the lo side above; handle the hi side here.
        if jamb_past(first, 1.0) {
            segs.push(Seg {
                lo: sq_along(first) + HT,
                hi: along(first) + HC,
                cap_lo: false,
                cap_hi: true,
            });
        } else {
            // Nothing beyond the square: retract the attachment claim
            // so the square closes its own face.
            let k = key(first.offset.x, first.offset.z);
            attached.remove(&(k, side_hi));
        }
    } else {
        segs.push(Seg { lo: cursor, hi: along(last) + HC, cap_lo: cursor_cap, cap_hi: true });
    }

    for s in segs {
        if s.hi - s.lo <= 1e-3 {
            continue;
        }
        // Sub-split the seg into stretches of consecutive same-kind
        // cells: solid stretches occupy all three Y bands, window
        // stretches only the outer two (the glass band stays open for
        // the alpha pass). Stretch boundaries are cell edges.
        let mut stretches: Vec<(f32, f32, bool)> = Vec::new();
        for n in run {
            let c0 = (along(n) - HC).max(s.lo);
            let c1 = (along(n) + HC).min(s.hi);
            if c1 - c0 <= 1e-3 {
                continue;
            }
            let win = n.kind == cdda::WallCellKind::Window;
            match stretches.last_mut() {
                Some((_, hi, w)) if *w == win && (*hi - c0).abs() < 1e-3 => *hi = c1,
                _ => stretches.push((c0, c1, win)),
            }
        }
        for i in 0..stretches.len() {
            let (lo, hi, win) = stretches[i];
            let span = Span {
                lo,
                hi,
                window: win,
                cap_lo: i == 0 && s.cap_lo,
                cap_hi: i == stretches.len() - 1 && s.cap_hi,
            };
            emit_stretch(out, axis, lateral, span);
            // Reveal at a solid|window boundary: the band-1 face that
            // closes the solid side, facing into the glass opening.
            if i > 0 && stretches[i - 1].2 != win {
                emit_reveal(out, axis, lateral, lo, win);
            }
        }
    }
}

/// One same-kind interval of a run in along-axis coordinates.
struct Span {
    lo: f32,
    hi: f32,
    window: bool,
    cap_lo: bool,
    cap_hi: bool,
}

/// One same-kind stretch of a run: banded side faces (all three Y
/// bands for solid, outer two for a window), a top cap, sill top /
/// lintel underside for windows, and optional banded end caps.
fn emit_stretch(out: &mut MeshOut, axis: Axis, lateral: f32, span: Span) {
    let Span { lo, hi, window, cap_lo, cap_hi } = span;
    let h = WALL_HEIGHT;
    let solid_bands: &[(f32, f32)] = &Y_BANDS;
    let window_bands: &[(f32, f32)] = &[Y_BANDS[0], Y_BANDS[2]];
    let bands = if window { window_bands } else { solid_bands };
    match axis {
        Axis::Ew => {
            let (z0, z1) = (lateral - HT, lateral + HT);
            for &(y0, y1) in bands {
                // North face (−z) and south face (+z).
                out.quad([lo, y0, z0], [hi, y0, z0], [hi, y1, z0], [lo, y1, z0], [0.0, 0.0, -1.0]);
                out.quad([hi, y0, z1], [lo, y0, z1], [lo, y1, z1], [hi, y1, z1], [0.0, 0.0, 1.0]);
                if cap_lo {
                    out.quad([lo, y0, z1], [lo, y0, z0], [lo, y1, z0], [lo, y1, z1], [-1.0, 0.0, 0.0]);
                }
                if cap_hi {
                    out.quad([hi, y0, z0], [hi, y0, z1], [hi, y1, z1], [hi, y1, z0], [1.0, 0.0, 0.0]);
                }
            }
            // Top.
            out.quad([lo, h, z0], [hi, h, z0], [hi, h, z1], [lo, h, z1], [0.0, 1.0, 0.0]);
            if window {
                // Sill top surface (+y) and lintel underside (−y).
                out.quad(
                    [lo, SILL_TOP, z0],
                    [hi, SILL_TOP, z0],
                    [hi, SILL_TOP, z1],
                    [lo, SILL_TOP, z1],
                    [0.0, 1.0, 0.0],
                );
                out.quad(
                    [hi, HEAD_BOTTOM, z0],
                    [lo, HEAD_BOTTOM, z0],
                    [lo, HEAD_BOTTOM, z1],
                    [hi, HEAD_BOTTOM, z1],
                    [0.0, -1.0, 0.0],
                );
            }
        }
        Axis::Ns => {
            let (x0, x1) = (lateral - HT, lateral + HT);
            for &(y0, y1) in bands {
                // West face (−x) and east face (+x).
                out.quad([x0, y0, hi], [x0, y0, lo], [x0, y1, lo], [x0, y1, hi], [-1.0, 0.0, 0.0]);
                out.quad([x1, y0, lo], [x1, y0, hi], [x1, y1, hi], [x1, y1, lo], [1.0, 0.0, 0.0]);
                if cap_lo {
                    out.quad([x0, y0, lo], [x1, y0, lo], [x1, y1, lo], [x0, y1, lo], [0.0, 0.0, -1.0]);
                }
                if cap_hi {
                    out.quad([x1, y0, hi], [x0, y0, hi], [x0, y1, hi], [x1, y1, hi], [0.0, 0.0, 1.0]);
                }
            }
            out.quad([x0, h, lo], [x1, h, lo], [x1, h, hi], [x0, h, hi], [0.0, 1.0, 0.0]);
            if window {
                out.quad(
                    [x0, SILL_TOP, lo],
                    [x1, SILL_TOP, lo],
                    [x1, SILL_TOP, hi],
                    [x0, SILL_TOP, hi],
                    [0.0, 1.0, 0.0],
                );
                out.quad(
                    [x1, HEAD_BOTTOM, lo],
                    [x0, HEAD_BOTTOM, lo],
                    [x0, HEAD_BOTTOM, hi],
                    [x1, HEAD_BOTTOM, hi],
                    [0.0, -1.0, 0.0],
                );
            }
        }
    }
}

/// The band-1 face at a solid|window boundary — closes the solid
/// stretch's cross-section and faces into the glass opening.
/// `facing_positive` = the window lies on the +axis side of `at`.
fn emit_reveal(out: &mut MeshOut, axis: Axis, lateral: f32, at: f32, facing_positive: bool) {
    let (y0, y1) = Y_BANDS[1];
    match axis {
        Axis::Ew => {
            let (z0, z1) = (lateral - HT, lateral + HT);
            if facing_positive {
                out.quad([at, y0, z0], [at, y0, z1], [at, y1, z1], [at, y1, z0], [1.0, 0.0, 0.0]);
            } else {
                out.quad([at, y0, z1], [at, y0, z0], [at, y1, z0], [at, y1, z1], [-1.0, 0.0, 0.0]);
            }
        }
        Axis::Ns => {
            let (x0, x1) = (lateral - HT, lateral + HT);
            if facing_positive {
                out.quad([x1, y0, at], [x0, y0, at], [x0, y1, at], [x1, y1, at], [0.0, 0.0, 1.0]);
            } else {
                out.quad([x0, y0, at], [x1, y0, at], [x1, y1, at], [x0, y1, at], [0.0, 0.0, -1.0]);
            }
        }
    }
}

#[derive(Default)]
struct MeshOut {
    verts: Vec<MeshVertex>,
    idx: Vec<u32>,
}

impl MeshOut {
    /// Push a quad as two CCW triangles. `a..d` wind counter-clockwise
    /// seen from the normal side. UVs are planar in the quad's plane
    /// (world units / CELL).
    fn quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3], n: [f32; 3]) {
        let base = self.verts.len() as u32;
        let uv = |p: [f32; 3]| -> [f32; 2] {
            if n[1].abs() > 0.5 {
                [p[0] / CELL, p[2] / CELL]
            } else if n[0].abs() > 0.5 {
                [p[2] / CELL, p[1] / CELL]
            } else {
                [p[0] / CELL, p[1] / CELL]
            }
        };
        for p in [a, b, c, d] {
            self.verts.push(MeshVertex { pos: p, normal: n, uv: uv(p) });
        }
        self.idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_math::Vec3;
    use cdda::{WallCellKind, WallEdge, WallGraph};
    use std::collections::HashMap;

    fn node(x: f32, z: f32, ew: Option<f32>, ns: Option<f32>) -> WallNode {
        WallNode {
            offset: Vec3::new(x, 0.0, z),
            kind: WallCellKind::Solid,
            color: Some([0.5, 0.4, 0.3]),
            ew,
            ns,
        }
    }

    /// Every positional edge must be shared by exactly two triangles,
    /// except edges lying on the ground (y = 0 on both ends) — walls
    /// are open at the bottom, they sit on the terrain.
    fn assert_manifold_or_grounded(verts: &[MeshVertex], idx: &[u32]) {
        let q = |v: f32| (v * 8.0).round() as i64;
        let pkey = |p: [f32; 3]| (q(p[0]), q(p[1]), q(p[2]));
        let mut edge_count: HashMap<((i64, i64, i64), (i64, i64, i64)), usize> = HashMap::new();
        let mut edge_ground: HashMap<((i64, i64, i64), (i64, i64, i64)), bool> = HashMap::new();
        for t in idx.chunks(3) {
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                let a = verts[t[i] as usize].pos;
                let b = verts[t[j] as usize].pos;
                let (ka, kb) = (pkey(a), pkey(b));
                let e = if ka <= kb { (ka, kb) } else { (kb, ka) };
                *edge_count.entry(e).or_default() += 1;
                edge_ground.insert(e, a[1].abs() < 1e-3 && b[1].abs() < 1e-3);
            }
        }
        // Quad diagonals appear twice within their own quad (the two
        // triangles share them) — that's fine. What must never happen:
        // an edge with count 1 that isn't on the ground (a hole), or
        // count > 2 (a non-manifold fan / doubled face).
        for (e, n) in &edge_count {
            let grounded = edge_ground[e];
            assert!(
                *n == 2 || (*n == 1 && grounded),
                "edge {e:?} has {n} incident triangles (grounded={grounded})"
            );
        }
    }

    fn assert_no_faces_inside_square(
        verts: &[MeshVertex],
        idx: &[u32],
        cx: f32,
        cz: f32,
    ) {
        for t in idx.chunks(3) {
            let c = t.iter().fold([0.0f32; 3], |acc, &i| {
                let p = verts[i as usize].pos;
                [acc[0] + p[0] / 3.0, acc[1] + p[1] / 3.0, acc[2] + p[2] / 3.0]
            });
            // Strictly inside the OPEN volume — faces on the square's
            // boundary (its top cap at y = H, its side faces) are the
            // legitimate surface; what must never exist is geometry
            // buried inside the solid (the old end-caps-in-the-joint).
            let inside = (c[0] - cx).abs() < HT - 0.5
                && (c[2] - cz).abs() < HT - 0.5
                && c[1] > 0.5
                && c[1] < WALL_HEIGHT - 0.5;
            assert!(
                !inside,
                "face centroid {c:?} inside the miter square at ({cx}, {cz})"
            );
        }
    }

    #[test]
    fn straight_run_is_one_capped_band() {
        // Three cells in a row, north-perimeter offset (−28).
        let g = WallGraph {
            nodes: vec![
                node(0.0, 0.0, Some(-28.0), None),
                node(80.0, 0.0, Some(-28.0), None),
                node(160.0, 0.0, Some(-28.0), None),
            ],
            edges: vec![WallEdge { a: 0, b: 1 }, WallEdge { a: 1, b: 2 }],
        };
        let (verts, idx) = wall_graph_mesh(&g);
        assert!(!verts.is_empty() && idx.len() % 3 == 0);
        // The band spans the full three cells (−40..200), 24 thick at
        // z = −28 ± 12, and reaches the wall height.
        for v in &verts {
            assert!(v.pos[0] >= -40.0 - 1e-3 && v.pos[0] <= 200.0 + 1e-3, "x out of run: {:?}", v.pos);
            assert!(v.pos[2] >= -40.0 - 1e-3 && v.pos[2] <= -16.0 + 1e-3, "z out of band: {:?}", v.pos);
        }
        assert!(verts.iter().any(|v| (v.pos[0] + 40.0).abs() < 1e-3), "west cap missing");
        assert!(verts.iter().any(|v| (v.pos[0] - 200.0).abs() < 1e-3), "east cap missing");
        assert!(verts.iter().any(|v| (v.pos[1] - WALL_HEIGHT).abs() < 1e-3), "no top");
        assert_manifold_or_grounded(&verts, &idx);
        // Normals are unit length.
        for v in &verts {
            let l = (v.normal[0].powi(2) + v.normal[1].powi(2) + v.normal[2].powi(2)).sqrt();
            assert!((l - 1.0).abs() < 1e-4);
        }
    }

    #[test]
    fn l_corner_is_manifold_with_no_faces_inside_the_miter() {
        // North perimeter coming from the west, turning south at the
        // corner cell (origin): the corner node carries both offsets.
        let g = WallGraph {
            nodes: vec![
                node(-80.0, 0.0, Some(-28.0), None),      // west run cell
                node(0.0, 0.0, Some(-28.0), Some(-28.0)), // corner
                node(0.0, 80.0, None, Some(-28.0)),       // south run cell
            ],
            edges: vec![WallEdge { a: 0, b: 1 }, WallEdge { a: 1, b: 2 }],
        };
        let (verts, idx) = wall_graph_mesh(&g);
        assert!(!verts.is_empty());
        assert_manifold_or_grounded(&verts, &idx);
        // The miter square sits at (−28, −28); no face may live inside it.
        assert_no_faces_inside_square(&verts, &idx, -28.0, -28.0);
        // The outer corner is a real welded position: (−40, −40) exists
        // (the square's outer corner = the building's outer corner).
        assert!(
            verts.iter().any(|v| (v.pos[0] + 40.0).abs() < 1e-3 && (v.pos[2] + 40.0).abs() < 1e-3),
            "outer corner vertex missing"
        );
        // No stray end-caps between run and square: no vertex sits on
        // the interface plane x = −40..−16 at z beyond the band.
        for v in &verts {
            assert!(v.pos[2] >= -40.0 - 1e-3, "geometry north of the wall outer face: {:?}", v.pos);
        }
    }

    #[test]
    fn t_junction_splits_the_through_run_and_stays_manifold() {
        // EW run through three cells; NS divider attaches from the
        // south at the middle cell (centred, ns = 0).
        let g = WallGraph {
            nodes: vec![
                node(-80.0, 0.0, Some(-28.0), None),
                node(0.0, 0.0, Some(-28.0), Some(0.0)), // T-junction
                node(80.0, 0.0, Some(-28.0), None),
                node(0.0, 80.0, None, Some(0.0)), // divider cell below
            ],
            edges: vec![
                WallEdge { a: 0, b: 1 },
                WallEdge { a: 1, b: 2 },
                WallEdge { a: 1, b: 3 },
            ],
        };
        let (verts, idx) = wall_graph_mesh(&g);
        assert_manifold_or_grounded(&verts, &idx);
        // Miter square at (0, −28).
        assert_no_faces_inside_square(&verts, &idx, 0.0, -28.0);
        // The through-run splits at the square: interface planes at
        // x = ±12 both exist.
        assert!(verts.iter().any(|v| (v.pos[0] + 12.0).abs() < 1e-3));
        assert!(verts.iter().any(|v| (v.pos[0] - 12.0).abs() < 1e-3));
        // The divider band runs from the square's south face down to
        // the divider cell's south edge.
        assert!(verts.iter().any(|v| (v.pos[2] - 120.0).abs() < 1e-3), "divider band missing");
    }

    /// No triangle centroid strictly inside the open glass volume of a
    /// window cell — the glass band belongs to the alpha pass, the wall
    /// mesh must leave it open. Reveals live ON the boundary planes.
    fn assert_glass_band_open(verts: &[MeshVertex], idx: &[u32], n: &WallNode) {
        for t in idx.chunks(3) {
            let c = t.iter().fold([0.0f32; 3], |acc, &i| {
                let p = verts[i as usize].pos;
                [acc[0] + p[0] / 3.0, acc[1] + p[1] / 3.0, acc[2] + p[2] / 3.0]
            });
            let inside = (c[0] - n.offset.x).abs() < CDDA_HALF - 1.0
                && (c[2] - n.offset.z).abs() < CDDA_HALF - 1.0
                && c[1] > SILL_TOP + 0.5
                && c[1] < HEAD_BOTTOM - 0.5;
            assert!(
                !inside,
                "face centroid {c:?} inside the glass band of the window at {:?}",
                n.offset
            );
        }
    }

    #[test]
    fn window_opens_the_glass_band_and_stays_manifold() {
        // Three-cell perimeter run, middle cell is a window: the wall
        // mesh emits sill (0..60) and lintel (180..220) but leaves the
        // glass band open, with reveals closing the solid neighbours.
        let mut mid = node(80.0, 0.0, Some(-28.0), None);
        mid.kind = WallCellKind::Window;
        let g = WallGraph {
            nodes: vec![
                node(0.0, 0.0, Some(-28.0), None),
                mid,
                node(160.0, 0.0, Some(-28.0), None),
            ],
            edges: vec![WallEdge { a: 0, b: 1 }, WallEdge { a: 1, b: 2 }],
        };
        let (verts, idx) = wall_graph_mesh(&g);
        assert_manifold_or_grounded(&verts, &idx);
        assert_glass_band_open(&verts, &idx, &g.nodes[1]);
        // Sill top surface and lintel underside exist.
        assert!(
            verts.iter().any(|v| (v.pos[1] - SILL_TOP).abs() < 1e-3),
            "sill top missing"
        );
        assert!(
            verts.iter().any(|v| (v.pos[1] - HEAD_BOTTOM).abs() < 1e-3),
            "lintel underside missing"
        );
        // Reveals: geometry at the window's side boundaries within the
        // glass band (x = 40 and 120, between y 60 and 180).
        assert!(
            verts.iter().any(|v| (v.pos[0] - 40.0).abs() < 1e-3
                && v.pos[1] > SILL_TOP - 1e-3
                && v.pos[1] < HEAD_BOTTOM + 1e-3),
            "west reveal missing"
        );
        // Coplanarity: every side-face vertex lies in one of the run's
        // two side planes (z = −40 or z = −16); nothing bulges.
        for v in &verts {
            if v.normal[2].abs() > 0.5 {
                let on_planes =
                    (v.pos[2] + 40.0).abs() < 1e-3 || (v.pos[2] + 16.0).abs() < 1e-3;
                assert!(on_planes, "side-face vertex off the run planes: {:?}", v.pos);
            }
        }
    }

    #[test]
    fn p_shape_meshes_manifold_with_clean_miters_and_door_gaps() {
        // End-to-end: the real graph from the cdda importer.
        let json = r#"[{
            "om_terrain": "p_shape",
            "object": {
                "rows": [
                    "wwwww",
                    "o d w",
                    "wdwdw",
                    "  w w",
                    "  w o",
                    "  www"
                ],
                "terrain": {
                    "w": "t_wall",
                    "o": "t_window",
                    "d": "t_door_c"
                }
            }
        }]"#;
        let t = cdda::mapgen_to_template(json, "p_shape", cdda::CDDA_TILE, 0).unwrap();
        let (verts, idx) = wall_graph_mesh(&t.walls);
        assert!(!verts.is_empty());
        assert_manifold_or_grounded(&verts, &idx);
        // Every junction square is clean inside.
        for n in t.walls.nodes.iter().filter(|n| n.ew.is_some() && n.ns.is_some()) {
            let (cx, cz) = square_center(n);
            assert_no_faces_inside_square(&verts, &idx, cx, cz);
        }
        // Windows are open in the glass band, on the real graph too.
        for n in t.walls.nodes.iter().filter(|n| n.kind == WallCellKind::Window) {
            assert_glass_band_open(&verts, &idx, n);
        }
        // Door cells are gaps: no geometry strictly inside a door
        // cell's interior (jambs live ON the cell boundary).
        for n in t.walls.nodes.iter().filter(|n| n.kind == WallCellKind::Door) {
            for tch in idx.chunks(3) {
                let c = tch.iter().fold([0.0f32; 3], |acc, &i| {
                    let p = verts[i as usize].pos;
                    [acc[0] + p[0] / 3.0, acc[1] + p[1] / 3.0, acc[2] + p[2] / 3.0]
                });
                let inside = (c[0] - n.offset.x).abs() < CDDA_HALF - 1.0
                    && (c[2] - n.offset.z).abs() < CDDA_HALF - 1.0;
                assert!(
                    !inside,
                    "face centroid {c:?} inside door cell at {:?}",
                    n.offset
                );
            }
        }
    }
}
