// Minimal forward shader for the wgpu POC.
//
// One directional light + ambient, straight LDR colour out to the
// surface. No HDR target, no bloom, no MSAA, no storage textures,
// no texture arrays — every construct here is inside the
// downlevel_webgl2 feature set, so the SAME shader runs on the
// WebGPU and WebGL2 backends. That single-source property is the
// whole point: Bevy needed compile-time shader-define gating and two
// wasm bundles; this does not.

struct Camera {
    view_proj: mat4x4<f32>,
    light_dir: vec4<f32>,   // xyz = direction toward the light
    ambient:   vec4<f32>,   // x = ambient term in [0,1]
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VsIn {
    @location(0) pos:    vec3<f32>,
    @location(1) normal: vec3<f32>,
    // Per-instance model matrix, split across four vec4 slots.
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
    @location(6) color: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color:  vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    let world = model * vec4<f32>(in.pos, 1.0);
    var out: VsOut;
    out.clip = camera.view_proj * world;
    // Uniform-ish scale assumption: rotate the normal by the model's
    // upper 3x3. Good enough for lit primitives in a POC.
    out.normal = (model * vec4<f32>(in.normal, 0.0)).xyz;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let l = normalize(camera.light_dir.xyz);
    let diff = max(dot(n, l), 0.0);
    let amb = camera.ambient.x;
    let lit = in.color * (amb + diff * (1.0 - amb));
    return vec4<f32>(lit, 1.0);
}
