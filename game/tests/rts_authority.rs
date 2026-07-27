// The authority invariant: who is driving this entity, and can only
// one thing drive it at a time.
//
// Two authorities exist. An *ordered* entity is steered from a
// destination it was given and keeps going without being told again.
// A *piloted* entity is the one the human has become, driven by held
// input, here and now. The seam between them is the whole of "I can
// become a movable entity, and also unbecome into an omnipresent
// observer" — and a seam is where the bug lives.
//
// The failure this file exists to forbid: you take the wheel and
// something else is still steering. A body that walks off toward a
// destination you gave it thirty seconds ago while you are holding W
// is not a movement bug, it is two drivers, and no amount of tuning
// the steering fixes it.
//
// Headless: no camera, no mouse, no input plumbing. Those come next
// and they all read this state — which is why it gets pinned first.

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use game::physics::{Position, Velocity};
use game::rts::{self, MoveOrder, Piloted, UnitMarker};

/// Enough ticks that anything still being steered has visibly moved.
const TICKS: u32 = 200;

fn spawn_unit(app: &mut App, at: Vec3, order: Option<Vec3>) -> Entity {
    let world = app.world_mut();
    let mut e = world.spawn((UnitMarker, Position(at), Velocity(Vec3::ZERO)));
    if let Some(o) = order {
        e.insert(MoveOrder(o));
    }
    e.id()
}

fn app_with_rts() -> App {
    let mut app = App::new();
    rts::register(&mut app);
    app
}

fn run(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.update();
    }
}

fn xz_dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

fn pos(app: &mut App, e: Entity) -> Vec3 {
    app.world().get::<Position>(e).expect("entity has a Position").0
}

fn has_order(app: &mut App, e: Entity) -> bool {
    app.world().get::<MoveOrder>(e).is_some()
}

fn is_piloted(app: &mut App, e: Entity) -> bool {
    app.world().get::<Piloted>(e).is_some()
}

#[test]
#[ignore = "RED: needs rts::become_pilot to seat the pilot and clear the order"]
fn becoming_takes_the_wheel_and_clears_the_standing_order() {
    let mut app = app_with_rts();
    let e = spawn_unit(&mut app, Vec3::ZERO, Some(Vec3::new(900.0, 0.0, -600.0)));

    rts::become_pilot(app.world_mut(), e);

    assert!(is_piloted(&mut app, e), "the human took the wheel but the seat is empty");
    assert!(
        !has_order(&mut app, e),
        "the body kept its standing order while a human is driving it — two drivers"
    );
}

#[test]
#[ignore = "RED: needs rts::unbecome to vacate the seat"]
fn unbecoming_does_not_resurrect_the_cleared_order() {
    let mut app = app_with_rts();
    let start = Vec3::new(100.0, 0.0, 100.0);
    let e = spawn_unit(&mut app, start, Some(Vec3::new(900.0, 0.0, -600.0)));

    rts::become_pilot(app.world_mut(), e);
    rts::unbecome(app.world_mut());

    assert!(!is_piloted(&mut app, e), "stepping out left the human still seated");
    assert!(
        !has_order(&mut app, e),
        "the order the human overrode came back on unbecome — the body wanders off on its own"
    );

    // And stays put, because it is now neither ordered nor piloted.
    run(&mut app, TICKS);
    let moved = xz_dist(pos(&mut app, e), start);
    assert!(
        moved < 1e-3,
        "an abandoned body walked {moved:.3} units with nothing driving it"
    );
}

#[test]
#[ignore = "RED: needs rts::unbecome to report who left the wheel"]
fn unbecoming_reports_the_body_that_was_left() {
    // The detached camera parks where the body stood instead of cutting
    // to somewhere else, so unbecome has to say which body that was.
    let mut app = app_with_rts();
    let e = spawn_unit(&mut app, Vec3::new(250.0, 0.0, -80.0), None);

    rts::become_pilot(app.world_mut(), e);
    assert_eq!(
        rts::unbecome(app.world_mut()),
        Some(e),
        "unbecome did not report which body the human stepped out of"
    );

    assert_eq!(
        rts::unbecome(app.world_mut()),
        None,
        "unbecome reported a body when the seat was already empty"
    );
}

#[test]
#[ignore = "RED: needs rts::become_pilot to vacate the seat before taking it"]
fn only_one_body_holds_the_wheel() {
    // Becoming is also how you switch bodies, so the invariant needs no
    // separate enforcement — but it does need to actually hold.
    let mut app = app_with_rts();
    let first = spawn_unit(&mut app, Vec3::new(0.0, 0.0, 0.0), None);
    let second = spawn_unit(&mut app, Vec3::new(400.0, 0.0, 0.0), None);

    rts::become_pilot(app.world_mut(), first);
    rts::become_pilot(app.world_mut(), second);

    assert!(is_piloted(&mut app, second), "the human is not in the body they became");
    assert!(
        !is_piloted(&mut app, first),
        "the abandoned body is still seated — the human is in two places"
    );

    let world = app.world_mut();
    let mut q = world.query_filtered::<Entity, With<Piloted>>();
    let seated: Vec<Entity> = q.iter(world).collect();
    assert_eq!(seated.len(), 1, "expected exactly one pilot, found {}", seated.len());
}

#[test]
fn a_stray_order_cannot_steal_a_piloted_body() {
    // The malformed state, forced directly: both authorities on one
    // entity. `become_pilot` clears the order so this should be
    // unreachable — but "should be" is not a guarantee, and the obvious
    // way to reach it is a right-click that lands while you are
    // embodied. Steering must refuse it on the query, so the mistake
    // costs a stray component rather than control of your own body.
    let mut app = app_with_rts();
    let start = Vec3::new(-300.0, 0.0, 220.0);
    let e = spawn_unit(&mut app, start, Some(Vec3::new(900.0, 0.0, -600.0)));
    app.world_mut().entity_mut(e).insert(Piloted);

    run(&mut app, TICKS);

    let drift = xz_dist(pos(&mut app, e), start);
    assert!(
        drift < 1e-3,
        "a stray order dragged a piloted body {drift:.3} units — the human is fighting the steering"
    );
    let v = app.world().get::<Velocity>(e).expect("entity has a Velocity").0;
    assert!(
        v.length() < 1e-6,
        "steering wrote a velocity onto a piloted body: {v:?}"
    );
}
