// RTS command layer. The axiom this module introduces:
//
//   A unit moves because it was COMMANDED, not because input is held
//   this frame.
//
// `physics::keyboard_input` reads a bitmask every tick and forgets it —
// release W and the player stops. A `MoveOrder` is a component: issued
// once, it outlives the click that produced it and keeps steering the
// unit until the unit arrives. That difference IS the genre change.
// Everything else an RTS needs — free camera, pointer picking,
// pathfinding, order networking — is downstream of it, and none of it
// is worth building until this holds.
//
// Steering is XZ-only, matching `physics::resolve_collisions`: the
// ground plane decides movement, `physics::ground_follow_*` owns Y.
//
// The second authority arrived with the observer: a *piloted* entity
// is the one a human has become, driven by held input rather than by a
// destination. The two are exclusive by construction — see `Piloted`.
//
// STATUS: the movement kernel, the authority state machine and the
// pointer maths are implemented and green
// (`tests/rts_move_order.rs`, `rts_authority.rs`, `rts_pointer.rs`).
// Not yet here: pathfinding — units steer straight at their order and
// walk through walls — and any input plumbing. Nothing in this module
// reads a key, a button or a wheel; callers supply NDC and notches.

use bevy_app::{App, Update};
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::IntoScheduleConfigs;
use bevy_math::Vec3;

use crate::physics::{Position, Velocity};
use crate::scene::{SceneCamera, ZOOM_MAX, ZOOM_MIN};

/// Unit half-width on the ground plane. Same quantum as
/// `physics::PLAYER_RADIUS` — a unit is a player-scale actor, and
/// separation pushes out to the same 2×R contact distance the player's
/// sphere-vs-sphere collision already uses.
pub const UNIT_RADIUS: f32 = 20.0;

/// Commanded movement speed, world units per tick. Between the NPC
/// wander (3.0) and the player's keyboard sprint (18.0): a squad
/// crosses a chunk in seconds without outrunning the follow of a
/// camera that isn't leashed to it.
pub const UNIT_SPEED: f32 = 12.0;

/// How close to its order a unit counts as arrived.
///
/// This is a SQUAD tolerance, not a point tolerance: eight discs of
/// radius `UNIT_RADIUS` cannot all occupy the order position, so the
/// squad settles into a blob around it. Eight unit circles pack into an
/// enclosing circle of roughly 3.3·R ≈ 66; doubling that leaves room
/// for the jostle separation adds on the final ticks without letting a
/// unit that simply stopped early pass for arrived.
pub const ARRIVE_RADIUS: f32 = 140.0;

/// How close a single unit presses before it stops pushing inward.
///
/// This has to exceed the squad's own packing radius or the design
/// fights itself: eight discs of `UNIT_RADIUS` need an enclosing circle
/// of about 3.3·R ≈ 66, so if a unit kept steering until it was nearer
/// than that, the outer ranks would press inward every tick against a
/// separation pass shoving them back out, forever. Nothing would look
/// broken — the squad would just quiver on the spot and never quite
/// stop overlapping.
///
/// 90 clears 66 with room to spare, and sits far enough under
/// `ARRIVE_RADIUS` that a settled squad is unambiguously arrived rather
/// than borderline.
const STOP_RADIUS: f32 = 90.0;

/// Below this squared distance two units are treated as coincident and
/// the contact normal is undefined. Same floor
/// `physics::resolve_collisions` uses, for the same reason: dividing by
/// that length yields NaN, and a NaN position is unrecoverable — it
/// propagates into the camera and blanks the frame.
const COINCIDENT_EPS_SQ: f32 = 1.0e-6;

/// A commandable actor. Distinct from `PlayerMarker` (one embodied
/// avatar driven by held input) and `NpcMarker` (autonomous wander) —
/// a unit does nothing until told, and then does it without being told
/// again.
#[derive(Component)]
pub struct UnitMarker;

/// A standing order to move to a world position. Y is carried for
/// symmetry with `Position` but ignored by steering.
///
/// Presence of this component is the command. Absence is not "stop
/// pressing the key", it is "was never ordered" — an entity that is
/// neither ordered nor piloted holds position forever.
#[derive(Component, Clone, Copy, Debug)]
pub struct MoveOrder(pub Vec3);

