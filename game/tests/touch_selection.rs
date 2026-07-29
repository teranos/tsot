// A finger touches a unit and lifts. Does it get selected, and does
// the bar then offer to become it?
//
// This is the question that was asked three times and that I answered
// with "unknown, look at the device" — because the gesture system read
// `gpu_web::touches()` itself and nothing but a browser could drive it.
// The touches ride in `InputFrame` now, so the whole path from finger
// to selection to action bar runs here.
//
// The bug this file exists to have caught: a tap used to set the left
// button for ONE frame. That frame is the PRESS edge, so it stored an
// anchor and returned; by the release frame `ndc` was back to the mouse
// pointer, which on a phone is None, so the release bailed. A tap could
// never select anything, and no amount of looking at the device would
// have told either of us why.

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use game::actions::{Action, SLOTS};
use game::physics::{Position, Velocity};
use game::rts::{self, ActionBar, CameraFocus, CameraZoom, InputFrame, Selected, UnitMarker};
use game::terrain;

fn on_terrain(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, terrain::height(x, z), z)
}

fn observer_app(offsets: &[(f32, f32)]) -> (App, Vec3, Vec<Entity>) {
    let mut app = App::new();
    rts::register(&mut app);
    let centre = on_terrain(1200.0, -800.0);
    app.insert_resource(CameraFocus::Free(centre));
    let world = app.world_mut();
    let ids = offsets
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

/// One frame with these fingers on the glass.
fn touch_frame(app: &mut App, touches: Vec<[f32; 2]>) {
    *app.world_mut().resource_mut::<InputFrame>() =
        InputFrame { touches, ..Default::default() };
    app.update();
}

fn camera(app: &App) -> game::scene::SceneCamera {
    let CameraFocus::Free(p) = *app.world().resource::<CameraFocus>() else {
        panic!("observer should be detached")
    };
    game::scene::SceneCamera::at(
        [p.x, p.y, p.z],
        app.world().resource::<CameraZoom>().half_extent(),
        game::room::FLOOR_HALF,
    )
}

fn ndc_of(app: &App, p: Vec3) -> [f32; 2] {
    camera(app).world_to_clip([p.x, p.y, p.z])
}

/// Press and lift at the same spot, which is what a tap is.
fn tap(app: &mut App, at: [f32; 2]) {
    touch_frame(app, vec![at]);
    touch_frame(app, vec![at]);
    touch_frame(app, vec![]);
}

fn is_selected(app: &App, e: Entity) -> bool {
    app.world().get::<Selected>(e).is_some()
}

fn bar(app: &App) -> [Option<Action>; SLOTS] {
    app.world().resource::<ActionBar>().0
}

#[test]
fn tapping_a_unit_selects_it() {
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0), (3000.0, 3000.0)]);
    let at = ndc_of(&app, on_terrain(centre.x, centre.z));

    tap(&mut app, at);

    assert!(is_selected(&app, ids[0]), "tapping a unit did not select it");
    assert!(!is_selected(&app, ids[1]), "tapping selected a distant unit too");
}

#[test]
fn tapping_a_unit_reveals_become() {
    // "I don't see the becoming action available" — this is that,
    // end to end: finger down, finger up, and the bar offers the verb.
    let (mut app, centre, _ids) = observer_app(&[(0.0, 0.0)]);
    let at = ndc_of(&app, on_terrain(centre.x, centre.z));

    tap(&mut app, at);

    assert!(
        bar(&app).contains(&Some(Action::Become)),
        "a unit is selected but the bar does not offer to become it: {:?}",
        bar(&app)
    );
}

#[test]
fn tapping_near_a_unit_still_selects_it() {
    // A thumb is not a cursor. Landing a little off must still pick,
    // which is what the pick radius is for.
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0)]);
    let c = ndc_of(&app, on_terrain(centre.x, centre.z));

    tap(&mut app, [c[0] + 0.02, c[1] - 0.015]);

    assert!(is_selected(&app, ids[0]), "a tap a couple of hundredths off missed");
}

#[test]
fn tapping_bare_ground_clears_the_selection() {
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0)]);
    let at = ndc_of(&app, on_terrain(centre.x, centre.z));
    tap(&mut app, at);
    assert!(is_selected(&app, ids[0]));

    tap(&mut app, [-0.85, 0.8]);
    assert!(!is_selected(&app, ids[0]), "tapping empty ground left it selected");
}

#[test]
fn dragging_across_a_unit_pans_and_does_not_select_it() {
    // Drag is pan. If it also selected, moving the camera would
    // constantly change what is selected under your thumb.
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0)]);
    let c = ndc_of(&app, on_terrain(centre.x, centre.z));

    touch_frame(&mut app, vec![c]);
    touch_frame(&mut app, vec![[c[0] + 0.3, c[1]]]);
    touch_frame(&mut app, vec![[c[0] + 0.6, c[1]]]);
    touch_frame(&mut app, vec![]);

    assert!(!is_selected(&app, ids[0]), "a pan across a unit selected it");
    let CameraFocus::Free(now) = *app.world().resource::<CameraFocus>() else {
        panic!("observer detached")
    };
    let moved = ((now.x - centre.x).powi(2) + (now.z - centre.z).powi(2)).sqrt();
    assert!(moved > 1.0, "dragging did not pan the camera: moved {moved:.2}");
}

#[test]
fn a_pinch_zooms_without_selecting() {
    let (mut app, centre, ids) = observer_app(&[(0.0, 0.0)]);
    let before = app.world().resource::<CameraZoom>().half_extent();
    let c = ndc_of(&app, on_terrain(centre.x, centre.z));

    touch_frame(&mut app, vec![[c[0] - 0.1, c[1]], [c[0] + 0.1, c[1]]]);
    touch_frame(&mut app, vec![[c[0] - 0.3, c[1]], [c[0] + 0.3, c[1]]]);
    touch_frame(&mut app, vec![]);

    let after = app.world().resource::<CameraZoom>().half_extent();
    assert!(after < before, "spreading two fingers did not zoom in: {before} -> {after}");
    assert!(!is_selected(&app, ids[0]), "a pinch selected a unit");
}

#[test]
fn a_finger_on_an_action_button_is_not_a_world_gesture() {
    // The bar sits over the world. Pressing it must not also pan.
    let (mut app, centre, _ids) = observer_app(&[]);
    let button = {
        let r = game::actions::slot_rects(game::gpu_web::viewport_size())[1];
        [r.cx, r.cy]
    };
    touch_frame(&mut app, vec![button]);
    touch_frame(&mut app, vec![[button[0] + 0.4, button[1] + 0.4]]);

    let CameraFocus::Free(now) = *app.world().resource::<CameraFocus>() else {
        panic!("observer detached")
    };
    let moved = ((now.x - centre.x).powi(2) + (now.z - centre.z).powi(2)).sqrt();
    assert!(moved < 1.0, "a drag starting on a button panned the camera {moved:.2}");
}
