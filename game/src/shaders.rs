//! game/src/shaders — WGSL shader source for every render pipeline
//! (UI overlay, ghost, glass, mesh trunks, leaf cards, and the world
//! cube). Kept in one module so the shared instance/vertex ABI stays a
//! single source across the native (render.rs) and wasm (render_web.rs)
//! paths.

/// UI overlay shader — draws screen-space quads. Vertices computed
/// from vertex_index (6 per instance, two triangles). Instance data
/// is the quad center + half-size in NDC + color + alpha. Shares
/// the same bind group layout as the world pipeline for a
/// no-branch pipeline-layout reuse; the camera uniform is
/// declared here so the pipeline validates, but unused.
pub const UI_SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> _camera: Camera;

struct UiInstance {
    @location(0) center_ndc: vec2<f32>,
    @location(1) half_size_ndc: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) alpha: f32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: UiInstance) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    let corner = corners[vi];
    let ndc = inst.center_ndc + corner * inst.half_size_ndc;
    var o: VOut;
    o.clip = vec4<f32>(ndc, 0.0, 1.0);
    o.color = vec4<f32>(inst.color, inst.alpha);
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// Ghost pass shader — cut-away walls + roof render here at low alpha
/// instead of vanishing, so the player still sees the outline of the
/// building they're inside. Same vertex + instance layout as the world
/// cube shader; distinct pipeline from glass so its alpha (and future
/// per-instance tuning) evolves separately.
pub const GHOST_SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct IIn {
    @location(2) i_pos: vec3<f32>,
    @location(3) i_color: vec3<f32>,
    @location(4) i_scale: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(v.normal);
    o.color = i.i_color;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;
/// Deliberately faint — the ghost is a hint of the wall's outline,
/// not a wall you look through.
const GHOST_ALPHA: f32 = 0.15;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, GHOST_ALPHA);
}
"#;

/// Glass pass shader — identical geometry to the world cube shader, but
/// the fragment emits a low constant alpha so windows read as real
/// see-through glass. Drawn after the opaque world with alpha blending,
/// depth-tested against the world (so glass behind a wall is occluded)
/// but not depth-writing (so overlapping panes blend). Same vertex +
/// instance layout as SHADER_WGSL, so it reuses the cube geometry and
/// SceneInstance buffer.
pub const GLASS_SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct IIn {
    @location(2) i_pos: vec3<f32>,
    @location(3) i_color: vec3<f32>,
    @location(4) i_scale: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(v.normal);
    o.color = i.i_color;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;
const GLASS_ALPHA: f32 = 0.34;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    // Pre-multiply by alpha? No — the pipeline uses straight-alpha
    // blending (src-alpha, one-minus-src-alpha), so emit straight RGBA.
    return vec4<f32>(in.color * k, GLASS_ALPHA);
}
"#;