/// The pilot seat. At most one entity in the world carries this: the
/// one the human has *become*.
///
/// This is the second of the two input authorities, and they are
/// exclusive by construction. An ordered entity is driven by
/// `steer_toward_order` from a destination it was given. A piloted
/// entity is driven by held input, here and now, the way
/// `physics::keyboard_input` drives `PlayerMarker`. Nothing may be
/// both — an entity still being dragged toward a stale destination
/// while a human holds the wheel is the exact failure this component
/// exists to make impossible.
#[derive(Component)]
pub struct Piloted;

/// A unit currently open to being commanded: one nobody has become.
/// The query filter that keeps the two authorities from overlapping,
/// named once so every system that must respect the invariant spells
/// it the same way.
pub type Commandable = (With<UnitMarker>, Without<Piloted>);

/// A unit that may receive an order right now: selected, and not the
/// body a human is currently driving. `Without<Piloted>` here is the
/// invariant defended at its source — `steer_toward_order` filtering
/// piloted entities catches the malformed state, but this stops the
/// ordinary path from ever creating one.
pub type Orderable = (With<Selected>, With<UnitMarker>, Without<Piloted>);

/// Take the wheel: seat the human in `entity`.
///
/// Vacates the seat first, so becoming is also how you switch bodies
/// and the one-pilot invariant needs no separate enforcement.
///
/// Clears any standing `MoveOrder`. The human took over; wherever the
/// entity had been told to walk is stale intent, and resurrecting it
/// on `unbecome` would read as the body wandering off on its own.
pub fn become_pilot(world: &mut World, entity: Entity) {
    unbecome(world);
    let Ok(mut e) = world.get_entity_mut(entity) else {
        crate::error::emit_region(
            crate::error::Severity::Error,
            "rts.become_pilot",
            "cannot become an entity that does not exist",
            format!(
                "{entity:?} is not in the world — the seat is left empty rather than \
                 silently seating you in nothing."
            ),
        );
        return;
    };
    e.insert(Piloted);
    e.remove::<MoveOrder>();
}

/// Leave the wheel. Returns the entity that was seated, so the caller
/// can park a detached camera exactly where the body stood instead of
/// cutting to somewhere else; `None` when the seat was already empty.
pub fn unbecome(world: &mut World) -> Option<Entity> {
    let mut q = world.query_filtered::<Entity, With<Piloted>>();
    let seated: Vec<Entity> = q.iter(world).collect();

    // More than one seat filled means the invariant was broken by some
    // path that inserted `Piloted` directly instead of going through
    // `become_pilot`. Clear them all — but say so. Quietly tidying a
    // broken invariant is how it stays broken.
    if seated.len() > 1 {
        crate::error::emit_region(
            crate::error::Severity::Error,
            "rts.unbecome",
            "more than one entity held the wheel",
            format!(
                "{} entities carried `Piloted`: {seated:?}. All were vacated. \
                 Something inserted `Piloted` without going through `become_pilot`.",
                seated.len()
            ),
        );
    }

    for &e in &seated {
        if let Ok(mut em) = world.get_entity_mut(e) {
            em.remove::<Piloted>();
        }
    }
    seated.first().copied()
}

/// Order → velocity. Points each ordered unit at its `MoveOrder` on the
/// XZ plane at `UNIT_SPEED`, and stops it inside `ARRIVE_RADIUS` so
/// arrived units don't jitter around the target forever.
///
/// `Without<Piloted>` is load-bearing, not defensive tidiness.
/// `become_pilot` clears the order, so the both-at-once state should
/// be unreachable — but "should be" is not a guarantee, and the
/// obvious way to reach it is a right-click that lands while you are
/// embodied. This filter means that mistake costs a stray component,
/// not control of your own body.
pub fn steer_toward_order(
    mut q: Query<(&Position, &MoveOrder, &mut Velocity), Commandable>,
) {
    for (p, order, mut v) in q.iter_mut() {
        let (dx, dz) = (order.0.x - p.0.x, order.0.z - p.0.z);
        let dist_sq = dx * dx + dz * dz;
        if dist_sq <= STOP_RADIUS * STOP_RADIUS {
            v.0 = Vec3::ZERO;
            continue;
        }
        let dist = dist_sq.sqrt();
        v.0 = Vec3::new(dx / dist * UNIT_SPEED, 0.0, dz / dist * UNIT_SPEED);
    }
}

