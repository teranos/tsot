// The red for the RTS spike.
//
// One sentence of intent: *given a move order at world P issued to a
// squad of 8, after K ticks every unit is within R of P and no two
// overlap.*
//
// What it actually pins is the axiom change underneath an RTS — units
// move because they were COMMANDED, not because input is held this
// frame. `physics::keyboard_input` reads a bitmask every tick and
// forgets it; a `MoveOrder` is issued once and outlives the click.
// Everything else the genre needs (unleashed camera, pointer picking,
// pathfinding, order networking) is downstream of this holding.
//
// Headless by construction: no GPU, no render, no terrain. Assertions
// are XZ-only because steering is XZ-only — `physics::ground_follow_*`
// owns Y, and pulling it in here would make a terrain change able to
// fail a movement test.
//
// The three tests that need `rts` to actually do something carry
// `#[ignore = "RED: ..."]`. That is not the red being hidden — the CI
// gate refuses a bare `#[ignore]` and prints every declared skip with
// its reason into the run summary, so these are named on every run.
// It buys the one distinction a strict-TDD repo needs from its gate:
// red because not built yet, versus red because broken. Look at them
// with `make red`; delete the attribute as each system gains a body.

use bevy_app::App;
use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use game::physics::{Position, Velocity};
use game::rts::{self, ARRIVE_RADIUS, MoveOrder, UNIT_RADIUS, UnitMarker};

const SQUAD: usize = 8;

/// The commanded destination. Far enough that arriving is real travel
/// (~1130 units from the far corner of the spawn grid) rather than the
/// squad already being inside `ARRIVE_RADIUS` at spawn.
fn order_pos() -> Vec3 {
    Vec3::new(900.0, 0.0, -600.0)
}

/// Ticks to allow. At `UNIT_SPEED` the trip is ~95 ticks; 400 leaves
/// room for separation jostling without the test passing on a unit
/// that took a wildly wrong path and happened to end up nearby.
const TICKS: u32 = 400;

/// A 4×2 grid at 50-unit spacing centred on the origin. Minimum
/// pairwise distance is 50, above the `2 * UNIT_RADIUS` = 40 contact
/// distance — the squad starts legal, so any overlap at the end was
/// produced by the movement, not carried in from the spawn.
fn spawn_grid() -> Vec<Vec3> {
    (0..SQUAD)
        .map(|i| {
            let col = (i % 4) as f32;
            let row = (i / 4) as f32;
            Vec3::new(col * 50.0 - 75.0, 0.0, row * 50.0 - 25.0)
        })
        .collect()
}

/// Ground-plane distance. Y is not steering's business.
fn xz_dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

/// Build an app running the real `rts::register` schedule — the
/// ordering under test is the ordering that ships — and spawn units at
/// `spawns`, each carrying `order` if one is given.
fn squad_app(spawns: &[Vec3], order: Option<Vec3>) -> App {
    let mut app = App::new();
    rts::register(&mut app);
    let world = app.world_mut();
    for &p in spawns {
        let mut unit = world.spawn((UnitMarker, Position(p), Velocity(Vec3::ZERO)));
        if let Some(o) = order {
            unit.insert(MoveOrder(o));
        }
    }
    app
}

fn run(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.update();
    }
}

fn unit_positions(app: &mut App) -> Vec<Vec3> {
    let world = app.world_mut();
    let mut q = world.query_filtered::<&Position, With<UnitMarker>>();
    q.iter(world).map(|p| p.0).collect()
}

#[test]
#[ignore = "RED: needs rts::steer_toward_order + rts::advance_units"]
fn commanded_squad_arrives_at_its_order() {
    let mut app = squad_app(&spawn_grid(), Some(order_pos()));
    run(&mut app, TICKS);

    let units = unit_positions(&mut app);
    assert_eq!(units.len(), SQUAD, "squad lost or gained units in transit");
    for (i, p) in units.iter().enumerate() {
        let d = xz_dist(*p, order_pos());
        assert!(
            d <= ARRIVE_RADIUS,
            "unit {i} did not arrive: {d:.1} from the order, \
             tolerance {ARRIVE_RADIUS} (at {:.1}, {:.1})",
            p.x,
            p.z
        );
    }
}

#[test]
fn arrived_squad_does_not_stack() {
    let mut app = squad_app(&spawn_grid(), Some(order_pos()));
    run(&mut app, TICKS);

    let units = unit_positions(&mut app);
    let contact = 2.0 * UNIT_RADIUS;
    for i in 0..units.len() {
        for j in (i + 1)..units.len() {
            let d = xz_dist(units[i], units[j]);
            assert!(
                d >= contact - 0.5,
                "units {i} and {j} overlap: {d:.1} apart, \
                 contact distance {contact}"
            );
        }
    }
}

#[test]
#[ignore = "RED: needs rts::steer_toward_order to stop inside ARRIVE_RADIUS"]
fn arrived_squad_stays_put() {
    let mut app = squad_app(&spawn_grid(), Some(order_pos()));
    run(&mut app, TICKS);
    // Long past arrival: a squad that orbits or oscillates around the
    // target passes the arrival test on a lucky tick and fails here.
    run(&mut app, 200);

    for (i, p) in unit_positions(&mut app).iter().enumerate() {
        let d = xz_dist(*p, order_pos());
        assert!(
            d <= ARRIVE_RADIUS,
            "unit {i} drifted off the order after arriving: {d:.1}"
        );
    }
}

#[test]
fn uncommanded_unit_holds_position() {
    // The command axiom stated negatively. No `MoveOrder` is not "the
    // key isn't held right now" — it is "was never ordered", and the
    // unit must not move at all, ever.
    let spawns = vec![Vec3::new(300.0, 0.0, 300.0)];
    let mut app = squad_app(&spawns, None);
    run(&mut app, TICKS);

    let units = unit_positions(&mut app);
    assert_eq!(units.len(), 1);
    let moved = xz_dist(units[0], spawns[0]);
    assert!(
        moved < 1e-3,
        "an unordered unit moved {moved:.3} units — something other \
         than a command is driving it"
    );
}

#[test]
#[ignore = "RED: needs rts::separate_units with a degenerate-normal floor"]
fn coincident_units_separate_without_nan() {
    // Two units on the exact same tile: the degenerate case for any
    // push-apart, where the contact normal is undefined and a naive
    // `delta / delta.length()` yields NaN. `physics::resolve_collisions`
    // already guards this with a `dist_sq > 1.0e-6` floor; separation
    // has to guard it too, and a NaN position is unrecoverable — it
    // propagates into the camera and blanks the frame.
    let here = Vec3::new(120.0, 0.0, -40.0);
    let mut app = squad_app(&[here, here], None);
    run(&mut app, TICKS);

    let units = unit_positions(&mut app);
    for (i, p) in units.iter().enumerate() {
        assert!(
            p.x.is_finite() && p.z.is_finite(),
            "unit {i} position went non-finite: ({}, {})",
            p.x,
            p.z
        );
    }
    let d = xz_dist(units[0], units[1]);
    assert!(
        d >= 2.0 * UNIT_RADIUS - 0.5,
        "coincident units never separated: {d:.1} apart"
    );
}
