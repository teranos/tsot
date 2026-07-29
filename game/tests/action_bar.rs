// Three action slots. Always three.
//
// The constraint, and what it buys: at any moment there are exactly
// three things you can do, they live in the same corner, and on desktop
// they are the same three keys forever. Everything else about the verb
// surface follows from that — including the answer to "how was I
// supposed to know about E", which is that you were not, and that was
// the bug.
//
// Four properties make the constraint real rather than decorative, and
// each one is a way it quietly stops working:
//
//   1. ALWAYS three. A row that shrinks to two when only two actions
//      apply moves the third button's position, and thumb memory dies.
//      Empty slots stay present and stay empty.
//   2. A verb owns a SLOT, not a position in a list. If "become" is the
//      middle key here and the left key over there, the muscle memory
//      the constraint exists to buy is squandered.
//   3. Contention is resolved by dropping, not by growing. When more
//      than three things could apply, the losers are simply not
//      offered. That is the constraint doing its job: no context may
//      grow a fourth verb without someone deciding what it displaces.
//   4. The bar is a PROJECTION of world state, recomputed, never a
//      stored menu. A stored one goes stale and offers you "chop" after
//      you walked away from the tree.

use game::actions::{self, Action, Affordances, SLOTS};

fn bar(selected: usize, piloting: bool) -> [Option<Action>; SLOTS] {
    actions::resolve(Affordances { selected_count: selected, piloting })
}

/// A representative phone-ish viewport; the maths is resolution
/// relative, so the exact numbers only need to be plausible.
const VIEWPORT: (u32, u32) = (900, 1600);

#[test]
fn tapping_a_button_produces_that_slots_key() {
    // The button and the key are one control. A tap yields the same bit
    // the keyboard yields, so there is exactly one edge detector and one
    // dispatch — not a touch path that can drift from the key path.
    for (slot, r) in actions::slot_rects(VIEWPORT).iter().enumerate() {
        assert_eq!(
            actions::slots_touched(VIEWPORT, &[[r.cx, r.cy]]),
            game::input::key::slot_bit(slot),
            "tapping button {slot} did not produce its key"
        );
    }
}

#[test]
fn a_touch_away_from_the_bar_invokes_nothing() {
    // Most of the screen is the world. Touching it must not fire a verb.
    for p in [[0.0, 0.0], [0.9, 0.9], [-0.9, 0.9], [0.5, -0.95]] {
        assert_eq!(
            actions::slots_touched(VIEWPORT, &[p]),
            0,
            "a touch at {p:?} invoked an action"
        );
    }
}

#[test]
fn the_buttons_do_not_overlap_each_other() {
    // Overlapping rects would fire two verbs from one thumb.
    let rects = actions::slot_rects(VIEWPORT);
    for i in 0..SLOTS {
        for j in (i + 1)..SLOTS {
            let gap = (rects[i].cx - rects[j].cx).abs() - (rects[i].hx + rects[j].hx);
            assert!(gap > 0.0, "buttons {i} and {j} overlap by {:.4} NDC", -gap);
        }
    }
}

#[test]
fn the_bar_sits_in_the_bottom_left_corner() {
    let rects = actions::slot_rects(VIEWPORT);
    for (i, r) in rects.iter().enumerate() {
        assert!(r.cy < -0.5, "button {i} is not near the bottom (cy={})", r.cy);
        assert!(r.cx < 0.0, "button {i} is not in the left half (cx={})", r.cx);
    }
    assert!(
        rects[0].cx < rects[1].cx && rects[1].cx < rects[2].cx,
        "the slots are not in left-to-right order"
    );
}

#[test]
fn a_touch_on_the_world_becomes_a_pointer() {
    // On a phone there is no mouse, so selection has to come from
    // somewhere. A touch that is not on a control IS the pointer —
    // routed into the same `InputFrame` the mouse fills, so tap and
    // drag select through one code path rather than a second one that
    // can drift.
    let p = [0.1, 0.3];
    assert_eq!(
        game::rts::world_touch(VIEWPORT, &[p]),
        Some(p),
        "a touch out in the world did not become the pointer"
    );
}