/// Velocity → position, for units. Deliberately separate from
/// `physics::advance_player` / `advance_npc`: those integrate one
/// avatar and one wanderer, this integrates the commanded set.
/// Deliberately `With<UnitMarker>` and not `Commandable`: a piloted
/// body's velocity must still be integrated, because that is exactly
/// how held input moves it. Steering is what has to leave piloted
/// entities alone, not integration.
pub fn advance_units(mut q: Query<(&mut Position, &Velocity), With<UnitMarker>>) {
    for (mut p, v) in q.iter_mut() {
        p.0 += v.0;
    }
}

/// Pairwise separation: push overlapping units apart on XZ until every
/// pair is at least `2 * UNIT_RADIUS` apart. Without this a squad
/// sharing one order collapses into a single stacked column at the
/// target — visually one unit, and the reason an order tolerance can
/// never be a point tolerance.
pub fn separate_units(mut q: Query<&mut Position, With<UnitMarker>>) {
    let contact = 2.0 * UNIT_RADIUS;
    let mut pairs = q.iter_combinations_mut();
    while let Some([mut a, mut b]) = pairs.fetch_next() {
        let (dx, dz) = (a.0.x - b.0.x, a.0.z - b.0.z);
        let dist_sq = dx * dx + dz * dz;
        if dist_sq >= contact * contact {
            continue;
        }
        // Coincident: the normal is undefined, so pick a fixed axis
        // rather than divide by ~zero. Deterministic on purpose — every
        // peer resolving the same stack the same way matters more than
        // the direction being interesting.
        let (nx, nz, dist) = if dist_sq > COINCIDENT_EPS_SQ {
            let d = dist_sq.sqrt();
            (dx / d, dz / d, d)
        } else {
            (1.0, 0.0, 0.0)
        };
        // Half the overlap each: neither unit outranks the other, and
        // splitting it keeps the pair's centre of mass where it was.
        let push = (contact - dist) * 0.5;
        a.0.x += nx * push;
        a.0.z += nz * push;
        b.0.x -= nx * push;
        b.0.z -= nz * push;
    }
}

/// How much world the detached observer covers, in `half_extent`
/// units. Clamped to `[ZOOM_MIN, ZOOM_MAX]` on construction and on
/// every nudge, so no caller can put the camera somewhere the world
/// cannot be drawn from.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CameraZoom(f32);

impl CameraZoom {
    pub fn new(half_extent: f32) -> Self {
        Self(half_extent.clamp(ZOOM_MIN, ZOOM_MAX))
    }

    pub fn half_extent(self) -> f32 {
        self.0
    }

    /// Apply accumulated wheel notches. Positive is wheel-up, which
    /// zooms IN — a tighter frustum, so the half-extent shrinks.
    /// Saturates at the bounds rather than wrapping or erroring:
    /// scrolling past the end of the range is an ordinary thing a
    /// hand does, not a fault.
    pub fn nudge(&mut self, wheel_notches: i32) {
        // Multiplicative, so a notch feels the same at every
        // magnification — a fixed additive step would crawl when zoomed
        // out and lurch when zoomed in.
        //
        // The extremes take care of themselves: a huge notch count
        // sends the factor to 0 or infinity, and the clamp turns both
        // into the nearer bound. No special-casing, no overflow branch.
        const PER_NOTCH: f32 = 1.1;
        self.0 = (self.0 * PER_NOTCH.powi(-wheel_notches)).clamp(ZOOM_MIN, ZOOM_MAX);
    }
}

