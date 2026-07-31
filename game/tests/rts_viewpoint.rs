// Where the world is centred.
//
// Everything expensive is built around one point: chunk streaming
// decides which trees and props exist, the terrain surface mesh is
// generated around it, buildings are baked near it, and roofs cut away
// over it. All of that was anchored to the PLAYER, which was correct
// while the camera was leashed to the player and became wrong the
// moment the observer could fly.
//
// The symptom is not subtle and it is not a rendering nicety: pan away
// and the world stops existing. Ground, trees and buildings simply are
// not there, because nothing generated them — they are generated near
// the avatar you left behind.
//
// So the world follows the VIEWPOINT: the focus when detached, the
// body when following one. One point, one resource, read by everything
// that decides what exists.

use bevy_app::App;
use bevy_math::Vec3;

use game::input::key;
use game::physics::{PlayerMarker, Position, Velocity};
use game::rts::{self, CameraFocus, InputFrame, UnitMarker, Viewpoint};
use game::terrain;

fn on_terrain(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, terrain::height(x, z), z)
}

fn app_at(centre: Vec3) -> App {
    let mut app = App::new();
    rts::register(&mut app);
    app.insert_resource(CameraFocus::Free(centre));
    app
}

fn frame(app: &mut App, f: InputFrame) {
    *app.world_mut().resource_mut::<InputFrame>() = f;
    app.update();
}

fn viewpoint(app: &App) -> Vec3 {
    app.world().resource::<Viewpoint>().0
}

fn xz_dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

#[test]
fn the_viewpoint_tracks_a_detached_observer() {
    let centre = on_terrain(1200.0, -800.0);
    let mut app = app_at(centre);
    frame(&mut app, InputFrame::default());
    assert!(
        xz_dist(viewpoint(&app), centre) < 1e-3,
        "the viewpoint did not start at the observer's focus"
    );

    for _ in 0..30 {
        frame(&mut app, InputFrame { keys: key::W, ..Default::default() });
    }

    let moved = xz_dist(viewpoint(&app), centre);
    assert!(
        moved > 100.0,
        "panned 30 ticks and the viewpoint moved {moved:.1} — the world is \
         still being built somewhere else"
    );
}

#[test]
fn the_viewpoint_ignores_the_player_it_left_behind() {
    // The regression stated directly. A player parked far from the
    // observer must not drag the world's centre back to itself.
    let centre = on_terrain(1200.0, -800.0);
    let mut app = app_at(centre);
    let far = on_terrain(-9000.0, 9000.0);
    app.world_mut()
        .spawn((PlayerMarker, Position(far), Velocity(Vec3::ZERO)));

    frame(&mut app, InputFrame::default());

    assert!(
        xz_dist(viewpoint(&app), centre) < 1e-3,
        "the viewpoint followed the abandoned player instead of the observer"
    );
    assert!(
        xz_dist(viewpoint(&app), far) > 1000.0,
        "the viewpoint is sitting on the player — panning will show empty world"
    );
}

#[test]
fn the_viewpoint_follows_the_body_you_became() {
    let centre = on_terrain(1200.0, -800.0);
    let mut app = app_at(centre);
    let body_at = on_terrain(centre.x + 2000.0, centre.z - 1500.0);
    let body = app
        .world_mut()
        .spawn((UnitMarker, Position(body_at), Velocity(Vec3::ZERO)))
        .id();

    rts::become_pilot(app.world_mut(), body);
    *app.world_mut().resource_mut::<CameraFocus>() = CameraFocus::Following(body);
    frame(&mut app, InputFrame::default());

    assert!(
        xz_dist(viewpoint(&app), body_at) < 1e-3,
        "the world is not centred on the body being driven"
    );
}

#[test]
fn the_snapshot_carries_the_viewpoint() {
    // The render path reads the snapshot, so the viewpoint has to reach
    // it — otherwise the terrain mesh and wall bakes keep being built
    // around the player no matter what the camera does.
    let centre = on_terrain(1200.0, -800.0);
    let mut app = app_at(centre);
    let far = on_terrain(-9000.0, 9000.0);
    app.world_mut()
        .spawn((PlayerMarker, Position(far), Velocity(Vec3::ZERO)));
    frame(&mut app, InputFrame::default());

    let snap = game::scene::snapshot_scene(&mut app);
    assert!(
        xz_dist(snap.viewpoint, centre) < 1e-3,
        "snapshot.viewpoint is not the observer's point"
    );
    assert!(
        xz_dist(snap.viewpoint, snap.player) > 1000.0,
        "snapshot.viewpoint collapsed onto the player"
    );
}
