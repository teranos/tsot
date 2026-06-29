pub const CHAT_TOPIC: &str = "rave-chat/v1";

pub const MAX_CHAT_BODY_BYTES: usize = 512;

#[cfg(not(target_arch = "wasm32"))]
pub fn is_chat_focused() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
pub use wasm::{handle_incoming, is_chat_focused, publish_pending_chat};

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{CHAT_TOPIC, MAX_CHAT_BODY_BYTES};
    use crate::error;
    use crate::net::RaveChatMsg;
    use bevy::prelude::*;
    use bevy_libp2p::{LayeNet, Topic};
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::prelude::*;

    thread_local! {
        static PENDING_OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static FOCUSED: Cell<bool> = const { Cell::new(false) };
    }

    #[wasm_bindgen]
    unsafe extern "C" {
        #[wasm_bindgen(js_namespace = window, js_name = "__raveChatRecv")]
        fn js_rave_chat_recv(json: &str);
    }

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

    #[wasm_bindgen]
    pub fn rave_chat_set_focus(focused: bool) {
        FOCUSED.with(|c| c.set(focused));
    }

    pub fn is_chat_focused() -> bool {
        FOCUSED.with(|c| c.get())
    }

    pub fn publish_pending_chat(net: Res<LayeNet>) {
        let self_peer = net.identity().0.clone();
        let drained: Vec<String> =
            PENDING_OUT.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        for body in drained {
            let msg = RaveChatMsg {
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
            if let Err(e) = net.publish(&Topic(CHAT_TOPIC.to_string()), json.as_bytes()) {
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

    pub fn handle_incoming(bytes: &[u8], self_peer: &str) {
        match serde_json::from_slice::<RaveChatMsg>(bytes) {
            Ok(msg) => {
                if msg.peer == self_peer {
                    return;
                }
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
