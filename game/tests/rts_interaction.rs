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
fn the_wheel_zooms_the_observer() {
    let (mut app, _c, _ids) = observer_app(&[]);
    let before = app.world().resource::<CameraZoom>().half_extent();

    frame(&mut app, InputFrame { wheel_y: 3, ..Default::default() });

    let after = app.world().resource::<CameraZoom>().half_extent();
    assert!(after < before, "wheel-up did not zoom in: {before} -> {after}");
}
