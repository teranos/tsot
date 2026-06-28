//! Chat — `rave-chat/v1` gossipsub topic.
//!
//! Bevy systems publish outgoing chat lines that JS pushed into a
//! thread-local buffer; incoming chat messages arrive through
//! `net_glue::drain_net_events` and get handed off to the HTML overlay
//! via the `__raveChatRecv` JS bridge. The Rust side never renders the
//! log — that lives in `web/src/chat-overlay.ts`, where it belongs:
//! HTML input + scroll log is what the DOM is for.
//!
//! Focus handshake: when the JS input has focus, the overlay calls
//! `rave_chat_set_focus(true)`; `room::move_player` reads
//! `is_chat_focused()` and skips WASD. Without this, typing "w" in the
//! input also moves the player.

/// gossipsub topic. Must match the relayer's `RAVE_CHAT_TOPIC` constant
/// and the integration test's `CHAT_TOPIC`.
pub const CHAT_TOPIC: &str = "rave-chat/v1";

/// Hard cap on outgoing body length. Anything past this is truncated at
/// the publish boundary so a paste-bomb can't blow past gossipsub's
/// per-message size cap (defaults to 64 KiB; we stay well under).
pub const MAX_CHAT_BODY_BYTES: usize = 512;

/// Returns `false` on native — chat doesn't exist outside wasm32. The
/// helper is callable unconditionally so `room::move_player` doesn't
/// need a cfg.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_chat_focused() -> bool {
    false
}

// `rave_chat_send` + `rave_chat_set_focus` are NOT re-exported here: they
// reach JS via `#[wasm_bindgen]` in the inner module; nothing in Rust
// calls them. Re-exporting tripped `unused_imports`.
#[cfg(target_arch = "wasm32")]
pub use wasm::{handle_incoming, is_chat_focused, publish_pending_chat};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{CHAT_TOPIC, MAX_CHAT_BODY_BYTES};
    use crate::error;
    use crate::net;
    use bevy::prelude::*;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::prelude::*;

    thread_local! {
        /// JS → Rust pending outgoing buffer. The overlay's submit
        /// handler calls `rave_chat_send`; the next Bevy Update tick's
        /// `publish_pending_chat` drains this and publishes via `Net`.
        static PENDING_OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };

        /// JS → Rust focus flag. The overlay's input toggles it on
        /// focus/blur; `room::move_player` reads it through
        /// `is_chat_focused()` and skips movement when true.
        static FOCUSED: Cell<bool> = const { Cell::new(false) };
    }

    #[wasm_bindgen]
    extern "C" {
        /// Rust → JS surface. Called from `publish_pending_chat`
        /// (self-echo) and from `handle_incoming` (peer message)
        /// with the serialised `RaveChatMsg` JSON. The overlay
        /// decodes + appends to the scroll log.
        #[wasm_bindgen(js_namespace = window, js_name = "__raveChatRecv")]
        fn js_rave_chat_recv(json: &str);
    }

    /// JS → Rust. Called when the user hits Enter in the chat input.
    /// Truncated past `MAX_CHAT_BODY_BYTES` at a UTF-8 char boundary
    /// so we never publish an invalid byte sequence.
    #[wasm_bindgen]
    pub fn rave_chat_send(body: String) {
        if body.is_empty() {
            return;
        }
        let trimmed = if body.len() > MAX_CHAT_BODY_BYTES {
            let mut end = MAX_CHAT_BODY_BYTES;
            while !body.is_char_boundary(end) {
                end -= 1;
            }
            body[..end].to_string()
        } else {
            body
        };
        PENDING_OUT.with(|cell| cell.borrow_mut().push(trimmed));
    }

    /// JS → Rust. Overlay calls `true` on input focus, `false` on
    /// blur. `room::move_player` reads it.
    #[wasm_bindgen]
    pub fn rave_chat_set_focus(focused: bool) {
        FOCUSED.with(|c| c.set(focused));
    }

    pub fn is_chat_focused() -> bool {
        FOCUSED.with(|c| c.get())
    }

    /// Drains `PENDING_OUT` and publishes each line to `rave-chat/v1`.
    /// Self-echoes locally via `__raveChatRecv` so the sender sees
    /// their own line — gossipsub doesn't send a publisher's messages
    /// back to themselves.
    pub fn publish_pending_chat(maybe_net: NonSend<Option<net::Net>>) {
        let Some(n) = maybe_net.as_ref() else {
            // Drop pending while the network is still booting; surface
            // a single typed error so the user knows messages typed
            // before identity resolves are lost.
            PENDING_OUT.with(|cell| {
                let mut q = cell.borrow_mut();
                if !q.is_empty() {
                    error::emit_region(
                        error::Severity::Warn,
                        "chat-pre-boot",
                        "chat send before Net ready",
                        format!("dropped {} pending message(s)", q.len()),
                    );
                    q.clear();
                }
            });
            return;
        };
        let self_peer = n.identity().0.clone();
        let drained: Vec<String> =
            PENDING_OUT.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        for body in drained {
            let msg = net::RaveChatMsg {
                peer: self_peer.clone(),
                body,
                at_ms: js_sys::Date::now() as u64,
            };
            let json = match serde_json::to_string(&msg) {
                Ok(s) => s,
                Err(e) => {
                    error::emit_region(
                        error::Severity::Error,
                        "chat-serialize",
                        "RaveChatMsg serialize failed",
                        format!("{e}"),
                    );
                    continue;
                }
            };
            if let Err(e) =
                n.publish(&net::Topic(CHAT_TOPIC.to_string()), json.as_bytes())
            {
                error::emit_region(
                    error::Severity::Error,
                    "chat-publish",
                    "publish to rave-chat/v1 failed",
                    format!("{e:?}"),
                );
                continue;
            }
            js_rave_chat_recv(&json);
        }
    }

    /// Called from `net_glue::drain_net_events` when a gossipsub
    /// message lands on `rave-chat/v1`. Deserialises, drops the
    /// sender's own messages (the self-echo path already showed
    /// them), and forwards the JSON to the overlay.
    pub fn handle_incoming(bytes: &[u8], self_peer: &str) {
        match serde_json::from_slice::<net::RaveChatMsg>(bytes) {
            Ok(msg) => {
                if msg.peer == self_peer {
                    return;
                }
                // Re-serialise rather than passing the raw bytes — the
                // wire shape is JSON anyway; this normalises whitespace
                // and proves the payload parsed before the overlay
                // sees it.
                match serde_json::to_string(&msg) {
                    Ok(json) => js_rave_chat_recv(&json),
                    Err(e) => {
                        error::emit_region(
                            error::Severity::Error,
                            "chat-reserialize",
                            "RaveChatMsg re-serialize failed",
                            format!("{e}"),
                        );
                    }
                }
            }
            Err(e) => {
                error::emit_region(
                    error::Severity::Error,
                    "chat-decode",
                    "malformed RaveChatMsg wire payload",
                    format!("{e}"),
                );
            }
        }
    }
}

