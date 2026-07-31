// The interaction layer: what a human's hand actually does.
//
// Drag the left button to select. Right-click to send the selection
// there. WASD moves the viewport. The wheel zooms. Each of those is a
// rule about EDGES — the frame a button went down, the frame it came
// up — derived from a level that only says what is held right now.
//
// Driven entirely by writing `InputFrame` and stepping the app. No
// env.*, no JS, no browser: `pump_input_frame` is the single place the
// outside world gets in, and on native it leaves the resource alone, so
// these tests drive the exact frames they mean. When this passes and
// the browser still misbehaves, the shim is the suspect and this file
// is the alibi.

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use game::input::key;
use game::physics::{Position, Velocity};
use game::rts::{
    self, button, CameraFocus, CameraZoom, InputFrame, MoveOrder, Piloted, Selected,
    UnitMarker,
};
use game::terrain;

fn on_terrain(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, terrain::height(x, z), z)
}

/// An app focused on a known patch of ground, with units placed
/// relative to that focus so screen-space reasoning is stable.
fn observer_app(unit_offsets: &[(f32, f32)]) -> (App, Vec3, Vec<Entity>) {
    let mut app = App::new();
    rts::register(&mut app);
    let centre = on_terrain(1200.0, -800.0);
    app.insert_resource(CameraFocus::Free(centre));
    let world = app.world_mut();
    let ids = unit_offsets
        .iter()
        .map(|&(dx, dz)| {
            world
                .spawn((
                    UnitMarker,
                    Position(on_terrain(centre.x + dx, centre.z + dz)),
                    Velocity(Vec3::ZERO),
                ))
                .id()
        })
        .collect();
    (app, centre, ids)
}

/// Step one frame with an exact input state.
fn frame(app: &mut App, f: InputFrame) {
    *app.world_mut().resource_mut::<InputFrame>() = f;
    app.update();
}

/// Press left at `a`, drag to `b`, release. Three frames, because a
/// drag is three edges and collapsing them would test something else.
fn drag(app: &mut App, a: [f32; 2], b: [f32; 2]) {
    frame(app, InputFrame { ndc: Some(a), buttons: button::LEFT, ..Default::default() });
    frame(app, InputFrame { ndc: Some(b), buttons: button::LEFT, ..Default::default() });
    frame(app, InputFrame { ndc: Some(b), buttons: 0, ..Default::default() });
}

fn right_click(app: &mut App, at: [f32; 2]) {
    frame(app, InputFrame { ndc: Some(at), buttons: 0, ..Default::default() });
    frame(app, InputFrame { ndc: Some(at), buttons: button::RIGHT, ..Default::default() });
}

/// The camera the interaction systems are using this frame.
fn observer_cam(app: &App) -> game::scene::SceneCamera {
    let CameraFocus::Free(p) = *app.world().resource::<CameraFocus>() else {
        panic!("observer should be detached")
    };
    game::scene::SceneCamera::at(
        [p.x, p.y, p.z],
        app.world().resource::<CameraZoom>().half_extent(),
        game::room::FLOOR_HALF,
    )
}

/// Where a world position lands on screen, for aiming a click at it.
fn ndc_of(app: &App, p: Vec3) -> [f32; 2] {
    observer_cam(app).world_to_clip([p.x, p.y, p.z])
}

/// A click aimed NEAR a unit rather than exactly at its projected
/// centre — which is what a hand does, and what stops these tests
/// passing for the wrong reason.
///
/// Aimed dead centre, a click's zero-area rectangle contains the unit
/// by exact float equality (the test computes the same NDC the
/// selection does), so rect-only selection appears to work and the
/// pick radius is never exercised. Off by a couple of hundredths, the
/// rectangle contains nothing and only a real proximity test passes.
fn click_near(app: &mut App, unit: Vec3, offset: [f32; 2]) {
    let c = ndc_of(app, unit);
    let at = [c[0] + offset[0], c[1] + offset[1]];
    click(app, at);
}

