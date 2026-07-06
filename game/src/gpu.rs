// Real wgpu adapter. Every SeerDevice::create_* method calls into the
// underlying wgpu::Device AND emits an obs event with size, usage,
// label — the observability contract established in Phase 3 now backed
// by an actual GPU resource.
//
// SeerBuffer::drop and SeerTexture::drop both explicitly call
// wgpu::Buffer::destroy() / Texture::destroy() before emitting the
// destroyed event. That's the architectural fix for the WebGPU-wasm
// no-op Drop issue that leaked resources in rave — done from day one
// as a design decision, not retrofitted.
//
// Native-only for now. The wasmtime target keeps calling obs::* via
// simulation. Browser deploy will bring wgpu back with wasm-bindgen
// glue in a later phase.

use wgpu::{Buffer, BufferDescriptor, Device, ShaderModule, ShaderModuleDescriptor, ShaderSource, Texture, TextureDescriptor};

use crate::obs;

pub struct SeerDevice {
    inner: Device,
}

impl SeerDevice {
    pub fn new(device: Device) -> Self {
        Self { inner: device }
    }

    pub fn wgpu(&self) -> &Device {
        &self.inner
    }

    pub fn create_buffer(&self, desc: &BufferDescriptor) -> SeerBuffer {
        let buffer = self.inner.create_buffer(desc);
        let label = desc.label.unwrap_or("");
        let id = obs::buffer_created(desc.size, desc.usage.bits(), label);
        SeerBuffer {
            inner: Some(buffer),
            id,
        }
    }

    pub fn create_texture(&self, desc: &TextureDescriptor) -> SeerTexture {
        let texture = self.inner.create_texture(desc);
        let label = desc.label.unwrap_or("");
        let bytes = approx_texture_bytes(desc);
        let id = obs::texture_created(bytes, desc.usage.bits(), label);
        SeerTexture {
            inner: Some(texture),
            id,
        }
    }

    pub fn create_shader_module(&self, desc: ShaderModuleDescriptor) -> SeerShader {
        let code_len = match &desc.source {
            ShaderSource::Wgsl(s) => s.len() as u64,
            _ => 0,
        };
        let label = desc.label.unwrap_or("").to_string();
        let shader = self.inner.create_shader_module(desc);
        let id = obs::shader_created(code_len, &label);
        SeerShader { inner: shader, id }
    }
}

pub struct SeerBuffer {
    inner: Option<Buffer>,
    id: u64,
}

impl SeerBuffer {
    pub fn wgpu(&self) -> &Buffer {
        self.inner.as_ref().expect("buffer already destroyed")
    }
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for SeerBuffer {
    fn drop(&mut self) {
        if let Some(buf) = self.inner.take() {
            buf.destroy();
            obs::resource_destroyed(self.id);
        }
    }
}

pub struct SeerTexture {
    inner: Option<Texture>,
    id: u64,
}

impl SeerTexture {
    pub fn wgpu(&self) -> &Texture {
        self.inner.as_ref().expect("texture already destroyed")
    }
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for SeerTexture {
    fn drop(&mut self) {
        if let Some(tex) = self.inner.take() {
            tex.destroy();
            obs::resource_destroyed(self.id);
        }
    }
}

pub struct SeerShader {
    inner: ShaderModule,
    id: u64,
}

impl SeerShader {
    pub fn wgpu(&self) -> &ShaderModule {
        &self.inner
    }
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for SeerShader {
    fn drop(&mut self) {
        obs::resource_destroyed(self.id);
    }
}

fn approx_texture_bytes(desc: &TextureDescriptor) -> u64 {
    let w = desc.size.width as u64;
    let h = desc.size.height as u64;
    let d = desc.size.depth_or_array_layers as u64;
    let mips = desc.mip_level_count.max(1) as u64;
    let bpp = 4u64;
    let base = w * h * d * bpp;
    let mut total = base;
    let mut this = base;
    for _ in 1..mips {
        this /= 4;
        total += this;
    }
    total
}
