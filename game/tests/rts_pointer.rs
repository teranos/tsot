// The pointer maths, and the claim the whole RTS argument rests on.
//
// I said an RTS on this codebase is affordable because the camera is
// orthographic: no perspective divide, so turning a cursor position
// into a world position is analytic rather than a solve. Every mouse
// interaction in the spec — drag a rectangle to select, right-click to
// send them there — is that one operation wearing two hats. If it does
// not hold, the mouse story costs more than I said it did, and better
// to find that out here than after spending a freeze exception on the
// browser shim.
//
// So this file is deliberately upstream of any input plumbing. No
// env.*, no JS, no buttons, no wheel events — just camera, terrain and
// arithmetic. What a real pointer eventually delivers is two floats in
// NDC and an integer of wheel notches, and both are supplied here by
// hand. The browser cannot be the reason this is wrong.
//
// Zoom appears throughout rather than in its own corner on purpose.
// Zoom is what makes the maths non-trivial: selection and unprojection
// both have to agree with whatever the camera currently covers, and
// "works at the default zoom, wrong when zoomed" is the exact bug that
// ships when they are tested apart.

use bevy_ecs::prelude::*;
use bevy_math::Vec3;

use game::room;
use game::rts::{self, CameraZoom};
use game::scene::{SceneCamera, ZOOM_MAX, ZOOM_MIN};
use game::terrain;

/// A spread of zooms across the legal range, so nothing here can pass
/// by being true only at the resting magnification.
const ZOOMS: [f32; 5] = [ZOOM_MIN, 700.0, 1450.0, 2600.0, ZOOM_MAX];

fn camera_at(focus: Vec3, half_extent: f32) -> SceneCamera {
    SceneCamera::at([focus.x, focus.y, focus.z], half_extent, room::FLOOR_HALF)
}

/// A focus that sits on the terrain, as the real camera always will.
fn focus() -> Vec3 {
    let (x, z) = (1200.0, -800.0);
    Vec3::new(x, terrain::height(x, z), z)
}

/// A world point known to be on the terrain surface.
fn on_terrain(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, terrain::height(x, z), z)
}

