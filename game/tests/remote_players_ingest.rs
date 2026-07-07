// Pure ingest logic — no ECS, no env.*, no proxy. Feeds bytes in the
// same wire format the browser will forward from `game-proxy` (JSON
// `GamePosition`), asserts the RemotePlayers table matches.

use bevy_math::Vec3;
use game::net::GamePosition;
use game::remote_players::{RemotePlayers, STALE_MS, evict_stale, ingest_message};

const SELF_PEER: &str = "self-abc";
const OTHER_PEER: &str = "peer-xyz";

fn wire(peer: &str, x: f32, y: f32, z: f32, at_ms: u64) -> Vec<u8> {
    serde_json::to_vec(&GamePosition {
        peer: peer.to_string(),
        x,
        y,
        z,
        at_ms,
    })
    .expect("serialize GamePosition")
}

#[test]
fn remote_position_appears() {
    let mut r = RemotePlayers::default();
    ingest_message(&mut r, &wire(OTHER_PEER, 10.0, 0.0, 20.0, 1_000), 1_000, SELF_PEER).unwrap();
    assert_eq!(r.0.len(), 1);
    let e = r.0.get(OTHER_PEER).expect("peer entry");
    assert_eq!(e.pos, Vec3::new(10.0, 0.0, 20.0));
    assert_eq!(e.last_seen_ms, 1_000);
}

#[test]
fn remote_position_updates_existing() {
    let mut r = RemotePlayers::default();
    ingest_message(&mut r, &wire(OTHER_PEER, 10.0, 0.0, 20.0, 1_000), 1_000, SELF_PEER).unwrap();
    ingest_message(&mut r, &wire(OTHER_PEER, 11.0, 0.0, 22.0, 2_500), 2_500, SELF_PEER).unwrap();
    assert_eq!(r.0.len(), 1);
    let e = r.0.get(OTHER_PEER).unwrap();
    assert_eq!(e.pos, Vec3::new(11.0, 0.0, 22.0));
    assert_eq!(e.last_seen_ms, 2_500);
}

#[test]
fn self_peer_ignored() {
    let mut r = RemotePlayers::default();
    ingest_message(&mut r, &wire(SELF_PEER, 5.0, 0.0, 5.0, 1_000), 1_000, SELF_PEER).unwrap();
    assert_eq!(r.0.len(), 0);
}

#[test]
fn stale_entry_evicted() {
    let mut r = RemotePlayers::default();
    ingest_message(&mut r, &wire(OTHER_PEER, 1.0, 0.0, 1.0, 1_000), 1_000, SELF_PEER).unwrap();
    // Just under the cutoff — still there.
    evict_stale(&mut r, 1_000 + STALE_MS - 1);
    assert_eq!(r.0.len(), 1);
    // At/over the cutoff — evicted.
    evict_stale(&mut r, 1_000 + STALE_MS);
    assert_eq!(r.0.len(), 0);
}

#[test]
fn malformed_json_errors() {
    let mut r = RemotePlayers::default();
    let err = ingest_message(&mut r, b"{not json", 1_000, SELF_PEER)
        .expect_err("malformed JSON must surface, not be swallowed");
    // Round-trip debug string so a future refactor of the error type
    // still exercises the surface visibly.
    let _ = format!("{err:?}");
    assert_eq!(r.0.len(), 0);
}
