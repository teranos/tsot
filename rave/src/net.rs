//! Network surface for rave's libp2p slice.
//!
//! Pure-data types: PeerId, Topic, NetError, NetEvent, RavePosition.
//! Wire format is JSON via serde for the position broadcast at
//! `rave-positions/v1`. Errors never collapse — every variant of
//! NetError pins one cause.

use serde::{Deserialize, Serialize};

/// libp2p PeerId in its base58btc form (`12D3KooW…` for Ed25519 keys).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub String);

/// gossipsub topic name. Single string, no hashing — IdentTopic on the
/// libp2p side.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Topic(pub String);

/// One cause per variant. No collapse — each carries the context the
/// drawer needs to surface what the user should do next.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetError {
    PublishFailed { topic: Topic, reason: String },
    SubscribeFailed { topic: Topic, reason: String },
    NotConnected { reason: String },
    InvalidTopic { topic: Topic, reason: String },
    ProviderInternal { reason: String },
}

/// Asynchronous events the Swarm task accumulates and the Bevy drain
/// system reads each frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetEvent {
    PeerUp {
        peer: PeerId,
        addrs: Vec<String>,
    },
    PeerDown {
        peer: PeerId,
        reason: String,
    },
    Message {
        topic: Topic,
        from: PeerId,
        bytes: Vec<u8>,
        at_ms: u64,
    },
    SubscriptionChange {
        topic: Topic,
        peer: PeerId,
        joined: bool,
    },
    Error(NetError),
}

/// Wire payload for `rave-positions/v1`. Player XYZ in world units,
/// peer's libp2p PeerId, wall-clock millis at publish.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RavePosition {
    pub peer: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
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
    fn peer_id_round_trips() {
        round_trip(PeerId("12D3KooWXYZ".to_string()));
    }

    #[test]
    fn topic_round_trips() {
        round_trip(Topic("rave-positions/v1".to_string()));
    }

    #[test]
    fn net_error_variants_round_trip() {
        round_trip(NetError::PublishFailed {
            topic: Topic("t".into()),
            reason: "queue full".into(),
        });
        round_trip(NetError::SubscribeFailed {
            topic: Topic("t".into()),
            reason: "transport down".into(),
        });
        round_trip(NetError::NotConnected {
            reason: "no mesh peers".into(),
        });
        round_trip(NetError::InvalidTopic {
            topic: Topic("".into()),
            reason: "empty topic name".into(),
        });
        round_trip(NetError::ProviderInternal {
            reason: "wasm-bindgen panic".into(),
        });
    }

    #[test]
    fn net_event_variants_round_trip() {
        round_trip(NetEvent::PeerUp {
            peer: PeerId("p".into()),
            addrs: vec!["/dns4/x/tcp/443/wss".into()],
        });
        round_trip(NetEvent::PeerDown {
            peer: PeerId("p".into()),
            reason: "timeout".into(),
        });
        round_trip(NetEvent::Message {
            topic: Topic("t".into()),
            from: PeerId("p".into()),
            bytes: vec![1, 2, 3],
            at_ms: 1_700_000_000_000,
        });
        round_trip(NetEvent::SubscriptionChange {
            topic: Topic("t".into()),
            peer: PeerId("p".into()),
            joined: true,
        });
        round_trip(NetEvent::Error(NetError::NotConnected {
            reason: "no mesh peers".into(),
        }));
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
}
