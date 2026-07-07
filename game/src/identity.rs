// Persistent per-browser identity — 32 bytes generated on first
// launch, stored in IndexedDB via the JS shim, loaded on every boot.
// Same hand-wired pattern as the GPU boundary: three env.* imports,
// Rust owns policy (generate/load/persist), JS owns storage.
//
// Not yet Ed25519 — for now the bytes are opaque. Once signing is
// needed (remote-players / gossipsub), the same 32 bytes seed
// ed25519-dalek's SigningKey with no schema change here.

pub const IDENTITY_LEN: usize = 32;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_identity_load(out_ptr: *mut u8) -> u32;
    fn game_identity_save(bytes_ptr: *const u8, bytes_len: u32);
    fn game_random_bytes(out_ptr: *mut u8, out_len: u32);
}

#[derive(Clone, Copy)]
pub struct Identity {
    pub bytes: [u8; IDENTITY_LEN],
    pub is_new: bool,
}

impl Identity {
    /// Try IndexedDB via game_identity_load. If nothing is stored,
    /// generate 32 bytes via game_random_bytes and persist them.
    #[cfg(target_arch = "wasm32")]
    pub fn load_or_create() -> Self {
        let mut buf = [0u8; IDENTITY_LEN];
        let n = unsafe { game_identity_load(buf.as_mut_ptr()) };
        if n as usize == IDENTITY_LEN {
            return Self { bytes: buf, is_new: false };
        }
        unsafe { game_random_bytes(buf.as_mut_ptr(), IDENTITY_LEN as u32) };
        unsafe { game_identity_save(buf.as_ptr(), IDENTITY_LEN as u32) };
        Self { bytes: buf, is_new: true }
    }

    /// Native has no IndexedDB and no live player — return a fixed
    /// zero identity so CI runs stay deterministic.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_or_create() -> Self {
        Self {
            bytes: [0u8; IDENTITY_LEN],
            is_new: true,
        }
    }

    pub fn as_hex(&self) -> String {
        let mut out = String::with_capacity(IDENTITY_LEN * 2);
        for b in &self.bytes {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }

    /// Short display label: "abcdef…123456" — enough to eyeball
    /// identity persistence across reloads.
    pub fn short(&self) -> String {
        let hex = self.as_hex();
        if hex.len() >= 12 {
            format!("{}…{}", &hex[..6], &hex[hex.len() - 6..])
        } else {
            hex
        }
    }
}