/// Press and release at the same point: a click, not a drag.
fn click(app: &mut App, at: [f32; 2]) {
    frame(app, InputFrame { ndc: Some(at), buttons: button::LEFT, ..Default::default() });
    frame(app, InputFrame { ndc: Some(at), buttons: 0, ..Default::default() });
}

fn pos_of(app: &App, e: Entity) -> Vec3 {
    app.world().get::<Position>(e).expect("entity has a Position").0
}

fn is_selected(app: &mut App, e: Entity) -> bool {
    app.world().get::<Selected>(e).is_some()
}

fn order_of(app: &mut App, e: Entity) -> Option<Vec3> {
    app.world().get::<MoveOrder>(e).map(|o| o.0)
}

#[test]
fn a_left_drag_selects_what_its_rectangle_covered() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0), (60.0, 60.0), (4000.0, 4000.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);

    assert!(is_selected(&mut app, ids[0]), "a unit under the rectangle was not selected");
    assert!(is_selected(&mut app, ids[1]), "a unit under the rectangle was not selected");
    assert!(
        !is_selected(&mut app, ids[2]),
        "a unit far outside the rectangle was selected anyway"
    );
}

#[test]
fn a_drag_is_not_resolved_until_the_button_comes_up() {
    // Selection lands on release, not while the hand is still moving.
    // Resolving per-frame would make the set flicker through everything
    // the rectangle swept over on its way.
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    frame(&mut app, InputFrame {
        ndc: Some([-0.4, -0.4]),
        buttons: button::LEFT,
        ..Default::default()
    });
    frame(&mut app, InputFrame {
        ndc: Some([0.4, 0.4]),
        buttons: button::LEFT,
        ..Default::default()
    });
    assert!(!is_selected(&mut app, ids[0]), "selection resolved mid-drag");

    frame(&mut app, InputFrame { ndc: Some([0.4, 0.4]), buttons: 0, ..Default::default() });
    assert!(is_selected(&mut app, ids[0]), "selection did not resolve on release");
}

#[test]
fn a_new_drag_replaces_the_previous_selection() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0), (4000.0, 4000.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    assert!(is_selected(&mut app, ids[0]));

    // A drag over empty ground far from anything.
    drag(&mut app, [-0.95, -0.95], [-0.9, -0.9]);
    assert!(
        !is_selected(&mut app, ids[0]),
        "the previous selection survived a new drag — selections accumulate forever"
    );
}

#[test]
fn a_right_click_sends_the_selection_to_the_ground_under_the_cursor() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    assert!(is_selected(&mut app, ids[0]));

    let target_ndc = [0.5, -0.3];
    let cam = game::scene::SceneCamera::at(
        {
            let CameraFocus::Free(p) = *app.world().resource::<CameraFocus>() else {
                panic!("observer should be detached")
            };
            [p.x, p.y, p.z]
        },
        app.world().resource::<CameraZoom>().half_extent(),
        game::room::FLOOR_HALF,
    );
    let want = rts::ground_under(&cam, target_ndc).expect("cursor is over ground");

    right_click(&mut app, target_ndc);

    let got = order_of(&mut app, ids[0]).expect("selected unit received no order");
    let err = ((got.x - want.x).powi(2) + (got.z - want.z).powi(2)).sqrt();
    assert!(err < 1.0, "order landed {err:.2} units from the cursor's ground point");
}

#[test]
fn a_right_click_with_nothing_selected_orders_nothing() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    right_click(&mut app, [0.5, -0.3]);
    assert_eq!(
        order_of(&mut app, ids[0]),
        None,
        "an unselected unit was ordered by a right-click it had nothing to do with"
    );
}

