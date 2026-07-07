// Pure ingest layer for remote peer positions. No env.*, no Bevy
// systems here — just: given wire bytes, mutate the table; given a
// clock, evict what's older than STALE_MS. The env.* boundary and
// Bevy system wrappers live one level up.

use std::collections::HashMap;

use bevy_ecs::resource::Resource;
use bevy_math::Vec3;

use crate::net::GamePosition;

pub const STALE_MS: u64 = 30_000;

/// Deterministic bright-RGB from a peer id string. Same peer always
/// gets the same colour; different peers get different colours. FNV-1a
/// hash → three bytes → clamped to [0.3, 1.0] so nothing blends into
/// the dark background. Kept as a top-level function so overriding
/// (querystring, wire field, IndexedDB pref) is a one-line swap at
/// each call site.
pub fn color_for_peer(peer: &str) -> [f32; 3] {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in peer.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let channel = |shift: u32| -> f32 {
        let byte = ((h >> shift) & 0xff) as f32 / 255.0;
        0.3 + byte * 0.7
    };
    [channel(0), channel(8), channel(16)]
}

// Wire boundary. Four env.* imports:
// - game_peers_pending()          bytes queued from proxy this tick
// - game_peers_recv(ptr, len)     drain those bytes into wasm memory
// - game_self_publish(ptr, len)   hand one GamePosition JSON to JS
// - game_now_ms()                 Date.now() as f64
// See imports.allow for the CI-enforced list.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn game_peers_pending() -> u32;
    fn game_peers_recv(out_ptr: *mut u8, out_len: u32) -> u32;
    fn game_self_publish(bytes_ptr: *const u8, bytes_len: u32);
    fn game_now_ms() -> f64;
}

pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        let ms = unsafe { game_now_ms() };
        ms as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct RemoteEntry {
    pub pos: Vec3,
    pub last_seen_ms: u64,
}

#[derive(Resource, Default)]
pub struct RemotePlayers(pub HashMap<String, RemoteEntry>);

#[derive(Debug)]
pub enum IngestError {
    Decode(serde_json::Error),
    Frame(FrameError),
    Encode(serde_json::Error),
}

#[derive(Debug, PartialEq)]
pub enum FrameError {
    ShortHeader,
    ShortBody { needed: usize, remaining: usize },
}

/// Length-prefixed frames: [len_le:u32][bytes[len]]... Concatenated
/// into a single buffer by the JS shim, one `game_peers_recv` call
/// drains everything the WebSocket has queued this tick.
pub fn parse_frames(buffer: &[u8]) -> Result<Vec<&[u8]>, FrameError> {
    let mut frames = Vec::new();
    let mut i = 0;
    while i < buffer.len() {
        if buffer.len() - i < 4 {
            return Err(FrameError::ShortHeader);
        }
        let len =
            u32::from_le_bytes([buffer[i], buffer[i + 1], buffer[i + 2], buffer[i + 3]]) as usize;
        i += 4;
        let remaining = buffer.len() - i;
        if remaining < len {
            return Err(FrameError::ShortBody {
                needed: len,
                remaining,
            });
        }
        frames.push(&buffer[i..i + len]);
        i += len;
    }
    Ok(frames)
}

pub fn ingest_message(
    remotes: &mut RemotePlayers,
    bytes: &[u8],
    now_ms: u64,
    self_peer: &str,
) -> Result<(), IngestError> {
    let pos: GamePosition = serde_json::from_slice(bytes).map_err(IngestError::Decode)?;
    if pos.peer == self_peer {
        return Ok(());
    }
    let entry = remotes.0.entry(pos.peer).or_default();
    entry.pos = Vec3::new(pos.x, pos.y, pos.z);
    entry.last_seen_ms = now_ms;
    Ok(())
}

pub fn evict_stale(remotes: &mut RemotePlayers, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(STALE_MS);
    remotes.0.retain(|_, e| e.last_seen_ms > cutoff);
}

/// Drain queued proxy bytes into RemotePlayers. Native returns Ok(0)
/// — no proxy attached. wasm calls game_peers_pending + game_peers_recv.
pub fn pump_from_proxy(
    remotes: &mut RemotePlayers,
    now_ms: u64,
    self_peer: &str,
) -> Result<usize, IngestError> {
    #[cfg(target_arch = "wasm32")]
    {
        let pending = unsafe { game_peers_pending() } as usize;
        if pending == 0 {
            return Ok(0);
        }
        let mut buf = vec![0u8; pending];
        let written =
            unsafe { game_peers_recv(buf.as_mut_ptr(), pending as u32) } as usize;
        buf.truncate(written);
        let frames = parse_frames(&buf).map_err(IngestError::Frame)?;
        let mut count = 0;
        for f in frames {
            ingest_message(remotes, f, now_ms, self_peer)?;
            count += 1;
        }
        Ok(count)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (remotes, now_ms, self_peer);
        Ok(0)
    }
}

/// Serialize one GamePosition and hand it to the JS shim. JS relays
/// to the proxy WebSocket. Native no-ops.
pub fn publish_position(
    self_peer: &str,
    pos: Vec3,
    at_ms: u64,
) -> Result<(), IngestError> {
    let msg = GamePosition {
        peer: self_peer.to_string(),
        x: pos.x,
        y: pos.y,
        z: pos.z,
        at_ms,
    };
    let bytes = serde_json::to_vec(&msg).map_err(IngestError::Encode)?;
    #[cfg(target_arch = "wasm32")]
    unsafe {
        game_self_publish(bytes.as_ptr(), bytes.len() as u32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = bytes;
    }
    Ok(())
}
