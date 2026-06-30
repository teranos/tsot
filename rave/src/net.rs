use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RavePosition {
    pub peer: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaveChatMsg {
    pub peer: String,
    pub body: String,
    pub at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: T)
    where
        T: serde::Serialize + for<'de> serde::Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(&value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, parsed);
    }

    #[test]
    fn rave_position_round_trips() {
        round_trip(RavePosition {
            peer: "12D3KooWPeerSelf".into(),
            x: 1.5,
            y: 0.0,
            z: -3.2,
            at_ms: 1_700_000_000_000,
        });
    }

    #[test]
    fn rave_chat_msg_round_trips() {
        round_trip(RaveChatMsg {
            peer: "12D3KooWPeerSelf".into(),
            body: "hello from the dancefloor 🪩".into(),
            at_ms: 1_700_000_000_000,
        });
    }

    #[test]
    fn rave_chat_msg_empty_body_round_trips() {
        round_trip(RaveChatMsg {
            peer: "12D3KooWPeerSelf".into(),
            body: String::new(),
            at_ms: 0,
        });
    }
}
