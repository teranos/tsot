/// One draw-call instance. Positions are world coords; scale is per-
/// axis stretch of the unit cube (-0.5..0.5); color is linear RGB.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneInstance {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub scale: [f32; 3],
}

/// Instance for the MESH pipeline. Same as `SceneInstance` plus `axis`:
/// `xyz` is the unit direction the shader rotates the geometry's local
/// +Y onto (one baked cone → a limb pointing anywhere), and `w` is the
/// per-instance WIND SWAY weight (0 = rigid trunk, →1 = a thin twig that
/// flutters most). 52 bytes, `#[repr(C)]`: layout must match the mesh
/// WGSL's IIn (loc 3/4/5/6 at offsets 0/12/24/36) and the vertex-buffer
/// layouts on both render paths — all held to `INSTANCE_ATTRS`. A
/// vertical trunk sets `axis = [0,1,0,0]` (identity rotation, no sway).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MeshInstance {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub scale: [f32; 3],
    pub axis: [f32; 4],
}

/// One per-instance vertex attribute of `MeshInstance`.
pub struct InstanceAttr {
    /// `@location` in the WGSL and `shaderLocation` in the JS shim.
    pub location: u32,
    /// Byte offset into `MeshInstance`.
    pub offset: u64,
    /// WebGPU vertex-format name, exactly as the JS shim spells it.
    pub format: &'static str,
}

/// THE single source of truth for the `MeshInstance` vertex layout. The
/// same 48-byte record is described in four places — the `#[repr(C)]`
/// struct above (the PRODUCER of the bytes), the WGSL `@location` list,
/// the native `wgpu` attribute array (`render.rs` builds its array FROM
/// this const), and the hand-written JS shim (`web/src/main.ts`). This
/// const is what the other three must agree with; `render.rs` derives
/// from it, and `web_shim_mesh_instance_layout_matches_this_const` holds
/// the JS copy to it so the one hand-maintained descriptor can't drift
/// silently (only a real browser exercises the JS path, so nothing else
/// would catch it). No proto / codegen — the JS stays hand-inspectable,
/// a test just checks it.
pub const INSTANCE_ATTRS: &[InstanceAttr] = &[
    InstanceAttr { location: 3, offset: 0, format: "float32x3" },
    InstanceAttr { location: 4, offset: 12, format: "float32x3" },
    InstanceAttr { location: 5, offset: 24, format: "float32x3" },
    // axis is a vec4: xyz = orientation, w = wind sway weight.
    InstanceAttr { location: 6, offset: 36, format: "float32x4" },
];

/// Stride of the per-instance buffer — the size of one `MeshInstance`.
/// Passed to both render paths (`instanceStride` in JS), so it can't
/// disagree; the offsets are what need guarding.
pub const INSTANCE_STRIDE: u64 = std::mem::size_of::<MeshInstance>() as u64;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

pub fn cube_geometry() -> Vec<GpuVertex> {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),   // +Z
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), // -Z
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),  // +X
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),  // -X
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),  // +Y
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),  // -Y
    ];
    let mut out: Vec<GpuVertex> = Vec::with_capacity(36);
    for (n, u, v) in faces {
        let center = [n[0] * 0.5, n[1] * 0.5, n[2] * 0.5];
        let mk = |su: f32, sv: f32| GpuVertex {
            pos: [
                center[0] + u[0] * su * 0.5 + v[0] * sv * 0.5,
                center[1] + u[1] * su * 0.5 + v[1] * sv * 0.5,
                center[2] + u[2] * su * 0.5 + v[2] * sv * 0.5,
            ],
            normal: n,
        };
        let c00 = mk(-1.0, -1.0);
        let c10 = mk(1.0, -1.0);
        let c11 = mk(1.0, 1.0);
        let c01 = mk(-1.0, 1.0);
        out.push(c00);
        out.push(c10);
        out.push(c11);
        out.push(c00);
        out.push(c11);
        out.push(c01);
    }
    out
}

