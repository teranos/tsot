// Wasm-side WebGPU init. Hand-wired env.* imports over a JS shim.
//
// Encapsulated pattern: JS owns the async chain (navigator.gpu →
// requestAdapter → requestDevice). Rust kicks it off with a policy
// argument and polls status. Rust never sees a Promise.

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuStatus {
    Pending = 0,
    Ready = 1,
    Unavailable = 2,
}

impl GpuStatus {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Ready,
            2 => Self::Unavailable,
            _ => Self::Pending,
        }
    }
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerPreference {
    Low = 0,
    High = 1,
}

// GPUBufferUsage flags from the WebGPU spec — passed through unchanged
// to createBuffer on the JS side.
pub mod usage {
    pub const MAP_READ: u32 = 0x0001;
    pub const MAP_WRITE: u32 = 0x0002;
    pub const COPY_SRC: u32 = 0x0004;
    pub const COPY_DST: u32 = 0x0008;
    pub const INDEX: u32 = 0x0010;
    pub const VERTEX: u32 = 0x0020;
    pub const UNIFORM: u32 = 0x0040;
    pub const STORAGE: u32 = 0x0080;
    pub const INDIRECT: u32 = 0x0100;
    pub const QUERY_RESOLVE: u32 = 0x0200;
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_gpu_init(power_pref: u32);
    fn game_gpu_status() -> u32;
    fn game_gpu_buffer_create(size: u32, usage: u32, label_ptr: *const u8, label_len: u32) -> u32;
    fn game_gpu_buffer_write(handle: u32, data_ptr: *const u8, data_len: u32);
    fn game_gpu_buffer_destroy(handle: u32);
}

#[cfg(target_arch = "wasm32")]
pub fn init(pref: PowerPreference) {
    unsafe { game_gpu_init(pref as u32) }
}

#[cfg(target_arch = "wasm32")]
pub fn status() -> GpuStatus {
    GpuStatus::from_u32(unsafe { game_gpu_status() })
}

/// Handle-wrapped GPUBuffer. Drop calls the JS-side destroy — the
/// axiom's whole point: buffer lifetime is Rust-controlled and
/// greppable, never left to a Rust-Drop-vs-JS-destroy mismatch.
#[cfg(target_arch = "wasm32")]
pub struct GameBuffer {
    handle: u32,
}

#[cfg(target_arch = "wasm32")]
impl GameBuffer {
    pub fn create(size: u32, usage: u32, label: &str) -> Option<Self> {
        let handle = unsafe {
            game_gpu_buffer_create(size, usage, label.as_ptr(), label.len() as u32)
        };
        if handle == 0 { None } else { Some(Self { handle }) }
    }

    pub fn write(&self, data: &[u8]) {
        unsafe { game_gpu_buffer_write(self.handle, data.as_ptr(), data.len() as u32) }
    }

    pub fn handle(&self) -> u32 {
        self.handle
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for GameBuffer {
    fn drop(&mut self) {
        unsafe { game_gpu_buffer_destroy(self.handle) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_u32_maps_documented_values() {
        assert_eq!(GpuStatus::from_u32(0), GpuStatus::Pending);
        assert_eq!(GpuStatus::from_u32(1), GpuStatus::Ready);
        assert_eq!(GpuStatus::from_u32(2), GpuStatus::Unavailable);
    }

    #[test]
    fn status_from_u32_out_of_range_is_pending() {
        assert_eq!(GpuStatus::from_u32(3), GpuStatus::Pending);
        assert_eq!(GpuStatus::from_u32(u32::MAX), GpuStatus::Pending);
    }
}
