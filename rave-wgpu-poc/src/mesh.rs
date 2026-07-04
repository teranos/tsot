//! Procedural unit primitives + the GPU-facing vertex/instance/uniform
//! structs. Everything is a UNIT mesh (unit quad, unit-radius cylinder
//! of unit height, unit-radius sphere); real-world size lives in each
//! instance's model matrix. That lets the forest reuse three vertex
//! buffers for hundreds of trees, mirroring rave's shared-mesh trick
//! in `rave/src/trees.rs`.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    pub const ATTRS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

/// Per-instance data: a full model matrix (translation * scale) + a
/// flat colour. `_pad` keeps the struct 16-byte aligned for bytemuck.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Instance {
    pub model: [[f32; 4]; 4],
    pub color: [f32; 3],
    pub _pad: f32,
}

impl Instance {
    pub const ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        2 => Float32x4,
        3 => Float32x4,
        4 => Float32x4,
        5 => Float32x4,
        6 => Float32x3,
    ];

    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRS,
        }
    }

    pub fn new(model: glam::Mat4, color: [f32; 3]) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color,
            _pad: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub light_dir: [f32; 4],
    pub ambient: [f32; 4],
}

/// CPU-side geometry, uploaded once to vertex + index buffers.
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Unit quad in the XZ plane, centred at origin, spanning
    /// [-1,1] on x and z, normal +Y. Scaled up per-instance to the
    /// forest floor's half-extent.
    pub fn quad() -> Self {
        let n = [0.0, 1.0, 0.0];
        let vertices = vec![
            Vertex { position: [-1.0, 0.0, -1.0], normal: n },
            Vertex { position: [1.0, 0.0, -1.0], normal: n },
            Vertex { position: [1.0, 0.0, 1.0], normal: n },
            Vertex { position: [-1.0, 0.0, 1.0], normal: n },
        ];
        let indices = vec![0, 2, 1, 0, 3, 2];
        Self { vertices, indices }
    }

    /// Unit-radius cylinder, unit height (y in [-0.5, 0.5]), `sides`
    /// radial segments. Side walls only — the caps are never seen on a
    /// tree trunk. Radial normals.
    pub fn cylinder(sides: u32) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for i in 0..sides {
            let a = (i as f32 / sides as f32) * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let normal = [c, 0.0, s];
            vertices.push(Vertex { position: [c, 0.5, s], normal });
            vertices.push(Vertex { position: [c, -0.5, s], normal });
        }
        for i in 0..sides {
            let top0 = (i * 2) % (sides * 2);
            let bot0 = (i * 2 + 1) % (sides * 2);
            let top1 = ((i + 1) * 2) % (sides * 2);
            let bot1 = ((i + 1) * 2 + 1) % (sides * 2);
            indices.extend_from_slice(&[top0, bot0, top1, top1, bot0, bot1]);
        }
        Self { vertices, indices }
    }

    /// Unit-radius UV sphere, `rings` latitude bands × `sectors`
    /// longitude segments. Position == normal (unit sphere).
    pub fn sphere(rings: u32, sectors: u32) -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for r in 0..=rings {
            let phi = std::f32::consts::PI * (r as f32 / rings as f32);
            let (sp, cp) = phi.sin_cos();
            for s in 0..=sectors {
                let theta = std::f32::consts::TAU * (s as f32 / sectors as f32);
                let (st, ct) = theta.sin_cos();
                let p = [sp * ct, cp, sp * st];
                vertices.push(Vertex { position: p, normal: p });
            }
        }
        let stride = sectors + 1;
        for r in 0..rings {
            for s in 0..sectors {
                let a = r * stride + s;
                let b = a + stride;
                indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        Self { vertices, indices }
    }
}