/// The shared LAYOUT every mesh pipeline binds — Camera + binding (with
/// the `wind` slot), the (pos, normal, uv) vertex layout at 0/1/2, the
/// (i_pos, i_color, i_scale, i_axis) instance layout at 3/4/5/6,
/// `basis_from_axis`, and the light consts. `MESH_SHADER_WGSL` and
/// `LEAF_SHADER_WGSL` BOTH begin with this, so the instance/vertex ABI
/// can never drift between the two pipelines — the real cross-pipeline
/// hazard (a mistyped location points limbs the wrong way in one
/// pipeline only). The per-pipeline VERTEX stage differs only in HOW MUCH
/// each instance sways: both call the shared `wind_offset`, but rigidity
/// is carried PER INSTANCE in `i_axis.w` (the sway weight) — a trunk sets
/// it to 0 and stands still, a thin twig sets it near 1 and flutters, a
/// leaf inherits its twig's weight so foliage and branch move together.
/// `i_axis.xyz` is the orientation; `i_axis.w` the sway weight. `wind.x`
/// is elapsed seconds (synthetic ticks — no `bevy_time`, same model as
/// the campfire flicker); `.yzw` spare. The vertex `uv` is the day-one
/// slot for downstream damage/bark textures.
macro_rules! mesh_layout_wgsl {
    () => {
        r#"
struct Camera { view_proj: mat4x4<f32>, wind: vec4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct IIn {
    @location(3) i_pos: vec3<f32>,
    @location(4) i_color: vec3<f32>,
    @location(5) i_scale: vec3<f32>,
    @location(6) i_axis: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

// Rotation mapping the local +Y axis onto `axis`. A cone/cylinder is
// symmetric about its length axis, so roll is irrelevant — any rotation
// taking +Y to the (unit) axis orients the limb correctly. Columns
// (right, up, fwd) → local (x,y,z) map to (right, axis, fwd); axis = +Y
// yields the identity, so trunks (axis = +Y) and leaf spheres pass
// through unchanged.
fn basis_from_axis(axis: vec3<f32>) -> mat3x3<f32> {
    let up = normalize(axis);
    let refv = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(up.z) > 0.9);
    let right = normalize(cross(up, refv));
    let fwd = cross(right, up);
    return mat3x3<f32>(right, up, fwd);
}

// Horizontal wind sway at a world point. ONE shared function so branch
// tips and the leaves anchored to them move in lockstep (same phase from
// the same world position, same amplitude) — no drifting-apart foliage.
// `amp` is world units; callers scale it by the instance's sway weight.
// `speed` scales the sinusoid's temporal frequency; 1.0 = the original
// pace. `amp` is world units; callers scale it by the instance's sway
// weight.
fn wind_offset(world: vec3<f32>, t: f32, amp: f32, speed: f32) -> vec3<f32> {
    let phase = world.x * 0.010 + world.z * 0.013;
    let ts = t * speed;
    return vec3<f32>(amp * sin(ts * 1.7 + phase), 0.0, amp * 0.6 * sin(ts * 1.3 + phase * 1.7));
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;
"#
    };
}

/// The standard mesh VERTEX stage — scale in the local +Y frame,
/// rotate onto the instance axis, translate, sway by the instance
/// weight pivoting at the base. Shared verbatim by the mesh (trees),
/// wall, and wall-ghost pipelines; the leaf pipeline has its own vs
/// (full-weight sway, no base taper).
macro_rules! mesh_standard_vs_wgsl {
    () => {
        r#"
@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let rot = basis_from_axis(i.i_axis.xyz);
    var world = rot * (v.pos * i.i_scale) + i.i_pos;
    world = world + wind_offset(world, camera.wind.x, camera.wind.y * i.i_axis.w * v.pos.y, camera.wind.z);
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(rot * (v.normal / i.i_scale));
    o.color = i.i_color;
    o.uv = v.uv;
    return o;
}
"#
    };
}

/// Mesh pipeline shader for trunks + branch cones — the shared layout, a
/// vertex stage that sways each limb by its sway weight PIVOTING AT THE
/// BASE (thin twigs bend most at their tip, the trunk's weight is 0 so it
/// stays rigid), and an opaque single-sided Lambert fragment. Depth-write
/// ON (set on the pipeline) so mesh geometry occludes correctly.
pub const MESH_SHADER_WGSL: &str = concat!(
    mesh_layout_wgsl!(),
    mesh_standard_vs_wgsl!(),
    r#"
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    // Procedural bark: vertical furrows around the stem (u wraps once per
    // revolution) with lengthwise grain (v), darkening the grooves. Cheap
    // and self-generated from the day-one UV — no texture image, no
    // sampler, no new env.* crossing. This is the first real material on
    // the mesh substrate; a sampled bark/brick image is the next step up
    // (and the walls-on-mesh prerequisite).
    let furrow = 0.5 + 0.5 * sin(in.uv.x * 6.2831853 * 7.0);
    let grain = 0.88 + 0.12 * sin(in.uv.y * 40.0 + furrow * 4.0);
    let bark = mix(0.70, 1.0, furrow) * grain;
    return vec4<f32>(in.color * k * bark, 1.0);
}
"#
);

/// Wall shader for the walls-on-mesh buildings (RENDER.md slice 4).
/// Same layout + vertex stage as the mesh pipeline (identity axis,
/// sway weight 0 → rigid), but the fragment is a plain Lambert with a
/// whisper of plaster grain from the day-one UV — walls must NOT get
/// the tree shader's procedural bark furrows. Same pipeline factory,
/// no new env.* crossing.
pub const WALL_SHADER_WGSL: &str = concat!(
    mesh_layout_wgsl!(),
    mesh_standard_vs_wgsl!(),
    r#"
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    // Higher ambient than the tree shader: walls are mostly vertical,
    // and a near-overhead light leaves vertical faces too dim to read
    // material or room colour.
    let k = 0.42 + 0.58 * ndotl;
    let grain = 0.97 + 0.03 * sin(in.uv.x * 11.0) * sin(in.uv.y * 8.0);
    return vec4<f32>(in.color * k * grain, 1.0);
}
"#
);