#[test]
fn a_right_click_cannot_order_the_body_being_piloted() {
    // The authority invariant, defended at the source rather than only
    // filtered downstream. `tests/rts_authority.rs` proves a stray order
    // cannot steal a piloted body; this proves the ordinary path never
    // creates one in the first place — right-clicking while embodied is
    // exactly how a human would try.
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    assert!(is_selected(&mut app, ids[0]));

    rts::become_pilot(app.world_mut(), ids[0]);
    right_click(&mut app, [0.5, -0.3]);

    assert!(app.world().get::<Piloted>(ids[0]).is_some(), "the body left the seat");
    assert_eq!(
        order_of(&mut app, ids[0]),
        None,
        "a right-click gave the piloted body an order — two drivers again"
    );
}

#[test]
fn a_click_selects_the_unit_under_it() {
    // A click is a zero-area rectangle, so rect-only selection picks
    // nothing — and clicking one unit is the most ordinary thing a hand
    // does, on a mouse and on a finger alike. Below a slop threshold
    // the gesture is a click and resolves by proximity instead.
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0), (3000.0, 3000.0)]);
    let target = on_terrain(centre.x, centre.z);

    click_near(&mut app, target, [0.015, 0.015]);

    assert!(is_selected(&mut app, ids[0]), "clicking a unit did not select it");
    assert!(!is_selected(&mut app, ids[1]), "clicking one unit selected a distant one");
}

#[test]
fn a_click_on_empty_ground_clears_the_selection() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    assert!(is_selected(&mut app, ids[0]));

    click(&mut app, [-0.9, 0.85]);
    assert!(
        !is_selected(&mut app, ids[0]),
        "clicking bare ground left the old selection standing"
    );
}

#[test]
fn a_click_takes_the_nearest_unit_not_every_nearby_one() {
    // Click selects ONE. Two units inside the same pick radius must not
    // both come along, or a click quietly becomes a small drag.
    // The two project about 0.031 apart in NDC. Aiming 0.01 off the
    // first puts BOTH inside the pick radius, with the first nearer —
    // so "take the nearest" and "take everything close" give different
    // answers, which is the only way this test means anything.
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0), (55.0, 55.0)]);
    let first = on_terrain(centre.x, centre.z);

    click_near(&mut app, first, [0.0, -0.01]);

    assert!(is_selected(&mut app, ids[0]), "the nearest unit was not selected");
    assert!(!is_selected(&mut app, ids[1]), "a click dragged in a second unit");
}

#[test]
fn wasd_pans_a_detached_observer() {
    let (mut app, centre, _ids) = observer_app(&[]);
    frame(&mut app, InputFrame { keys: key::W, ..Default::default() });

    let CameraFocus::Free(after) = *app.world().resource::<CameraFocus>() else {
        panic!("observer should still be detached")
    };
    let moved = ((after.x - centre.x).powi(2) + (after.z - centre.z).powi(2)).sqrt();
    assert!(moved > 0.0, "holding W did not move the viewport");
}

#[test]
fn wasd_does_not_pan_while_following_a_body() {
    // Becoming a body hands those keys to the body. A viewport that
    // also panned would slide the camera off whatever you are driving.
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    rts::become_pilot(app.world_mut(), ids[0]);
    *app.world_mut().resource_mut::<CameraFocus>() = CameraFocus::Following(ids[0]);

    frame(&mut app, InputFrame { keys: key::W, ..Default::default() });

    assert_eq!(
        *app.world().resource::<CameraFocus>(),
        CameraFocus::Following(ids[0]),
        "panning knocked the observer off the body it was following"
    );
}

#[test]
fn become_takes_the_selected_body_and_follows_it() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    assert!(is_selected(&mut app, ids[0]));

    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });

    assert!(
        app.world().get::<Piloted>(ids[0]).is_some(),
        "pressing become did not seat the human in the selected body"
    );
    assert_eq!(
        *app.world().resource::<CameraFocus>(),
        CameraFocus::Following(ids[0]),
        "the observer did not follow the body it became"
    );
}