/// The world point on the terrain directly under a pointer at `ndc`.
///
/// This is the inverse of `SceneCamera::world_to_clip`, and the reason
/// an RTS on this codebase is affordable: the camera is orthographic,
/// so there is no perspective divide to undo and the unprojection is
/// analytic. The only iteration is finding where the resulting ray
/// meets the heightfield, which is a march plus a bisection, not a
/// solve.
///
/// `None` when the pointer is off the canvas (NDC outside `[-1, 1]`,
/// which is what `input::pointer_ndc` reports for a cursor that has
/// left the surface), or when the ray never meets the terrain inside
/// the far plane. Both are answered with `None` rather than a
/// fabricated coordinate: inventing one would send a squad marching to
/// a place nobody clicked, and it would read as a pathfinding bug for
/// as long as it took to find.
pub fn ground_under(camera: &SceneCamera, ndc: [f32; 2]) -> Option<Vec3> {
    if ndc[0].abs() > 1.0 || ndc[1].abs() > 1.0 {
        return None;
    }
    let inv = bevy_math::Mat4::from_cols_array_2d(&camera.view_proj()).inverse();
    // wgpu clip depth runs [0, 1]: z=0 is the near plane, z=1 the far.
    let near = inv.project_point3(Vec3::new(ndc[0], ndc[1], 0.0));
    let far = inv.project_point3(Vec3::new(ndc[0], ndc[1], 1.0));
    let span = far - near;
    let len = span.length();
    if !len.is_finite() || len < 1.0 {
        return None;
    }
    let dir = span / len;

    // Signed vertical clearance: positive above the ground, negative
    // below. The surface is where it changes sign.
    let clearance = |t: f32| {
        let p = near + dir * t;
        p.y - crate::terrain::height(p.x, p.z)
    };
    let land = |t: f32| {
        let p = near + dir * t;
        Vec3::new(p.x, crate::terrain::height(p.x, p.z), p.z)
    };

    if clearance(0.0) <= 0.0 {
        // The near plane is already underground — the camera is closer
        // to the hill than its own clip plane. The first visible ground
        // is right there.
        return Some(land(0.0));
    }

    // Fixed-step search for a sign change, then bisect it. A fixed step
    // is sound here because `terrain::height` is documented continuous
    // and gentle — "no cliffs, no mountains" — so the surface cannot
    // rise through a step and back out of it between samples.
    //
    // No performance claim attached to the step size: if this shows up
    // in seer's `[perf]`, the fix is to bound the search to the field's
    // actual height range instead of walking the whole frustum.
    const STEP: f32 = 50.0;
    const BISECTIONS: u32 = 40;
    let mut prev = 0.0f32;
    let mut t = STEP;
    while t <= len {
        if clearance(t) <= 0.0 {
            let (mut lo, mut hi) = (prev, t);
            for _ in 0..BISECTIONS {
                let mid = 0.5 * (lo + hi);
                if clearance(mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            return Some(land(hi));
        }
        prev = t;
        t += STEP;
    }
    None
}

/// Every unit whose position projects inside the NDC rectangle spanned
/// by two pointer corners — the left-drag selection.
///
/// Goes the cheap direction on purpose: project each unit to clip
/// space with the camera and test it against the rect, rather than
/// unprojecting the rect into a world frustum. Same answer, no
/// inversion, and it stays correct at any zoom for free because the
/// projection is the thing that knows about zoom.
///
/// Corners may be given in any order; the rect is normalised.
pub fn units_in_rect(
    camera: &SceneCamera,
    units: &[(Entity, Vec3)],
    corner_a: [f32; 2],
    corner_b: [f32; 2],
) -> Vec<Entity> {
    let (min_x, max_x) = (corner_a[0].min(corner_b[0]), corner_a[0].max(corner_b[0]));
    let (min_y, max_y) = (corner_a[1].min(corner_b[1]), corner_a[1].max(corner_b[1]));
    units
        .iter()
        .filter(|(_, p)| {
            let c = camera.world_to_clip([p.x, p.y, p.z]);
            c[0] >= min_x && c[0] <= max_x && c[1] >= min_y && c[1] <= max_y
        })
        .map(|(e, _)| *e)
        .collect()
}

/// One frame of human input, as data.
///
/// Every interaction system reads this and nothing else — no system
/// below calls an `env.*` import, reads a static, or knows a browser
/// exists. `pump_input_frame` is the single place the outside world
/// gets in, which is what makes a drag, an order and a zoom testable
/// by writing a struct.
///
/// `buttons` is a LEVEL, not an event: it says what is held right now,
/// mirroring the DOM's `buttons` property. Press and release edges are
/// derived by comparing against the previous frame, because that is the
/// one thing a level cannot tell you and every interaction needs.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct InputFrame {
    /// Keyboard bitmask, `input::key::*`.
    pub keys: u32,
    /// Pointer in NDC, or `None` when it is not over the canvas.
    pub ndc: Option<[f32; 2]>,
    /// Buttons held this frame, `button::*`.
    pub buttons: u32,
    /// Wheel notches accumulated since the last frame. Positive is
    /// wheel-up, which zooms in.
    pub wheel_y: i32,
}

pub mod button {
    /// Matches the DOM `MouseEvent.buttons` bit layout, so the shim
    /// hands the value across unchanged rather than re-encoding it.
    pub const LEFT: u32 = 1;
    pub const RIGHT: u32 = 2;
}

/// A unit the human has selected. Orders land on exactly this set.
#[derive(Component)]
pub struct Selected;

/// What the observer is looking at: a point it can pan freely, or a
/// body it is following because the human became that body.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub enum CameraFocus {
    Free(Vec3),
    Following(Entity),
}