/// Ghost fragment for cut-away WALL MESH geometry — the mesh-layout
/// sibling of GHOST_SHADER_WGSL: same faint alpha, so a wall that the
/// cut-away removed still leaves its outline instead of silently
/// vanishing. Drawn with the ghost mesh pipeline (alpha blend, depth
/// test on, depth write off).
pub const WALL_GHOST_SHADER_WGSL: &str = concat!(
    mesh_layout_wgsl!(),
    mesh_standard_vs_wgsl!(),
    r#"
const GHOST_ALPHA: f32 = 0.15;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = 0.42 + 0.58 * ndotl;
    return vec4<f32>(in.color * k, GHOST_ALPHA);
}
"#
);

/// Ground shader for the solid terrain surface. Same vertex/instance
/// layout as the mesh pipeline (so it uses the same pipeline factory and
/// buffers), but it carries WORLD XZ to the fragment and paints a faint,
/// WORLD-ANCHORED reference grid there — major lines every 80 units (the
/// CDDA cell), minor every 40. This replaces the old draped-bar dev-grid:
/// the grid is now free fragment math, fixed to the world (not centred on
/// the player), and always present on the ground.
pub const GROUND_SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32>, wind: vec4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};
struct IIn {
    @location(3) i_pos: vec3<f32>,
    @location(4) i_color: vec3<f32>,
    @location(5) i_scale: vec3<f32>,
    @location(6) i_axis: vec4<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) world_xz: vec2<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    // The surface is drawn with an identity instance, so world = v.pos.
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(v.normal / i.i_scale);
    o.color = i.i_color;
    o.world_xz = world.xz;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;

// Distance (world units) from `coord` to the nearest line at `spacing`.
fn line_dist(coord: f32, spacing: f32) -> f32 {
    return abs(fract(coord / spacing - 0.5) - 0.5) * spacing;
}
// Anti-aliased line intensity: ~1 on the line, fading over one pixel.
fn line_at(coord: f32, spacing: f32) -> f32 {
    let d = line_dist(coord, spacing);
    let w = fwidth(coord);
    return 1.0 - smoothstep(0.0, w, d);
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    var col = in.color * k;
    // World-anchored grid: major at 80, minor offset by 40.
    let major = max(line_at(in.world_xz.x, 80.0), line_at(in.world_xz.y, 80.0));
    let minor = max(line_at(in.world_xz.x + 40.0, 80.0), line_at(in.world_xz.y + 40.0, 80.0));
    let grid_col = vec3<f32>(0.55, 0.60, 0.68);
    col = mix(col, grid_col, major * 0.30 + minor * 0.14);
    return vec4<f32>(col, 1.0);
}
"#;

/// Mesh pipeline shader for LEAF cards — the SAME shared layout as
/// `MESH_SHADER_WGSL` (so the instance/vertex ABI is one source), a
/// vertex stage that adds WIND SWAY, and a fragment that carves a leaf
/// silhouette out of the quad via its UV and lights it two-sided.
/// Trunks/branches keep MESH_SHADER; only the canopy draw uses this.
pub const LEAF_SHADER_WGSL: &str = concat!(
    mesh_layout_wgsl!(),
    r#"
@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let rot = basis_from_axis(i.i_axis.xyz);
    var world = rot * (v.pos * i.i_scale) + i.i_pos;
    // Wind: a leaf is a point at its twig's tip, so it sways by the FULL
    // weight (no v.pos.y taper) — and a leaf carries its twig's sway
    // weight, so leaf and branch tip share one `wind_offset` at the same
    // world point and move in lockstep, never drifting apart.
    world = world + wind_offset(world, camera.wind.x, camera.wind.y * camera.wind.w * i.i_axis.w, camera.wind.z);
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(rot * (v.normal / i.i_scale));
    o.color = i.i_color;
    o.uv = v.uv;
    return o;
}

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    // Procedural leaf silhouette: a pointed-oval (almond) mask — widest
    // at v=0.5, tapering to points at v=0 and v=1. Discard outside so a
    // rectangular card renders as a leaf outline; the card's aspect
    // (instance scale) makes it a broad blade or a slim needle.
    let half_w = 0.5 * sin(3.14159265 * in.uv.y);
    if (abs(in.uv.x - 0.5) > half_w) { discard; }
    let l = normalize(LIGHT_DIR);
    // Two-sided: a leaf catches light on whichever face meets the sun.
    let ndotl = abs(dot(normalize(in.normal), l));
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, 1.0);
}
"#
);