#[test]
fn become_again_steps_out_where_the_body_stands() {
    // Unbecoming parks the detached observer on the body it left, so
    // stepping out does not cut the view to somewhere else.
    let (mut app, centre, ids) = observer_app(&[(400.0, -300.0)]);
    drag(&mut app, [-0.9, -0.9], [0.9, 0.9]);
    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });
    assert!(app.world().get::<Piloted>(ids[0]).is_some());

    // Released, then pressed again — a level held across frames is one
    // press, not many.
    frame(&mut app, InputFrame::default());
    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });

    assert!(app.world().get::<Piloted>(ids[0]).is_none(), "the human stayed seated");
    let CameraFocus::Free(at) = *app.world().resource::<CameraFocus>() else {
        panic!("stepping out did not detach the observer")
    };
    let body = pos_of(&app, ids[0]);
    assert!(
        ((at.x - body.x).powi(2) + (at.z - body.z).powi(2)).sqrt() < 1.0,
        "the observer parked at {at:?}, not on the body at {body:?}"
    );
    assert!((at.x - centre.x).abs() > 1.0, "the observer snapped back to where it started");
}

#[test]
fn become_does_nothing_without_a_selection() {
    let (mut app, centre, _ids) = observer_app(&[(0.0, 0.0)]);
    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });

    assert_eq!(
        *app.world().resource::<CameraFocus>(),
        CameraFocus::Free(centre),
        "become with nothing selected moved the observer anyway"
    );
}

#[test]
fn the_become_key_is_the_slot_the_verb_declares() {
    // The button and the key are the same control. If the verb moved to
    // another slot, the key that invokes it must move with it — this
    // ties the two together so they cannot drift apart silently.
    use game::actions::{self, Action, Affordances};
    let bar = actions::resolve(Affordances { selected_count: 1, piloting: false, selecting: false });
    let slot = bar.iter().position(|a| *a == Some(Action::Become)).expect("become offered");
    assert_eq!(
        game::input::key::slot_bit(slot),
        key::SLOT_B,
        "become sits in slot {slot}, whose key is not the one the tests press"
    );
}

#[test]
fn holding_become_does_not_flip_every_frame() {
    // The key is a level too. Acting on it rather than on its press
    // edge would toggle in and out of the body sixty times a second.
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    for _ in 0..7 {
        frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });
    }
    assert!(
        app.world().get::<Piloted>(ids[0]).is_some(),
        "holding the key toggled the seat instead of taking it once"
    );
}

#[test]
fn wasd_drives_the_body_you_became() {
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });
    let before = pos_of(&app, ids[0]);

    for _ in 0..20 {
        frame(&mut app, InputFrame { keys: key::W, ..Default::default() });
    }

    let after = pos_of(&app, ids[0]);
    let moved = ((after.x - before.x).powi(2) + (after.z - before.z).powi(2)).sqrt();
    assert!(moved > 10.0, "holding W moved the body {moved:.1} units — nobody is driving");
}

#[test]
fn an_unpiloted_unit_is_not_driven_by_the_keyboard() {
    // WASD reaches exactly one body: the one you are in. Otherwise a
    // keypress marches the whole world.
    let (mut app, _c, ids) = observer_app(&[(0.0, 0.0), (300.0, 0.0)]);
    drag(&mut app, [-0.4, -0.4], [0.4, 0.4]);
    frame(&mut app, InputFrame { keys: key::SLOT_B, ..Default::default() });
    let before = pos_of(&app, ids[1]);

    for _ in 0..20 {
        frame(&mut app, InputFrame { keys: key::W, ..Default::default() });
    }

    let after = pos_of(&app, ids[1]);
    let moved = ((after.x - before.x).powi(2) + (after.z - before.z).powi(2)).sqrt();
    assert!(moved < 1e-3, "a unit nobody is piloting walked {moved:.3} units");
}

#[test]
fn the_wheel_zooms_the_observer() {
    let (mut app, _c, _ids) = observer_app(&[]);
    let before = app.world().resource::<CameraZoom>().half_extent();

    frame(&mut app, InputFrame { wheel_y: 3, ..Default::default() });

    let after = app.world().resource::<CameraZoom>().half_extent();
    assert!(after < before, "wheel-up did not zoom in: {before} -> {after}");
}