/// Observer pan speed, world units per tick per key. Faster than
/// `KEYBOARD_SPEED` because panning a view has no body to outrun and
/// crossing the map is the common case.
pub const PAN_SPEED: f32 = 26.0;

/// Buttons held on the previous frame, so press and release edges can
/// be derived from a level, plus the NDC the current drag started at.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct DragState {
    pub prev_buttons: u32,
    pub anchor: Option<[f32; 2]>,
}

/// Read the outside world into `InputFrame`. THE env boundary for
/// interaction — on native it leaves the resource alone, so tests write
/// the struct directly and drive the exact frames they mean.
pub fn pump_input_frame(mut _frame: ResMut<InputFrame>) {
    #[cfg(target_arch = "wasm32")]
    {
        *_frame = InputFrame {
            keys: crate::input::state(),
            ndc: crate::input::pointer_ndc(),
            buttons: crate::input::buttons(),
            wheel_y: crate::input::wheel_delta_y(),
        };
    }
}

/// Left press anchors a drag; left release resolves the rectangle it
/// swept into the selection, replacing whatever was selected before.
pub fn apply_drag_selection(
    frame: Res<InputFrame>,
    mut drag: ResMut<DragState>,
    focus: Res<CameraFocus>,
    zoom: Res<CameraZoom>,
    units: Query<(Entity, &Position), With<UnitMarker>>,
    mut commands: Commands,
) {
    let held_before = drag.prev_buttons & button::LEFT != 0;
    let held_now = frame.buttons & button::LEFT != 0;

    if !held_before && held_now {
        // Press: remember where the rectangle starts. `None` when the
        // press happened off-canvas, which then resolves to no drag.
        drag.anchor = frame.ndc;
        return;
    }
    if !(held_before && !held_now) {
        return;
    }

    // Release: resolve the swept rectangle. Anything that stops us
    // computing it leaves the old selection alone rather than silently
    // clearing — a hand that let go outside the canvas did not ask for
    // its selection to be thrown away.
    let anchor = drag.anchor.take();
    let (Some(a), Some(b)) = (anchor, frame.ndc) else {
        return;
    };
    let Some(camera) = observer_camera(&focus, *zoom, &units) else {
        return;
    };
    let all: Vec<(Entity, Vec3)> = units.iter().map(|(e, p)| (e, p.0)).collect();
    let picked = units_in_rect(&camera, &all, a, b);
    for (e, _) in &all {
        commands.entity(*e).remove::<Selected>();
    }
    for e in picked {
        commands.entity(e).insert(Selected);
    }
}

/// Right press sends the current selection to the ground under the
/// cursor.
pub fn apply_right_click_orders(
    frame: Res<InputFrame>,
    drag: Res<DragState>,
    focus: Res<CameraFocus>,
    zoom: Res<CameraZoom>,
    selected: Query<Entity, Orderable>,
    units: Query<(Entity, &Position), With<UnitMarker>>,
    mut commands: Commands,
) {
    // Press edge only. Acting on the level would re-issue the same
    // order every frame the button stayed down.
    let held_before = drag.prev_buttons & button::RIGHT != 0;
    let held_now = frame.buttons & button::RIGHT != 0;
    if held_before || !held_now {
        return;
    }
    let Some(ndc) = frame.ndc else {
        return;
    };
    let Some(camera) = observer_camera(&focus, *zoom, &units) else {
        return;
    };
    let Some(target) = ground_under(&camera, ndc) else {
        return;
    };
    for e in selected.iter() {
        commands.entity(e).insert(MoveOrder(target));
    }
}

