use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_libp2p::{LayeNet, LibP2PMessage, NetEvent, Topic};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMsg {
    pub peer: String,
    pub body: String,
    pub at_ms: u64,
}

#[derive(Resource)]
pub struct ChatConfig {
    pub topic: Topic,
    pub max_body_bytes: usize,
}

#[derive(Message, Debug, Clone)]
pub struct OutgoingChat(pub String);

#[derive(Message, Debug, Clone)]
pub struct IncomingChat(pub ChatMsg);

pub struct ChatPlugin {
    pub topic: String,
    pub max_body_bytes: usize,
}

impl Default for ChatPlugin {
    fn default() -> Self {
        Self {
            topic: "laye-chat/v1".to_string(),
            max_body_bytes: 512,
        }
    }
}

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChatConfig {
            topic: Topic(self.topic.clone()),
            max_body_bytes: self.max_body_bytes,
        });
        app.add_message::<OutgoingChat>();
        app.add_message::<IncomingChat>();
        app.add_systems(Update, (publish_outgoing, route_incoming));
    }
}

fn publish_outgoing(
    net: Res<LayeNet>,
    cfg: Res<ChatConfig>,
    mut reader: MessageReader<OutgoingChat>,
) {
    let self_peer = net.identity().0.clone();
    for OutgoingChat(body) in reader.read() {
        if body.is_empty() {
            continue;
        }
        let trimmed = trim_to_char_boundary(body, cfg.max_body_bytes);
        let msg = ChatMsg {
            peer: self_peer.clone(),
            body: trimmed,
            at_ms: now_ms(),
        };
        let Ok(bytes) = serde_json::to_vec(&msg) else {
            continue;
        };
        let _ = net.publish(&cfg.topic, &bytes);
    }
}

fn route_incoming(
    net: Res<LayeNet>,
    cfg: Res<ChatConfig>,
    mut reader: MessageReader<LibP2PMessage>,
    mut writer: MessageWriter<IncomingChat>,
) {
    let self_peer = net.identity().0.clone();
    for msg in reader.read() {
        let NetEvent::Message { topic, bytes, .. } = &msg.0 else {
            continue;
        };
        if topic.0 != cfg.topic.0 {
            continue;
        }
        let Ok(chat) = serde_json::from_slice::<ChatMsg>(bytes) else {
            continue;
        };
        if chat.peer == self_peer {
            continue;
        }
        writer.write(IncomingChat(chat));
    }
}

fn trim_to_char_boundary(body: &str, max_bytes: usize) -> String {
    if body.len() <= max_bytes {
        return body.to_string();
    }
    let mut end = max_bytes;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    body[..end].to_string()
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