#[test]
fn a_touch_on_a_control_is_not_a_pointer() {
    // Pressing a button is not aiming at the ground under it. Without
    // this, every tap on an action button would also fire a click into
    // the world behind it, and every D-pad press would drag a selection
    // rectangle across the map.
    let on_button = {
        let r = actions::slot_rects(VIEWPORT)[1];
        [r.cx, r.cy]
    };
    assert_eq!(
        game::rts::world_touch(VIEWPORT, &[on_button]),
        None,
        "tapping an action button also aimed at the world behind it"
    );

}

#[test]
fn no_touches_means_no_pointer() {
    assert_eq!(game::rts::world_touch(VIEWPORT, &[]), None);
}

#[test]
fn with_nothing_selected_the_readout_says_how_to_select() {
    // "not immediately obvious how to do it" — so the screen says how.
    // An empty readout, or one that just says zero, answers neither
    // "what do I do" nor "did it work".
    let s = actions::selection_readout(0);
    assert!(!s.is_empty(), "nothing on screen tells you how to select");
    assert!(
        s.contains("tap"),
        "the idle readout does not name the gesture: {s:?}"
    );
}

#[test]
fn the_readout_counts_what_is_selected() {
    // "not obvious if I'm even selecting anything" — so the count is on
    // screen. This is the only thing that can answer that question
    // without guessing.
    assert!(actions::selection_readout(1).contains('1'));
    assert!(actions::selection_readout(7).contains('7'));
}

#[test]
fn the_readout_is_always_drawable() {
    for n in [0, 1, 2, 9, 12, 100] {
        for c in actions::selection_readout(n).chars() {
            assert!(
                c == ' ' || game::watermark::glyph(c).is_some(),
                "readout for {n} contains {c:?}, which the font cannot draw"
            );
        }
    }
}

#[test]
fn a_drag_in_progress_draws_its_rectangle() {
    // "not obvious if I'm doing it correctly" — you cannot see a
    // rectangle that is never drawn. Four edges, so it reads as a box
    // rather than a filled slab over the world.
    let quads = actions::drag_rect_quads([-0.4, -0.2], [0.3, 0.5]);
    assert_eq!(quads.len(), 4, "expected four edges, got {}", quads.len());

    // Dragged the other way, the same box.
    let flipped = actions::drag_rect_quads([0.3, 0.5], [-0.4, -0.2]);
    let key = |q: &[game::dpad::DpadInstance]| {
        let mut v: Vec<[i32; 4]> = q
            .iter()
            .map(|i| {
                [
                    (i.center_ndc[0] * 1e4) as i32,
                    (i.center_ndc[1] * 1e4) as i32,
                    (i.half_size_ndc[0] * 1e4) as i32,
                    (i.half_size_ndc[1] * 1e4) as i32,
                ]
            })
            .collect();
        v.sort();
        v
    };
    assert_eq!(key(&quads), key(&flipped), "the box depends on drag direction");
}

#[test]
fn a_drag_rectangle_stays_inside_the_corners_it_was_given() {
    let (a, b) = ([-0.4, -0.2], [0.3, 0.5]);
    for q in actions::drag_rect_quads(a, b) {
        let (lo_x, hi_x) = (a[0].min(b[0]), a[0].max(b[0]));
        let (lo_y, hi_y) = (a[1].min(b[1]), a[1].max(b[1]));
        assert!(
            q.center_ndc[0] >= lo_x - 0.01 && q.center_ndc[0] <= hi_x + 0.01,
            "edge escaped the box in x: {:?}",
            q.center_ndc
        );
        assert!(
            q.center_ndc[1] >= lo_y - 0.01 && q.center_ndc[1] <= hi_y + 0.01,
            "edge escaped the box in y: {:?}",
            q.center_ndc
        );
    }
}

