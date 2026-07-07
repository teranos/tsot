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

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_gpu_init(power_pref: u32);
    fn game_gpu_status() -> u32;
}

#[cfg(target_arch = "wasm32")]
pub fn init(pref: PowerPreference) {
    unsafe { game_gpu_init(pref as u32) }
}

#[cfg(target_arch = "wasm32")]
pub fn status() -> GpuStatus {
    GpuStatus::from_u32(unsafe { game_gpu_status() })
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