pub const SHADER_WGSL: &str = r#"
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct IIn {
    @location(2) i_pos: vec3<f32>,
    @location(3) i_color: vec3<f32>,
    @location(4) i_scale: vec3<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs(v: VIn, i: IIn) -> VOut {
    let world = v.pos * i.i_scale + i.i_pos;
    var o: VOut;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.normal = normalize(v.normal);
    o.color = i.i_color;
    return o;
}

const LIGHT_DIR: vec3<f32> = vec3<f32>(0.3, 0.85, 0.4);
const AMBIENT: f32 = 0.25;

@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let l = normalize(LIGHT_DIR);
    let ndotl = max(dot(normalize(in.normal), l), 0.0);
    let k = AMBIENT + (1.0 - AMBIENT) * ndotl;
    return vec4<f32>(in.color * k, 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trunk_fragment_paints_bark_from_the_uv() {
        // The mesh (trunk/branch) fragment now reads the day-one UV to
        // paint procedural bark — the first real material on the substrate,
        // self-generated, no texture resource. Leaves use the UV for their
        // silhouette; trunks use it for bark.
        assert!(
            MESH_SHADER_WGSL.contains("in.uv.x") && MESH_SHADER_WGSL.contains("bark"),
            "trunk fragment must derive bark from the UV"
        );
    }

    #[test]
    fn mesh_and_leaf_shaders_share_one_layout() {
        // The instance/vertex ABI (Camera, VIn/IIn/VOut, basis_from_axis,
        // light consts) must be ONE shared source, not two hand-mirrored
        // copies — a drift there points limbs the wrong way in exactly one
        // pipeline and only a seer frame would catch it. Both shaders BEGIN
        // with the shared layout; each then adds its own single vertex +
        // fragment stage (the vertex stages differ — leaves sway, trunks
        // don't — which is why the layout, not the whole vertex half, is
        // what's shared).
        let layout = mesh_layout_wgsl!();
        assert!(
            MESH_SHADER_WGSL.starts_with(layout),
            "mesh shader must begin with the shared layout"
        );
        assert!(
            LEAF_SHADER_WGSL.starts_with(layout),
            "leaf shader must begin with the shared layout"
        );
        for src in [MESH_SHADER_WGSL, LEAF_SHADER_WGSL] {
            assert_eq!(src.matches("@vertex").count(), 1);
            assert_eq!(src.matches("@fragment").count(), 1);
        }
    }

    #[test]
    fn wind_is_a_shared_offset_weighted_per_instance() {
        // Wind moves BOTH branches and leaves now; what keeps a trunk rigid
        // is its per-instance sway weight (i_axis.w = 0), not the shader
        // lacking wind. Both vertex stages must call the shared
        // `wind_offset` and scale it by `i_axis.w`, so branch tips and the
        // leaves anchored to them move in lockstep and a zero-weight trunk
        // stays put. The instance axis must be a vec4 (xyz + weight).
        for src in [MESH_SHADER_WGSL, LEAF_SHADER_WGSL] {
            assert!(src.contains("wind_offset("), "both vertex stages sway via wind_offset");
            assert!(src.contains("i_axis.w"), "sway must be weighted per instance");
            assert!(src.contains("i_axis: vec4"), "axis carries orientation + sway weight");
        }
        // Only the mesh (limb) stage pivots at the base via v.pos.y; the
        // leaf is a point and sways by the full weight.
        assert!(
            MESH_SHADER_WGSL.contains("i_axis.w * v.pos.y"),
            "limbs pivot at the base (weight × v.pos.y)"
        );
    }
}
