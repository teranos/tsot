#![allow(dead_code)]

use std::cell::RefCell;
use std::ops::Deref;

use crate::teranos::{FlowerColor, FlowerCore, TileKind};
use crate::trace::{
    drain_json, emit, pending_count, TraceEvent, STATE_READ_COUNT, TICK_BLOCKED_COUNT, TICK_COUNT,
    VIEWPORT_READ_COUNT,
};
use std::sync::atomic::Ordering;
use crate::viewport::{viewport_ptr, write_viewport};
use crate::world::World;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use web_sys::HtmlCanvasElement;

thread_local! {
    static WORLD: RefCell<Option<World>> = const { RefCell::new(None) };
}

pub(crate) fn roam_init_impl() {
    // Install the panic hook FIRST so any panic raised during World
    // construction lands in the trace bus before the wasm module
    // aborts. The hook is idempotent (Once-gated), so repeated
    // roam_init calls are safe — they just rebuild the World on top
    // of the already-installed hook.
    crate::trace::install_panic_hook();
    WORLD.with(|w| *w.borrow_mut() = Some(World::new()));
}

pub(crate) fn roam_tick_impl(input: u32, dt_ms: f32) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.step(input, dt_ms);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_tick",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

pub(crate) fn roam_state_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::state_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_state",
                    msg: "called before roam_init; returning {}".to_string(),
                });
                "{}".to_string()
            })
    })
}

// ----- binary player-state FFI -----
//
// JSON-parsing the state every frame was a measurable cost in the
// Firefox profiler. The dirty-flag fingerprint + GL render only need
// (x, y, z, facing); the inventory still ships as JSON but the
// throttled HUD path reads it once every 500ms, not per frame.
//
// Layout (16 bytes, little-endian, repr(C)):
//   offset 0  f32  x
//   offset 4  f32  y
//   offset 8  i32  z
//   offset 12 u8   facing
//   offset 13 u8[3] padding
//
// `roam_player_state_ptr` snapshots into a thread-local static buffer
// and returns its base pointer. Caller reads via DataView; the buffer
// is overwritten on every call.

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct PlayerStateBinary {
    x: f32,
    y: f32,
    z: i32,
    facing: u8,
    _pad: [u8; 3],
}

const _: () = assert!(core::mem::size_of::<PlayerStateBinary>() == 16);

pub(crate) const PLAYER_STATE_LEN: u32 = 16;

thread_local! {
    static PLAYER_STATE_BUFFER: RefCell<PlayerStateBinary> =
        const { RefCell::new(PlayerStateBinary {
            x: 0.0,
            y: 0.0,
            z: 0,
            facing: 4,
            _pad: [0; 3],
        }) };
}

pub(crate) fn roam_player_state_ptr_impl() -> u32 {
    WORLD.with(|w| {
        if let Some(world) = w.borrow().as_ref() {
            PLAYER_STATE_BUFFER.with(|cell| {
                let mut s = cell.borrow_mut();
                s.x = world.player.x;
                s.y = world.player.y;
                s.z = world.player.z;
                s.facing = world.player.facing.as_u8();
            });
        }
        PLAYER_STATE_BUFFER.with(|cell| cell.borrow().deref() as *const _ as u32)
    })
}

pub(crate) fn roam_player_state_len_impl() -> u32 {
    PLAYER_STATE_LEN
}

pub(crate) fn roam_set_position_impl(x: f32, y: f32, facing: u8) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.set_position(x, y, facing);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_set_position",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

/// Binary viewport FFI. Writes the typed `[ViewportHeader, TileCell × N]`
/// byte sequence into a thread-local buffer and returns the total length.
/// JS reads the buffer through wasm memory at `roam_viewport_ptr()`.
pub(crate) fn roam_viewport_write_impl(view_w: u32, view_h: u32) -> u32 {
    WORLD.with(|w| match w.borrow().as_ref() {
        Some(world) => write_viewport(world, view_w, view_h),
        None => {
            emit(TraceEvent::Note {
                tag: "roam_viewport_write",
                msg: "called before roam_init; returning 0".to_string(),
            });
            0
        }
    })
}

pub(crate) fn roam_viewport_ptr_impl() -> u32 {
    viewport_ptr()
}

