//! Same-sleeve (Z.7) intent tests.
//!
//! A same-sleeve card is fused inside its host's sleeve (RULES Z.7, C.4):
//! it cannot be peeled off, targeted, or moved independently of the host,
//! and it leaves play ONLY when the host does. In particular RULES P.29:
//! a same-sleeve mutation is NOT swept to EXILE by P.8's attached-cascade
//! when the host dies — it is fused, not merely attached.
//!
//! This is the first slice: it captures the sharpest behavioural
//! discriminator between an ordinary attached payment (P.31, strippable,
//! exiled by the cascade) and a fused same-sleeve mutation (Z.7, stays
//! with the host).

use super::*;
use crate::game::test_helpers::*;

#[test]
fn p8_cascade_exiles_attached_payment_but_not_same_sleeve_mutation() {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));

    // Host creature on the BOARD, carrying two attached cards:
    //   - `payment`: an ordinary P.31 attached payment (strippable).
    //   - `mutation`: a fused same-sleeve mutation (Z.7).
    let host = s.a.hand[0].clone();
    let payment = s.a.hand[1].clone();
    let mutation = s.a.hand[2].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    for a in [&payment, &mutation] {
        let _ = s.remove_from_zone(a, PlayerId::A, Zone::Hand);
        s.add_attached(&host, a);
    }
    // Mark the mutation as fused inside the host's sleeve.
    s.card_pool.get_mut(&mutation).unwrap().card.same_sleeve = true;

    // Host dies: P.8's cascade sweeps remaining attached cards to EXILE.
    s.exile_remaining_attached(&host);

    // The ordinary payment is swept to its owner's EXILE (P.8).
    assert!(
        s.a.exile.contains(&payment),
        "P.8: an ordinary attached payment must be exiled when the host dies"
    );

    // The same-sleeve mutation is fused (Z.7 / P.29): it must NOT be
    // exiled by the cascade — it leaves play only with the host, and so
    // remains fused to the host after the sweep.
    assert!(
        !s.a.exile.contains(&mutation),
        "Z.7/P.29: a fused same-sleeve mutation must not be exiled by the P.8 cascade"
    );
    assert!(
        s.card_pool
            .get(&host)
            .map(|h| h.attached.contains(&mutation))
            .unwrap_or(false),
        "Z.7: a fused same-sleeve mutation stays with the host through the P.8 sweep"
    );
}