/// Latch this frame's buttons for the next frame's edge detection.
/// Runs after every consumer of the edges, so both the drag and the
/// order see the same previous frame — one owner for the latch, rather
/// than whichever system happened to run first quietly consuming it.
pub fn latch_buttons(frame: Res<InputFrame>, mut drag: ResMut<DragState>) {
    drag.prev_buttons = frame.buttons;
}

/// WASD pans a detached observer. Does nothing while the observer is
/// following a body — those keys belong to the body then.
pub fn pan_observer(frame: Res<InputFrame>, mut focus: ResMut<CameraFocus>) {
    let CameraFocus::Free(point) = *focus else {
        return;
    };
    // Same 45° rotation `physics::keyboard_input` applies, for the same
    // reason: the camera is isometric, so W has to mean "up the screen"
    // rather than "along world -Z", or every key sends you diagonally.
    let mut dir = Vec3::ZERO;
    if frame.keys & crate::input::key::W != 0 {
        dir += Vec3::new(-1.0, 0.0, -1.0);
    }
    if frame.keys & crate::input::key::S != 0 {
        dir += Vec3::new(1.0, 0.0, 1.0);
    }
    if frame.keys & crate::input::key::A != 0 {
        dir += Vec3::new(-1.0, 0.0, 1.0);
    }
    if frame.keys & crate::input::key::D != 0 {
        dir += Vec3::new(1.0, 0.0, -1.0);
    }
    if dir.length_squared() <= 0.0 {
        return;
    }
    let step = dir.normalize() * PAN_SPEED;
    let (x, z) = (point.x + step.x, point.z + step.z);
    // The focus rides the terrain, as `follow` has always done with the
    // player's height — so panning over a hill lifts the view with it
    // instead of burying it.
    *focus = CameraFocus::Free(Vec3::new(x, crate::terrain::height(x, z), z));
}

/// The wheel zooms the observer.
pub fn zoom_observer(frame: Res<InputFrame>, mut zoom: ResMut<CameraZoom>) {
    // Guarded so an idle wheel doesn't mark the resource changed every
    // frame — change detection is a signal, and a signal that fires
    // constantly carries nothing.
    if frame.wheel_y != 0 {
        zoom.nudge(frame.wheel_y);
    }
}

/// The observer's camera for this frame, resolving `CameraFocus` to a
/// world point. `None` when it is following a body that no longer
/// exists — a corpse is not a viewpoint, and inventing one would put
/// the human somewhere nothing chose.
pub fn observer_camera(
    focus: &CameraFocus,
    zoom: CameraZoom,
    positions: &Query<(Entity, &Position), With<UnitMarker>>,
) -> Option<SceneCamera> {
    let point = match *focus {
        CameraFocus::Free(p) => p,
        CameraFocus::Following(e) => positions.get(e).ok()?.1.0,
    };
    Some(SceneCamera::at(
        [point.x, point.y, point.z],
        zoom.half_extent(),
        crate::room::FLOOR_HALF,
    ))
}

/// Register the command layer's systems in `Update`, in the order the
/// data flows: input → selection → orders → steering → velocity →
/// position → de-overlap. The tests drive this same registration, so
/// the ordering under test is the ordering that ships.
pub fn register(app: &mut App) {
    app.insert_resource(InputFrame::default());
    app.insert_resource(DragState::default());
    app.insert_resource(CameraFocus::Free(Vec3::ZERO));
    app.insert_resource(CameraZoom::new(1450.0));
    app.add_systems(
        Update,
        (
            pump_input_frame,
            pan_observer.after(pump_input_frame),
            zoom_observer.after(pump_input_frame),
            apply_drag_selection.after(pan_observer).after(zoom_observer),
            apply_right_click_orders.after(apply_drag_selection),
            latch_buttons.after(apply_right_click_orders),
            steer_toward_order.after(latch_buttons),
            advance_units.after(steer_toward_order),
            separate_units.after(advance_units),
        ),
    );
}

// `refuse` lived here again for the length of the interaction slice —
// the scaffolding that announces a stubbed function on the sacred error
// bus, so half-built input says so instead of looking like a dead
// mouse. Every function it covered has a body now, so it left again.
// It comes back with the next stub; clippy's dead-code gate is what
// makes sure it never lingers past its last caller.
