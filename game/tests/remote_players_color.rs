// Each peer id maps deterministically to a distinct, bright RGB.
// Same id → same color across calls (so the dot doesn't strobe as
// positions update). Different ids → different colors (so eyes can
// tell two remote peers apart).

use game::remote_players::color_for_peer;

#[test]
fn same_peer_yields_same_color() {
    let a = color_for_peer("peer-abc");
    let b = color_for_peer("peer-abc");
    assert_eq!(a, b);
}

#[test]
fn different_peers_yield_different_colors() {
    let a = color_for_peer("peer-abc");
    let b = color_for_peer("peer-xyz");
    assert_ne!(a, b);
}

#[test]
fn every_channel_is_bright_enough_to_see() {
    // Bias floor: any channel below 0.3 blends into the dark
    // background. Deterministic so we can assert the invariant.
    for peer in ["a", "abc", "12D3KooWLongLibp2pLike", "self"] {
        let c = color_for_peer(peer);
        for ch in c {
            assert!(ch >= 0.3, "channel too dark: peer={peer} c={c:?}");
            assert!(ch <= 1.0, "channel over 1.0: peer={peer} c={c:?}");
        }
    }
}