/// Color table FFI. Returns a pointer to a static byte buffer of RGB
/// triplets, laid out in the order documented below. Single source of
/// truth: JS / Elm never re-invent RGB on their side.
///
/// Layout, all triplets are u8[3]:
///
///   [0..15)  TileKind:    Air, Grass, Rock, ShallowWater, DeepWater
///   [15..36) FlowerColor: Red, Yellow, Blue, Purple, Azure, Pink, Glow
///   [36..45) FlowerCore:  White, Yellow, Black
///
/// The discriminant of each enum value matches its position in the
/// table; JS indexes as `palette[3 * (kind_offset + discriminant) + c]`.
pub(crate) const COLOR_TABLE_LEN: u32 = 45;
pub(crate) const COLOR_TABLE_TILE_OFFSET: u32 = 0;
pub(crate) const COLOR_TABLE_PETAL_OFFSET: u32 = 15;
pub(crate) const COLOR_TABLE_CORE_OFFSET: u32 = 36;

thread_local! {
    static COLOR_TABLE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn roam_color_table_ptr_impl() -> u32 {
    COLOR_TABLE.with(|t| {
        let mut buf = t.borrow_mut();
        if buf.is_empty() {
            for tk in [
                TileKind::Air,
                TileKind::Grass,
                TileKind::Rock,
                TileKind::ShallowWater,
                TileKind::DeepWater,
            ] {
                buf.extend_from_slice(&tk.rgb());
            }
            for fc in [
                FlowerColor::Red,
                FlowerColor::Yellow,
                FlowerColor::Blue,
                FlowerColor::Purple,
                FlowerColor::Azure,
                FlowerColor::Pink,
                FlowerColor::Glow,
            ] {
                buf.extend_from_slice(&fc.rgb());
            }
            for c in [FlowerCore::White, FlowerCore::Yellow, FlowerCore::Black] {
                buf.extend_from_slice(&c.rgb());
            }
            debug_assert_eq!(buf.len() as u32, COLOR_TABLE_LEN);
        }
        buf.as_ptr() as u32
    })
}

pub(crate) fn roam_color_table_len_impl() -> u32 {
    COLOR_TABLE_LEN
}

pub(crate) fn roam_pixels_per_tile_impl() -> u32 {
    crate::world::PIXELS_PER_TILE
}

// ----- per-frame counters (replaced the per-frame trace events) -----

pub(crate) fn roam_tick_count_impl() -> u64 {
    TICK_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_tick_blocked_count_impl() -> u64 {
    TICK_BLOCKED_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_state_read_count_impl() -> u64 {
    STATE_READ_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_viewport_read_count_impl() -> u64 {
    VIEWPORT_READ_COUNT.load(Ordering::Relaxed)
}

pub(crate) fn roam_drain_trace_impl() -> String {
    drain_json()
}

pub(crate) fn roam_trace_pending_count_impl() -> u32 {
    pending_count() as u32
}

pub(crate) fn roam_drain_errors_impl() -> String {
    let errors = crate::error::drain();
    match serde_json::to_string(&errors) {
        Ok(s) => s,
        Err(e) => {
            // Sacred-error rule: never swallow. `serde_json::to_string`
            // on `Vec<Error>` failing is an internal-shape regression
            // (the Error struct's serde derive broke) — surface it
            // through the trace bus so it lands in the event-log
            // panel, then fall back to `"[]"` so the FFI call
            // returns something parseable instead of corrupt JSON.
            crate::trace::emit(crate::trace::TraceEvent::Error {
                file: file!().to_string(),
                line: line!(),
                message: format!(
                    "roam_drain_errors_impl: serde_json::to_string failed: {e}"
                ),
            });
            "[]".to_string()
        }
    }
}

pub(crate) fn roam_session_snapshot_impl() -> String {
    WORLD.with(|w| {
        w.borrow()
            .as_ref()
            .map(World::session_snapshot_json)
            .unwrap_or_else(|| {
                emit(TraceEvent::Note {
                    tag: "roam_session_snapshot",
                    msg: "called before roam_init; returning empty".to_string(),
                });
                r#"{"picked":[],"inv":[]}"#.to_string()
            })
    })
}

pub(crate) fn roam_restore_session_impl(raw: String) {
    WORLD.with(|w| {
        if let Some(world) = w.borrow_mut().as_mut() {
            world.restore_session_json(&raw);
        } else {
            emit(TraceEvent::Note {
                tag: "roam_restore_session",
                msg: "called before roam_init; ignoring".to_string(),
            });
        }
    });
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports {
    use super::*;

    #[wasm_bindgen]
    pub fn roam_init() {
        super::roam_init_impl();
    }

    #[wasm_bindgen]
    pub fn roam_tick(input: u32, dt_ms: f32) {
        super::roam_tick_impl(input, dt_ms);
    }

    #[wasm_bindgen]
    pub fn roam_state() -> String {
        super::roam_state_impl()
    }

    #[wasm_bindgen]
    pub fn roam_viewport_write(view_w: u32, view_h: u32) -> u32 {
        super::roam_viewport_write_impl(view_w, view_h)
    }

    #[wasm_bindgen]
    pub fn roam_viewport_ptr() -> u32 {
        super::roam_viewport_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_color_table_ptr() -> u32 {
        super::roam_color_table_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_color_table_len() -> u32 {
        super::roam_color_table_len_impl()
    }

    #[wasm_bindgen]
    pub fn roam_pixels_per_tile() -> u32 {
        super::roam_pixels_per_tile_impl()
    }

    #[wasm_bindgen]
    pub fn roam_player_state_ptr() -> u32 {
        super::roam_player_state_ptr_impl()
    }

    #[wasm_bindgen]
    pub fn roam_player_state_len() -> u32 {
        super::roam_player_state_len_impl()
    }

    /// Construct the application-layer network state and attach it
    /// to `World`. Called once from the JS bridge after libp2p is up.
    ///
    /// The five JS callbacks form the seam between the trait
    /// (`crate::net::NetworkProvider`) and the existing libp2p
    /// instance. Future provider impls (`RustLibp2pProvider`,
    /// `RemoteServerProvider`) replace this constructor; the rest of
    /// the application never sees a JS function again.
    #[wasm_bindgen]
    pub fn roam_net_init(
        self_peer_id_fn: js_sys::Function,
        publish: js_sys::Function,
        subscribe: js_sys::Function,
        unsubscribe: js_sys::Function,
        drain_events: js_sys::Function,
    ) -> Result<(), JsValue> {
        let provider = crate::net::worker_bridge::WorkerBridge::new(
            self_peer_id_fn,
            publish,
            subscribe,
            unsubscribe,
            drain_events,
        )?;
        let mut net = crate::net::state::Net::new(Box::new(provider));
        if let Err(err) = net.subscribe_positions() {
            crate::error::emit(
                crate::error::Severity::Error,
                "roam::wasm_ffi::roam_net_init",
                "subscribe_positions failed",
                format!("{err:?}"),
            );
        }
        if let Err(err) = net.subscribe_pickups() {
            crate::error::emit(
                crate::error::Severity::Error,
                "roam::wasm_ffi::roam_net_init",
                "subscribe_pickups failed",
                format!("{err:?}"),
            );
        }
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow_mut().as_mut() {
                world.net = Some(net);
            } else {
                crate::error::emit(
                    crate::error::Severity::Error,
                    "roam::wasm_ffi::roam_net_init",
                    "called before roam_init",
                    "World is not constructed; Net not attached",
                );
            }
        });
        Ok(())
    }

    /// Construct a `RustLibp2pProvider` and attach it to `World.net`.
    /// `bootstrap_json` is a JSON array of multiaddr strings; the
    /// provider dials each one as the Swarm comes up. Parity drop-in
    /// for `roam_net_init`.
    ///
    /// The export is **always** present so the bridge's static
    /// `import { roam_net_init_rust_libp2p } from '/roam.js'` resolves
    /// regardless of build features (a missing export would
    /// `SyntaxError` at module load on default `make wasm` builds).
    /// When the `rust-libp2p` feature is OFF, the body returns an
    /// explicit error explaining the missing feature; the JS bridge
    /// already routes that through `logError`.
    #[wasm_bindgen]
    pub fn roam_net_init_rust_libp2p(bootstrap_json: String) -> Result<(), JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            let bootstrap_addrs: Vec<String> =
                serde_json::from_str(&bootstrap_json).map_err(|e| {
                    JsValue::from_str(&format!(
                        "roam_net_init_rust_libp2p: bootstrap_json parse failed: {e}"
                    ))
                })?;
            // Main-thread provider construction. The worker path
            // gets identity bytes from IndexedDB via the bridge; this
            // main-thread path is currently only used by the wasm-
            // bindgen test harness, so `None` is fine — it generates
            // a fresh keypair. Wire identity-bytes through here too
            // when the main-thread path becomes user-facing.
            let provider =
                crate::net::rust_libp2p::RustLibp2pProvider::new(bootstrap_addrs, None)
                    .map_err(|e| JsValue::from_str(&format!("RustLibp2pProvider::new: {e:?}")))?;
            let mut net = crate::net::state::Net::new(Box::new(provider));
            if let Err(err) = net.subscribe_positions() {
                crate::error::emit(
                    crate::error::Severity::Error,
                    "roam::wasm_ffi::roam_net_init_rust_libp2p",
                    "subscribe_positions failed",
                    format!("{err:?}"),
                );
            }
            if let Err(err) = net.subscribe_pickups() {
                crate::error::emit(
                    crate::error::Severity::Error,
                    "roam::wasm_ffi::roam_net_init_rust_libp2p",
                    "subscribe_pickups failed",
                    format!("{err:?}"),
                );
            }
            super::WORLD.with(|w| {
                if let Some(world) = w.borrow_mut().as_mut() {
                    world.net = Some(net);
                } else {
                    crate::error::emit(
                        crate::error::Severity::Error,
                        "roam::wasm_ffi::roam_net_init_rust_libp2p",
                        "called before roam_init",
                        "World is not constructed; Net not attached",
                    );
                }
            });
            Ok(())
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            let _ = bootstrap_json;
            Err(JsValue::from_str(
                "roam_net_init_rust_libp2p: this build was not compiled with --features rust-libp2p. Use `make wasm-rust` to enable the rust-libp2p substrate.",
            ))
        }
    }

    // ---- Worker-direct provider FFI (Option B) ----
    //
    // This wasm module is also instantiated inside `assets/src/net-worker.js`.
    // That worker gets its own browser event loop, so the Swarm's
    // `spawn_local` tasks aren't starved by render + Elm + wasm-init
    // contention on the main page thread (measured: up to 9.8s gaps —
    // see commit fc00b2a's heartbeat instrumentation).
    //
    // The exports below let the worker hold ONE `RustLibp2pProvider`
    // in a thread-local and drive it directly — no `Net`, no `World`.
    // Net stays in main-thread wasm with a `JsLibp2pProvider` whose
    // five callbacks postMessage commands/events to/from this worker.

    #[cfg(feature = "rust-libp2p")]
    thread_local! {
        static WORKER_PROVIDER: std::cell::RefCell<Option<crate::net::rust_libp2p::RustLibp2pProvider>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Construct the worker's singleton `RustLibp2pProvider` and
    /// return its peer-id (libp2p `12D3KooW…` string). The main-thread
    /// bridge captures the returned identity once on worker `ready`
    /// and exposes it through the `selfPeerId` callback to its
    /// `JsLibp2pProvider`.
    ///
    /// `identity_bytes` is the libp2p-canonical protobuf-encoded
    /// keypair the bridge loaded from IndexedDB. An empty slice
    /// means "no stored identity"; the worker generates a fresh
    /// keypair, but the bridge is expected to mint identity via
    /// `roam_net_generate_identity_bytes` first and pass those bytes
    /// here, so this path should only fire during a fault. PeerId
    /// derives from the keypair's public bits; passing the same
    /// bytes across sessions makes PeerId stable.
    #[wasm_bindgen]
    pub fn roam_net_worker_provider_init(
        bootstrap_json: String,
        identity_bytes: Vec<u8>,
    ) -> Result<String, JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            use crate::net::NetworkProvider;
            let bootstrap_addrs: Vec<String> = serde_json::from_str(&bootstrap_json)
                .map_err(|e| JsValue::from_str(&format!("bootstrap_json parse: {e}")))?;
            let identity_opt = if identity_bytes.is_empty() {
                None
            } else {
                Some(identity_bytes.as_slice())
            };
            let provider = crate::net::rust_libp2p::RustLibp2pProvider::new(
                bootstrap_addrs,
                identity_opt,
            )
            .map_err(|e| JsValue::from_str(&format!("provider::new: {e:?}")))?;
            let identity = provider.identity().0;
            WORKER_PROVIDER.with(|p| *p.borrow_mut() = Some(provider));
            Ok(identity)
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            let _ = (bootstrap_json, identity_bytes);
            Err(JsValue::from_str(
                "roam_net_worker_provider_init: rust-libp2p feature not enabled",
            ))
        }
    }

    // IDENTITY MENU (roam/docs/identity.md):
    //   A5 — read UCAN v1.0 spec abstract + invocation envelope shape.
    //   M8 — UCAN-based capability delegation, cross-device control without key transfer.
    /// Mint a fresh Ed25519 keypair and return its libp2p-canonical
    /// protobuf encoding. The bridge calls this once on first visit
    /// (when IndexedDB has no `roam/identity/v1` entry), persists
    /// the returned bytes, and passes them to every subsequent
    /// `roam_net_worker_provider_init` so PeerId is stable across
    /// sessions and devices that import the same key.
    #[wasm_bindgen]
    pub fn roam_net_generate_identity_bytes() -> Result<Vec<u8>, JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            crate::identity::generate_identity_protobuf()
                .map_err(|e| JsValue::from_str(&format!("identity gen: {e:?}")))
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            Err(JsValue::from_str(
                "roam_net_generate_identity_bytes: rust-libp2p feature not enabled",
            ))
        }
    }

    /// Self DID for the SELF panel (S1). Returns `did:key:z6Mk…` for
    /// the worker's persistent Ed25519 keypair, or an empty string if
    /// the provider isn't initialised or the keypair isn't Ed25519
    /// (today's bridge always feeds Ed25519). The worker reads this
    /// after init and includes it in the `kind: 'ready'` message so
    /// the bridge can render it alongside the PeerId.
    #[wasm_bindgen]
    pub fn roam_net_worker_provider_self_did_key() -> String {
        #[cfg(feature = "rust-libp2p")]
        {
            WORKER_PROVIDER.with(|p| {
                p.borrow()
                    .as_ref()
                    .map(|provider| provider.self_did_key().to_string())
                    .unwrap_or_default()
            })
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            String::new()
        }
    }

    #[wasm_bindgen]
    pub fn roam_net_worker_provider_publish(topic: String, bytes: Vec<u8>) -> Result<(), JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            use crate::net::{NetworkProvider, Topic};
            WORKER_PROVIDER.with(|p| match p.borrow_mut().as_mut() {
                Some(provider) => provider
                    .publish(&Topic(topic), &bytes)
                    .map_err(|e| JsValue::from_str(&format!("publish: {e:?}"))),
                None => Err(JsValue::from_str(
                    "roam_net_worker_provider_publish: provider not initialized",
                )),
            })
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            let _ = (topic, bytes);
            Err(JsValue::from_str("rust-libp2p feature not enabled"))
        }
    }

    #[wasm_bindgen]
    pub fn roam_net_worker_provider_subscribe(topic: String) -> Result<(), JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            use crate::net::{NetworkProvider, Topic};
            WORKER_PROVIDER.with(|p| match p.borrow_mut().as_mut() {
                Some(provider) => provider
                    .subscribe(&Topic(topic))
                    .map_err(|e| JsValue::from_str(&format!("subscribe: {e:?}"))),
                None => Err(JsValue::from_str(
                    "roam_net_worker_provider_subscribe: provider not initialized",
                )),
            })
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            let _ = topic;
            Err(JsValue::from_str("rust-libp2p feature not enabled"))
        }
    }

    #[wasm_bindgen]
    pub fn roam_net_worker_provider_unsubscribe(topic: String) -> Result<(), JsValue> {
        #[cfg(feature = "rust-libp2p")]
        {
            use crate::net::{NetworkProvider, Topic};
            WORKER_PROVIDER.with(|p| match p.borrow_mut().as_mut() {
                Some(provider) => provider
                    .unsubscribe(&Topic(topic))
                    .map_err(|e| JsValue::from_str(&format!("unsubscribe: {e:?}"))),
                None => Err(JsValue::from_str(
                    "roam_net_worker_provider_unsubscribe: provider not initialized",
                )),
            })
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            let _ = topic;
            Err(JsValue::from_str("rust-libp2p feature not enabled"))
        }
    }

    /// Drain queued NetEvents from the worker's provider. Returns a
    /// JSON-encoded array; the main thread's `JsLibp2pProvider` shim
    /// passes this through its `drainEvents` callback unmodified.
    #[wasm_bindgen]
    pub fn roam_net_worker_provider_drain_events() -> String {
        #[cfg(feature = "rust-libp2p")]
        {
            use crate::net::NetworkProvider;
            // The shape the JS-side `JsLibp2pProvider` callbacks expect
            // is a list of `{ topic, from, bytes: number[], at_ms }`
            // (see `js_libp2p.rs::MessageWire`). Message events flow
            // through that channel. PeerUp/PeerDown/Subscription/Error
            // events don't fit that shape — we surface them as
            // `trace::Note` events instead so they land in the main
            // thread's `#log` via `roam_drain_trace`. Without this,
            // dial failures and connection events are silently dropped.
            WORKER_PROVIDER.with(|p| match p.borrow_mut().as_mut() {
                Some(provider) => {
                    let events = provider.poll_events();
                    let mut messages: Vec<serde_json::Value> = Vec::new();
                    for e in events {
                        match e {
                            crate::net::NetEvent::Message {
                                topic,
                                from,
                                bytes,
                                at_ms,
                            } => {
                                messages.push(serde_json::json!({
                                    "topic": topic.0,
                                    // `from` is `Author(PeerId(String))`
                                    // post-F2 newtype. Emit the inner
                                    // String for the JS bridge; wire
                                    // shape unchanged.
                                    "from": from.0.0,
                                    "bytes": bytes,
                                    "at_ms": at_ms,
                                }));
                            }
                            crate::net::NetEvent::PeerUp { peer, addrs } => {
                                crate::trace::emit(crate::trace::TraceEvent::Note {
                                    tag: "net::peer_up",
                                    msg: format!("peer={} addrs={}", peer.0, addrs.join(",")),
                                });
                            }
                            crate::net::NetEvent::PeerDown { peer, reason } => {
                                crate::trace::emit(crate::trace::TraceEvent::Note {
                                    tag: "net::peer_down",
                                    msg: format!("peer={} reason={}", peer.0, reason),
                                });
                            }
                            crate::net::NetEvent::SubscriptionChange { topic, peer, joined } => {
                                crate::trace::emit(crate::trace::TraceEvent::Note {
                                    tag: "net::sub_change",
                                    msg: format!(
                                        "topic={} peer={} joined={}",
                                        topic.0, peer.0, joined
                                    ),
                                });
                            }
                            crate::net::NetEvent::Error(err) => {
                                crate::trace::emit(crate::trace::TraceEvent::Note {
                                    tag: "net::provider_error",
                                    msg: format!("{err:?}"),
                                });
                            }
                        }
                    }
                    match serde_json::to_string(&messages) {
                        Ok(s) => s,
                        Err(e) => {
                            // Same sacred-error treatment as
                            // `roam_drain_errors_impl` — surface the
                            // failure through the trace bus before
                            // falling back to `"[]"`.
                            crate::trace::emit(crate::trace::TraceEvent::Error {
                                file: file!().to_string(),
                                line: line!(),
                                message: format!(
                                    "roam_net_worker_provider_drain_events: serde_json::to_string failed: {e}"
                                ),
                            });
                            "[]".to_string()
                        }
                    }
                }
                None => "[]".to_string(),
            })
        }
        #[cfg(not(feature = "rust-libp2p"))]
        {
            "[]".to_string()
        }
    }

    /// Drain provider events, update the peer table, prune stale
    /// peers. Called once per frame from the JS bridge. `now_ms` is
    /// the JS-side `Date.now()` value (f64 to avoid BigInt at the
    /// boundary; truncated to u64 inside).
    #[wasm_bindgen]
    pub fn roam_net_tick(now_ms: f64) {
        super::WORLD.with(|w| {
            let mut wref = w.borrow_mut();
            let world = match wref.as_mut() {
                Some(w) => w,
                None => return,
            };
            // M6 — Net.tick processes inbound gossipsub events
            // (position updates + pickup claims); pickup claims queue
            // for World-level application. Borrowck split: drain the
            // queue out of `world.net`, then mutate `world.canonical_picked`
            // — both touch `world` but only one at a time.
            let pickups = match world.net.as_mut() {
                Some(net) => {
                    net.tick(now_ms as u64);
                    net.drain_pending_canonical_pickups()
                }
                None => return,
            };
            for (x, y) in pickups {
                world.canonical_picked.insert((x, y));
                crate::perf::PICKUP_APPLIED
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
    }

    /// Every gossipsub topic this node subscribes to, as a JSON
    /// array of strings. The JS bridge calls this once on
    /// worker-ready and posts one `subscribe` command per topic.
    /// Single source of truth for the topic list is
    /// `roam::net::state::ALL_TOPICS`; JS never holds topic strings.
    #[wasm_bindgen]
    pub fn roam_subscribed_topics_json() -> String {
        let mut s = String::from("[");
        let mut first = true;
        for t in crate::net::state::ALL_TOPICS {
            if !first {
                s.push(',');
            }
            first = false;
            s.push('"');
            s.push_str(t);
            s.push('"');
        }
        s.push(']');
        s
    }

    /// Canvas side length (square) for the given viewport. Layout
    /// decision lives in `roam::layout::canvas_side_px`; JS passes
    /// `window.innerWidth/Height` and applies the returned side to
    /// the canvas.
    #[wasm_bindgen]
    pub fn roam_canvas_side_px(window_w: u32, window_h: u32) -> u32 {
        crate::layout::canvas_side_px(window_w, window_h)
    }

    /// JSON snapshot of every perf counter. JS polls this once per
    /// second and computes per-second rates by diffing successive
    /// snapshots — Rust stays cumulative. Shape lives in
    /// `roam::perf::snapshot_json`.
    #[wasm_bindgen]
    pub fn roam_perf_snapshot_json() -> String {
        crate::perf::snapshot_json()
    }

    /// Number of remote peers currently in the Rust-owned peer table.
    /// Used by the bridge for the status HUD and as the canonical
    /// peer-count source (the JS-side table was deleted in 2d).
    #[wasm_bindgen]
    pub fn roam_net_peer_count() -> u32 {
        super::WORLD.with(|w| {
            w.borrow()
                .as_ref()
                .and_then(|world| world.net.as_ref().map(|n| n.peers().count() as u32))
                .unwrap_or(0)
        })
    }

    /// JSON snapshot of the peer table for the panel renderer. Shape:
    /// `[{ peer_id, did_key, x, y, facing, last_seen_ms }, …]`.
    /// `did_key` is the empty string for any peer whose libp2p PeerId
    /// can't be decoded as Ed25519 — in production today every PeerId
    /// is Ed25519 (we mint Ed25519 only), so the empty string is the
    /// "should never happen" sentinel; the sacred-error path captures
    /// the actual decode failure for the event log so it can't slip
    /// past unobserved. Cadence is "whenever the panel renders," not
    /// per-frame — the renderer reads positions through the packed
    /// buffer in `roam_render_frame`, not this JSON.
    #[wasm_bindgen]
    pub fn roam_net_peers_json() -> String {
        super::WORLD.with(|w| {
            let world_ref = w.borrow();
            let net = match world_ref.as_ref().and_then(|world| world.net.as_ref()) {
                Some(n) => n,
                None => return "[]".to_string(),
            };
            let mut out = String::from("[");
            let mut first = true;
            for p in net.peers() {
                if !first {
                    out.push(',');
                }
                first = false;
                let did = match p.peer_id.did_key() {
                    Ok(d) => d,
                    Err(e) => {
                        crate::error::emit(
                            crate::error::Severity::Warn,
                            "roam::wasm_ffi::roam_net_peers_json",
                            "peer did:key decode failed",
                            format!("peer_id={} err={:?}", p.peer_id.0.0, e),
                        );
                        String::new()
                    }
                };
                let entry = serde_json::json!({
                    "peer_id": p.peer_id.0.0,
                    "did_key": did,
                    "x": p.x,
                    "y": p.y,
                    "facing": p.facing,
                    "last_seen_ms": p.last_seen_ms,
                });
                out.push_str(&entry.to_string());
            }
            out.push(']');
            out
        })
    }

    /// Monotonic counter that bumps on any peer-table change (add,
    /// remove, position update, timeout). Bridge folds this into the
    /// render dirty-flag fingerprint so peers moving on screen
    /// triggers a repaint even when the local player is still.
    #[wasm_bindgen]
    pub fn roam_net_peer_state_seq() -> u32 {
        super::WORLD.with(|w| {
            w.borrow()
                .as_ref()
                .and_then(|world| world.net.as_ref().map(|n| n.peer_state_seq()))
                .unwrap_or(0)
        })
    }

    /// Publish the local player's current position on the canonical
    /// positions topic. Called from the JS bridge's broadcast timer.
    /// No-op if `Net` hasn't been attached yet (libp2p still booting).
    #[wasm_bindgen]
    pub fn roam_net_publish_position() {
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow_mut().as_mut() {
                let (x, y, z, facing) = (
                    world.player.x,
                    world.player.y,
                    world.player.z,
                    world.player.facing.as_u8(),
                );
                if let Some(net) = world.net.as_mut() {
                    if let Err(err) = net.publish_position(x, y, z, facing) {
                        crate::error::emit(
                            crate::error::Severity::Warn,
                            "roam::wasm_ffi::roam_net_publish_position",
                            "publish_position failed",
                            format!("{err:?}"),
                        );
                    }
                }
            }
        });
    }

    #[wasm_bindgen]
    pub fn roam_tick_count() -> u64 {
        super::roam_tick_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_tick_blocked_count() -> u64 {
        super::roam_tick_blocked_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_state_read_count() -> u64 {
        super::roam_state_read_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_viewport_read_count() -> u64 {
        super::roam_viewport_read_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_set_position(x: f32, y: f32, facing: u8) {
        super::roam_set_position_impl(x, y, facing);
    }

    #[wasm_bindgen]
    pub fn roam_drain_trace() -> String {
        super::roam_drain_trace_impl()
    }

    #[wasm_bindgen]
    pub fn roam_trace_pending_count() -> u32 {
        super::roam_trace_pending_count_impl()
    }

    #[wasm_bindgen]
    pub fn roam_session_snapshot() -> String {
        super::roam_session_snapshot_impl()
    }

    #[wasm_bindgen]
    pub fn roam_restore_session(raw: String) {
        super::roam_restore_session_impl(raw);
    }

    #[wasm_bindgen]
    pub fn roam_drain_errors() -> String {
        super::roam_drain_errors_impl()
    }

    // ----- S4a: WebGL2 renderer wire -----

    #[wasm_bindgen]
    pub fn roam_render_init(canvas: HtmlCanvasElement) -> Result<(), JsValue> {
        crate::render_gl::init(canvas)
    }

    #[wasm_bindgen]
    pub fn roam_render_frame(
        player_x_px: f32,
        player_y_px: f32,
        facing: u8,
        zoom: f32,
        canvas_w: u32,
        canvas_h: u32,
        day_brightness: f32,
    ) -> Result<(), JsValue> {
        // Phase 2d: peer markers come from the Rust-owned Net.peers
        // table, not a JS-published list. We pack here right before
        // the draw call so the renderer always sees the latest peer
        // table without a separate FFI per frame.
        super::WORLD.with(|w| {
            if let Some(world) = w.borrow().as_ref() {
                if let Some(net) = world.net.as_ref() {
                    let mut packed: Vec<f32> = Vec::with_capacity(net.peers().count() * 3);
                    for p in net.peers() {
                        packed.push(p.x);
                        packed.push(p.y);
                        // Source tag: 0.0 = libp2p. We dropped the
                        // BroadcastChannel fallback when the JS-side
                        // peer table went away — all remote peers
                        // flow through the libp2p mesh now.
                        packed.push(0.0);
                    }
                    crate::render_gl::set_peers(&packed);
                }
            }
        });

        crate::render_gl::render_frame(
            player_x_px,
            player_y_px,
            facing,
            zoom,
            canvas_w,
            canvas_h,
            day_brightness,
        )
    }
}