/// Reinterpret a `&[T]` where T is `#[repr(C)] Copy` as raw bytes.
pub fn as_bytes<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_attrs_match_the_repr_c_struct() {
        // The source-of-truth const must agree with the actual repr(C)
        // byte layout it claims to describe: four vec3s at 0/12/24/36 and
        // a 48-byte stride. If MeshInstance changes, this fails first.
        assert_eq!(INSTANCE_ATTRS.len(), 4);
        // pos/color/scale are vec3 at 0/12/24; axis is a vec4 at 36.
        let expected = [
            (3u32, 0u64, "float32x3"),
            (4, 12, "float32x3"),
            (5, 24, "float32x3"),
            (6, 36, "float32x4"),
        ];
        for (a, (loc, off, fmt)) in INSTANCE_ATTRS.iter().zip(expected) {
            assert_eq!(a.location, loc);
            assert_eq!(a.offset, off);
            assert_eq!(a.format, fmt);
        }
        assert_eq!(INSTANCE_STRIDE, 52);
        assert_eq!(INSTANCE_STRIDE, std::mem::size_of::<MeshInstance>() as u64);
    }

    #[test]
    fn web_shim_mesh_instance_layout_matches_this_const() {
        // The ONE hand-written copy of the instance layout is the JS shim.
        // seer renders the native path, so native drift shows in a frame;
        // nothing exercises the JS offsets but a real browser — so hold
        // web/src/main.ts to INSTANCE_ATTRS here. Embedded at compile time
        // so this runs in the fast game-tests gate, not just in a browser.
        let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/main.ts"));
        // Scope to the mesh pipeline's INSTANCE attribute array (locations
        // 3-6), not the vertex one (0-2) or any other pipeline's.
        let fn_at = src
            .find("game_gpu_render_pipeline_create_mesh")
            .expect("mesh pipeline factory in main.ts");
        let inst_at = fn_at
            + src[fn_at..]
                .find("stepMode: 'instance'")
                .expect("instance buffer in the mesh pipeline");
        let start = inst_at
            + src[inst_at..].find("attributes: [").expect("attributes array");
        let end = start + src[start..].find(']').expect("attributes array close");
        let parsed = parse_js_attrs(&src[start..end]);
        assert_eq!(
            parsed.len(),
            INSTANCE_ATTRS.len(),
            "JS mesh-instance attribute count drifted from INSTANCE_ATTRS"
        );
        for (want, got) in INSTANCE_ATTRS.iter().zip(&parsed) {
            assert_eq!(got.0, want.location, "JS shaderLocation drifted");
            assert_eq!(got.1, want.offset, "JS offset drifted at location {}", want.location);
            assert_eq!(got.2, want.format, "JS format drifted at location {}", want.location);
        }
    }

    /// Parse `{ shaderLocation: N, offset: M, format: 'F' }` entries, in
    /// order, from a JS attributes-array slice.
    fn parse_js_attrs(block: &str) -> Vec<(u32, u64, String)> {
        fn uint(s: &str) -> u64 {
            s.trim_start()
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap()
        }
        let mut out = Vec::new();
        let mut rest = block;
        while let Some(p) = rest.find("shaderLocation:") {
            rest = &rest[p + "shaderLocation:".len()..];
            let loc = uint(rest) as u32;
            let o = rest.find("offset:").expect("offset in attr");
            let off = uint(&rest[o + "offset:".len()..]);
            let f = rest.find("format:").expect("format in attr");
            let fs = &rest[f + "format:".len()..];
            let q0 = fs.find('\'').unwrap() + 1;
            let q1 = fs[q0..].find('\'').unwrap() + q0;
            out.push((loc, off, fs[q0..q1].to_string()));
        }
        out
    }

    #[test]
    fn cube_geometry_is_36_verts_on_unit_bounds_with_unit_normals() {
        let verts = cube_geometry();
        assert_eq!(verts.len(), 36);
        for v in &verts {
            for c in v.pos {
                assert!(c.abs() <= 0.5 + 1e-6, "vertex out of unit cube: {:?}", v.pos);
            }
            let n2: f32 = v.normal.iter().map(|c| c * c).sum();
            assert!((n2 - 1.0).abs() < 1e-4, "normal not unit: {:?}", v.normal);
        }
    }
}