#[test]
fn there_are_always_exactly_three_slots() {
    // Not "up to three". The array's length is the constraint, and it
    // holds in the emptiest state there is.
    for (selected, piloting) in [(0, false), (1, false), (5, false), (0, true), (1, true)] {
        let b = bar(selected, piloting);
        assert_eq!(
            b.len(),
            3,
            "selected={selected} piloting={piloting} gave {} slots",
            b.len()
        );
    }
}

#[test]
fn an_empty_context_still_has_three_empty_slots() {
    let b = bar(0, false);
    assert!(
        b.iter().all(Option::is_none),
        "nothing is selected and nothing is held, yet the bar offered {b:?}"
    );
}

#[test]
fn selecting_one_body_reveals_become() {
    // The fix for the actual bug: the verb announces itself instead of
    // being a key you had to be told about.
    let b = bar(1, false);
    assert!(
        b.contains(&Some(Action::Become)),
        "selected a becomeable body and the bar did not offer to become it: {b:?}"
    );
}

#[test]
fn become_needs_exactly_one_body() {
    // Becoming a squad is not a thing. With two selected the verb is
    // not offered, rather than silently picking one of them.
    assert!(
        !bar(0, false).contains(&Some(Action::Become)),
        "become offered with nothing selected"
    );
    assert!(
        !bar(4, false).contains(&Some(Action::Become)),
        "become offered for a whole squad — which one did it mean?"
    );
}

#[test]
fn being_in_a_body_swaps_become_for_leave() {
    let b = bar(1, true);
    assert!(
        b.contains(&Some(Action::Leave)),
        "driving a body and the bar did not offer a way out: {b:?}"
    );
    assert!(
        !b.contains(&Some(Action::Become)),
        "offered to become a body while already in one"
    );
}

#[test]
fn a_verb_keeps_its_slot_across_contexts() {
    // Property 2, stated as the thing that would break it: become and
    // leave are the same verb pointing opposite ways, so they must land
    // on the same key. Otherwise stepping in and out of a body means
    // reaching for a different button each time.
    let becoming = bar(1, false)
        .iter()
        .position(|a| *a == Some(Action::Become))
        .expect("become is offered");
    let leaving = bar(1, true)
        .iter()
        .position(|a| *a == Some(Action::Leave))
        .expect("leave is offered");
    assert_eq!(
        becoming, leaving,
        "become sits in slot {becoming} but leave sits in slot {leaving} — \
         the same verb moved under the hand"
    );
}

#[test]
fn every_offered_action_sits_in_the_slot_it_declares() {
    // The resolver may not shuffle actions into whatever slot is free.
    // A verb's slot is a property of the verb.
    for (selected, piloting) in [(0, false), (1, false), (3, false), (1, true), (0, true)] {
        for (i, slot) in bar(selected, piloting).iter().enumerate() {
            if let Some(a) = slot {
                assert_eq!(
                    a.slot(),
                    i,
                    "{a:?} was placed in slot {i} but declares slot {}",
                    a.slot()
                );
            }
        }
    }
}

#[test]
fn every_offered_action_can_be_labelled() {
    // The bar is drawn with the 3x5 pixel font, which knows lowercase,
    // digits and a little punctuation. A verb whose label cannot be
    // drawn is a blank button — worse than no button, because it looks
    // like something is broken.
    for (selected, piloting) in [(1, false), (1, true), (2, false)] {
        for a in bar(selected, piloting).iter().flatten() {
            let label = a.label();
            assert!(!label.is_empty(), "{a:?} has no label");
            for c in label.chars() {
                assert!(
                    game::watermark::glyph(c).is_some(),
                    "{a:?} label {label:?} contains {c:?}, which the font cannot draw"
                );
            }
        }
    }
}
