use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GamePosition {
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
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(&value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, parsed);
    }

    #[test]
    fn game_position_round_trips() {
        round_trip(GamePosition {
            peer: "12D3KooWPeerSelf".into(),
            x: 1.5,
            y: 0.0,
            z: -3.2,
            at_ms: 1_700_000_000_000,
        });
    }
}