fn xz_dist(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

#[test]
#[ignore = "RED: needs CameraZoom::nudge"]
fn the_wheel_zooms_and_saturates_at_both_ends() {
    // Wheel-up is zoom IN, which is a tighter frustum, so half_extent
    // shrinks. Getting this backwards is the classic inverted-zoom bug
    // and it is invisible in a screenshot.
    let mut z = CameraZoom::new(1450.0);
    z.nudge(1);
    assert!(
        z.half_extent() < 1450.0,
        "wheel-up did not zoom in: {} is not tighter than 1450",
        z.half_extent()
    );

    // Scrolling past the end of the range is an ordinary thing a hand
    // does. It saturates; it does not wrap, overshoot or fault.
    let mut deep = CameraZoom::new(1450.0);
    deep.nudge(10_000);
    assert_eq!(
        deep.half_extent(),
        ZOOM_MIN,
        "zooming in without limit did not stop at ZOOM_MIN"
    );

    let mut far = CameraZoom::new(1450.0);
    far.nudge(-10_000);
    assert_eq!(
        far.half_extent(),
        ZOOM_MAX,
        "zooming out without limit did not stop at ZOOM_MAX"
    );
}

#[test]
fn zooming_in_covers_less_world() {
    // The guard under the whole file: if the camera's coverage did not
    // actually track half_extent, every selection and unprojection
    // result below would be meaningless even when it looked right.
    let f = focus();
    let tight = camera_at(f, 600.0);
    let wide = camera_at(f, 3000.0);

    let probe = on_terrain(f.x + 900.0, f.z);
    let tight_ndc = tight.world_to_clip([probe.x, probe.y, probe.z]);
    let wide_ndc = wide.world_to_clip([probe.x, probe.y, probe.z]);

    assert!(
        tight_ndc[0].abs() > wide_ndc[0].abs(),
        "the same world point should sit further from screen centre when zoomed in: \
         tight {:.3} vs wide {:.3}",
        tight_ndc[0],
        wide_ndc[0]
    );
}

#[test]
#[ignore = "RED: needs rts::ground_under"]
fn the_point_under_the_cursor_round_trips_at_every_zoom() {
    // The load-bearing property, stated as a round trip: project a
    // point that is known to be on the terrain, hand the resulting NDC
    // back as if it were a cursor, and get the same ground position.
    //
    // Asserted on XZ. Y is the heightfield's answer for that XZ, so
    // testing it as a third coordinate would only restate terrain, and
    // a Y mismatch with matching XZ is a terrain bug, not a pointer one.
    let f = focus();
    for half_extent in ZOOMS {
        let cam = camera_at(f, half_extent);
        for (dx, dz) in [(0.0, 0.0), (300.0, -200.0), (-450.0, 380.0), (120.0, 640.0)] {
            let want = on_terrain(f.x + dx, f.z + dz);
            let ndc = cam.world_to_clip([want.x, want.y, want.z]);
            let got = rts::ground_under(&cam, ndc).unwrap_or_else(|| {
                panic!(
                    "no ground under a cursor aimed at a point that IS on the ground \
                     (zoom {half_extent}, offset {dx},{dz})"
                )
            });
            let err = xz_dist(got, want);
            assert!(
                err < 1.0,
                "round trip drifted {err:.2} units at zoom {half_extent}: \
                 wanted ({:.1}, {:.1}), got ({:.1}, {:.1})",
                want.x,
                want.z,
                got.x,
                got.z
            );
        }
    }
}

#[test]
fn a_cursor_on_the_sky_names_no_destination() {
    // Near the horizon a cursor genuinely points at nothing. The honest
    // answer is None. Returning a fabricated coordinate would send a
    // squad marching toward a place the user never clicked, and it
    // would look like a pathfinding bug for as long as it took to find.
    //
    // A guard, not a red: the stub returns None for everything, so this
    // passes vacuously today and only bites once `ground_under` starts
    // returning positions. Declaring it RED would be a lie in the
    // opposite direction from the one the gate catches.
    let cam = camera_at(focus(), 1450.0);
    assert_eq!(
        rts::ground_under(&cam, [0.0, 50.0]),
        None,
        "a cursor far off the top of the frustum was given a ground position anyway"
    );
}

#[test]
#[ignore = "RED: needs rts::units_in_rect"]
fn a_drag_selects_exactly_what_it_covers() {
    let f = focus();
    let cam = camera_at(f, 1450.0);

    let inside_a = on_terrain(f.x - 100.0, f.z - 100.0);
    let inside_b = on_terrain(f.x + 100.0, f.z + 100.0);
    let outside = on_terrain(f.x + 2000.0, f.z + 2000.0);

    let mut world = World::new();
    let ea = world.spawn_empty().id();
    let eb = world.spawn_empty().id();
    let eo = world.spawn_empty().id();
    let units = [(ea, inside_a), (eb, inside_b), (eo, outside)];

    // A rect built around screen centre, where the focus projects.
    let picked = rts::units_in_rect(&cam, &units, [-0.35, -0.35], [0.35, 0.35]);

    assert!(picked.contains(&ea) && picked.contains(&eb), "the drag missed units it covered");
    assert!(!picked.contains(&eo), "the drag caught a unit far outside it");
    assert_eq!(picked.len(), 2, "expected exactly the two covered units");
}

#[test]
#[ignore = "RED: needs rts::units_in_rect to normalise its corners"]
fn a_drag_selects_the_same_set_dragged_backwards() {
    // Users drag up-left as often as down-right. The rect is defined by
    // two corners, not by an origin and a direction.
    let f = focus();
    let cam = camera_at(f, 1450.0);

    let mut world = World::new();
    let e = world.spawn_empty().id();
    let units = [(e, on_terrain(f.x, f.z))];

    let forward = rts::units_in_rect(&cam, &units, [-0.3, -0.3], [0.3, 0.3]);
    let backward = rts::units_in_rect(&cam, &units, [0.3, 0.3], [-0.3, -0.3]);

    assert_eq!(forward, vec![e], "the forward drag missed the unit under it");
    assert_eq!(backward, forward, "dragging the other way selected a different set");
}

#[test]
#[ignore = "RED: needs rts::units_in_rect to respect the camera's zoom"]
fn the_same_drag_catches_more_when_zoomed_out() {
    // The zoom-coupling bug, pinned. A screen rectangle is a claim
    // about pixels; what it covers in WORLD terms depends entirely on
    // how much world is on screen. A selection that ignores zoom passes
    // every test written at one magnification and is wrong at all the
    // others.
    let f = focus();
    let mut world = World::new();

    // A ring of units spread far enough that a tight camera pushes most
    // of them off-screen and a wide one brings them back in.
    let units: Vec<(Entity, Vec3)> = [
        (0.0, 0.0),
        (600.0, 0.0),
        (-600.0, 0.0),
        (0.0, 600.0),
        (0.0, -600.0),
    ]
    .into_iter()
    .map(|(dx, dz)| (world.spawn_empty().id(), on_terrain(f.x + dx, f.z + dz)))
    .collect();

    let rect = ([-0.5, -0.5], [0.5, 0.5]);
    let tight = rts::units_in_rect(&camera_at(f, ZOOM_MIN), &units, rect.0, rect.1);
    let wide = rts::units_in_rect(&camera_at(f, ZOOM_MAX), &units, rect.0, rect.1);

    assert!(
        wide.len() > tight.len(),
        "the same screen rect selected {} zoomed in and {} zoomed out — \
         selection is ignoring the camera",
        tight.len(),
        wide.len()
    );
}
