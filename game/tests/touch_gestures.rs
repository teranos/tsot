// What the fingers did.
//
// A touch array says where fingers ARE. Every gesture is a claim about
// what they are DOING, which only exists across frames — so this is the
// piece that has to carry state, and the piece where a phone and a
// headless test can be made to agree.
//
// Pure by construction: the touch positions are arguments. Every rule
// below is checkable without a device, which matters because the device
// is exactly what I cannot see.

use game::rts::{self, TouchGesture, TouchState, TAP_SLOP_NDC};

fn read(state: &mut TouchState, touches: &[[f32; 2]]) -> TouchGesture {
    rts::read_touch_gesture(state, touches)
}

#[test]
fn a_finger_that_barely_moves_is_a_tap() {
    // Selection is a tap. If a resting thumb's jitter counted as a
    // drag, tapping a unit would pan the camera instead of selecting.
    let mut s = TouchState::default();
    read(&mut s, &[[0.10, 0.20]]);
    read(&mut s, &[[0.104, 0.203]]);
    assert_eq!(
        read(&mut s, &[]),
        TouchGesture::Tap { at: [0.104, 0.203] },
        "a finger that moved a few thousandths and lifted was not a tap"
    );
}

#[test]
fn a_finger_that_travels_is_a_pan_and_never_a_tap() {
    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    let g = read(&mut s, &[[0.30, 0.0]]);
    assert!(
        matches!(g, TouchGesture::Pan { .. }),
        "a finger crossing a third of the screen was not a pan: {g:?}"
    );
    assert_eq!(
        read(&mut s, &[]),
        TouchGesture::None,
        "lifting after a pan also fired a tap — the camera would move AND select"
    );
}

#[test]
fn a_pan_reports_this_frames_movement_not_the_total() {
    // The camera follows the finger frame by frame. Reporting the whole
    // travel each frame would accelerate away.
    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    read(&mut s, &[[0.20, 0.0]]);
    let g = read(&mut s, &[[0.25, 0.10]]);
    match g {
        TouchGesture::Pan { dx, dy } => {
            assert!((dx - 0.05).abs() < 1e-5, "dx was {dx}, expected this frame's 0.05");
            assert!((dy - 0.10).abs() < 1e-5, "dy was {dy}, expected this frame's 0.10");
        }
        other => panic!("expected a pan, got {other:?}"),
    }
}

#[test]
fn the_first_frame_of_a_touch_moves_nothing() {
    // There is no previous position to subtract, so inventing a delta
    // would jump the camera by wherever the finger landed.
    let mut s = TouchState::default();
    assert_eq!(read(&mut s, &[[0.7, -0.4]]), TouchGesture::None);
}

#[test]
fn two_fingers_spreading_zoom_in_and_closing_zoom_out() {
    let mut s = TouchState::default();
    read(&mut s, &[[-0.10, 0.0], [0.10, 0.0]]);
    let out = read(&mut s, &[[-0.30, 0.0], [0.30, 0.0]]);
    match out {
        TouchGesture::Pinch { notches } => assert!(notches > 0, "spreading gave {notches}"),
        other => panic!("expected a pinch, got {other:?}"),
    }

    let mut s = TouchState::default();
    read(&mut s, &[[-0.30, 0.0], [0.30, 0.0]]);
    let inn = read(&mut s, &[[-0.10, 0.0], [0.10, 0.0]]);
    match inn {
        TouchGesture::Pinch { notches } => assert!(notches < 0, "closing gave {notches}"),
        other => panic!("expected a pinch, got {other:?}"),
    }
}

#[test]
fn a_second_finger_does_not_pan() {
    // Putting a second finger down must not be read as the first one
    // teleporting. That would lurch the camera at the start of every
    // pinch.
    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    read(&mut s, &[[0.05, 0.0]]);
    let g = read(&mut s, &[[0.05, 0.0], [0.60, 0.30]]);
    assert!(
        !matches!(g, TouchGesture::Pan { .. }),
        "adding a finger produced a pan: {g:?}"
    );
}

#[test]
fn lifting_one_finger_from_a_pinch_does_not_pan_or_tap() {
    // Fingers never come up together. The frame where one leaves must
    // not be read as the other suddenly dragging, nor as a tap.
    let mut s = TouchState::default();
    read(&mut s, &[[-0.20, 0.0], [0.20, 0.0]]);
    read(&mut s, &[[-0.30, 0.0], [0.30, 0.0]]);
    let g = read(&mut s, &[[0.30, 0.0]]);
    assert_eq!(g, TouchGesture::None, "dropping to one finger produced {g:?}");
    assert_eq!(
        read(&mut s, &[]),
        TouchGesture::None,
        "ending a pinch fired a tap"
    );
}

#[test]
fn a_gesture_does_not_leak_into_the_next_one() {
    // State is per-gesture. A tap right after a long pan must still be
    // a tap, not a continuation of the travel that disqualified it.
    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    read(&mut s, &[[0.5, 0.5]]);
    read(&mut s, &[]);

    read(&mut s, &[[-0.2, -0.2]]);
    assert!(
        matches!(read(&mut s, &[]), TouchGesture::Tap { .. }),
        "the previous pan's travel disqualified a fresh tap"
    );
}

#[test]
fn the_slop_threshold_is_the_boundary_it_says_it_is() {
    // Just under: tap. Well over: pan. Named so the constant cannot
    // drift away from the behaviour without this failing.
    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    read(&mut s, &[[TAP_SLOP_NDC * 0.5, 0.0]]);
    assert!(matches!(read(&mut s, &[]), TouchGesture::Tap { .. }));

    let mut s = TouchState::default();
    read(&mut s, &[[0.0, 0.0]]);
    read(&mut s, &[[TAP_SLOP_NDC * 4.0, 0.0]]);
    assert_eq!(read(&mut s, &[]), TouchGesture::None);
}
