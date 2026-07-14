//! Same-sleeve (Z.7) intent tests.
//!
//! A same-sleeve card is fused inside its host's sleeve (RULES Z.7, C.4):
//! it cannot be peeled off, targeted, or moved independently of the host,
//! and it leaves play only when the host does. The engine models this as a
//! separate `CardInstance.same_sleeve` child list, distinct from `attached`
//! (Z.6). That separation makes four rules fall out:
//!   - P.8 cascade sweeps `attached` only → fused cards are never exiled.
//!   - P.29 move-with-host: the sleeve is a child field on the host
//!     instance, so it rides every zone move structurally, no follow-logic.
//!   - C.16 / AttachedCount reads `attached` only → fused cards don't count.
//!   - effect/static/event sites read `children()` (the union) so a fused
//!     mutation's statics and handlers still reach the host.

use super::*;
use crate::game::test_helpers::*;

/// Build a host on the BOARD carrying one ordinary attached payment (Z.6)
/// and one fused same-sleeve card (Z.7). Returns (state, host, payment,
/// sleeved).
fn host_with_payment_and_sleeve() -> (GameState, InstanceId, InstanceId, InstanceId) {
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    let payment = s.a.hand[1].clone();
    let sleeved = s.a.hand[2].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&payment, PlayerId::A, Zone::Hand);
    s.add_attached(&host, &payment);
    let _ = s.remove_from_zone(&sleeved, PlayerId::A, Zone::Hand);
    s.add_same_sleeve(&host, &sleeved);
    (s, host, payment, sleeved)
}

#[test]
fn p8_cascade_exiles_attached_payment_but_not_same_sleeve_card() {
    let (mut s, host, payment, sleeved) = host_with_payment_and_sleeve();

    // Host dies: P.8's cascade sweeps remaining `attached` cards to EXILE.
    s.exile_remaining_attached(&host);

    // The ordinary payment is swept to its owner's EXILE (P.8).
    assert!(
        s.a.exile.contains(&payment),
        "P.8: an ordinary attached payment must be exiled when the host dies"
    );
    // The same-sleeve card is fused (Z.7 / P.29): not exiled, still fused.
    assert!(
        !s.a.exile.contains(&sleeved),
        "Z.7/P.29: a fused same-sleeve card must not be exiled by the P.8 cascade"
    );
    assert!(
        s.card_pool
            .get(&host)
            .map(|h| h.same_sleeve.contains(&sleeved))
            .unwrap_or(false),
        "Z.7: a fused same-sleeve card stays fused to the host through the P.8 sweep"
    );
}

#[test]
fn p29_same_sleeve_rides_host_into_every_zone() {
    // The sleeve is a child field on the host instance, so wherever the
    // host's iid travels, the fused card travels with it — no follow-logic.
    for dest in [Zone::Graveyard, Zone::Exile, Zone::Hand, Zone::Deck] {
        let (mut s, host, _payment, sleeved) = host_with_payment_and_sleeve();
        let _ = s.move_card(&host, PlayerId::A, Zone::Board, dest);

        // Host is in the destination zone.
        let host_zone = match dest {
            Zone::Graveyard => &s.a.graveyard,
            Zone::Exile => &s.a.exile,
            Zone::Hand => &s.a.hand,
            Zone::Deck => &s.a.deck,
            Zone::Board => unreachable!(),
        };
        assert!(host_zone.contains(&host), "host moved to {dest:?}");

        // The fused card followed the host: still in its sleeve, and NOT
        // sitting loose in any zone list of its own.
        assert!(
            s.card_pool
                .get(&host)
                .map(|h| h.same_sleeve.contains(&sleeved))
                .unwrap_or(false),
            "Z.7/P.29: same-sleeve card stays fused after host moves to {dest:?}"
        );
        for zone in [&s.a.board, &s.a.graveyard, &s.a.exile, &s.a.hand, &s.a.deck] {
            assert!(
                !zone.contains(&sleeved),
                "P.29: fused card must not appear loose in any zone (dest {dest:?})"
            );
        }
    }
}

#[test]
fn c16_attached_count_excludes_same_sleeve() {
    // A host carries one attached payment and one fused mutation. The unit
    // has two children, but only the payment is "attached" — the count that
    // `ModifierValue::AttachedCount` reads (source.attached.len()) is 1.
    let (s, host, _payment, _sleeved) = host_with_payment_and_sleeve();
    let inst = s.card_pool.get(&host).unwrap();

    assert_eq!(inst.attached.len(), 1, "only the payment is attached (Z.6)");
    assert_eq!(inst.same_sleeve.len(), 1, "the mutation is fused (Z.7)");
    assert_eq!(
        inst.children().count(),
        2,
        "children() unions both for effect/static/event sites"
    );
}

#[test]
fn same_sleeve_add_round_trips_through_journal() {
    // AddSameSleeve / RemoveSameSleeve must invert cleanly, or full-game
    // rollback (the strongest journal test) diverges once a mutation casts.
    let mut s = GameState::new(deck_of(50, "a"), deck_of(50, "b"));
    let host = s.a.hand[0].clone();
    let sleeved = s.a.hand[1].clone();
    let _ = s.move_card(&host, PlayerId::A, Zone::Hand, Zone::Board);
    let _ = s.remove_from_zone(&sleeved, PlayerId::A, Zone::Hand);

    let before = format!("{:?}", s.card_pool.get(&host).unwrap().same_sleeve);
    s.journal = Some(crate::game::Journal::new());
    s.add_same_sleeve(&host, &sleeved);
    assert!(s.card_pool.get(&host).unwrap().same_sleeve.contains(&sleeved));

    let journal = s.journal.take().unwrap();
    journal.rollback(&mut s);
    let after = format!("{:?}", s.card_pool.get(&host).unwrap().same_sleeve);
    assert_eq!(before, after, "AddSameSleeve rolled back to empty sleeve");
}
