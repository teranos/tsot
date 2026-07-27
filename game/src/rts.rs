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
// STATUS: vocabulary only. The three systems below are declared and
// registered but have no bodies yet; each surfaces once on the sacred
// error bus so a run whose units refuse to move says why, at the
// point of interaction, instead of looking like a physics bug.
// `tests/rts_move_order.rs` is the red that defines done.

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

/// Take the wheel: seat the human in `entity`.
///
/// Vacates the seat first, so becoming is also how you switch bodies
/// and the one-pilot invariant needs no separate enforcement.
///
/// Clears any standing `MoveOrder`. The human took over; wherever the
/// entity had been told to walk is stale intent, and resurrecting it
/// on `unbecome` would read as the body wandering off on its own.
pub fn become_pilot(_world: &mut World, _entity: Entity) {
    refuse(
        "become_pilot",
        "the seat stays empty and the entity keeps its old order, so becoming a body does nothing.",
        "tests/rts_authority.rs",
    );
}

/// Leave the wheel. Returns the entity that was seated, so the caller
/// can park a detached camera exactly where the body stood instead of
/// cutting to somewhere else; `None` when the seat was already empty.
pub fn unbecome(_world: &mut World) -> Option<Entity> {
    refuse(
        "unbecome",
        "nothing is ever seated, so there is no body to step out of and no position to hand back.",
        "tests/rts_authority.rs",
    );
    None
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
    _q: Query<(&Position, &MoveOrder, &mut Velocity), Commandable>,
    mut announced: Local<bool>,
) {
    refuse_once(
        &mut announced,
        "steer_toward_order",
        "ordered units never receive a velocity, so a squad ignores every move order.",
        "tests/rts_move_order.rs",
    );
}

/// Velocity → position, for units. Deliberately separate from
/// `physics::advance_player` / `advance_npc`: those integrate one
/// avatar and one wanderer, this integrates the commanded set.
pub fn advance_units(
    _q: Query<(&mut Position, &Velocity), With<UnitMarker>>,
    mut announced: Local<bool>,
) {
    refuse_once(
        &mut announced,
        "advance_units",
        "unit velocity is never integrated, so units hold position even once steering lands.",
        "tests/rts_move_order.rs",
    );
}

/// Pairwise separation: push overlapping units apart on XZ until every
/// pair is at least `2 * UNIT_RADIUS` apart. Without this a squad
/// sharing one order collapses into a single stacked column at the
/// target — visually one unit, and the reason an order tolerance can
/// never be a point tolerance.
pub fn separate_units(
    _q: Query<&mut Position, With<UnitMarker>>,
    mut announced: Local<bool>,
) {
    refuse_once(
        &mut announced,
        "separate_units",
        "units are never pushed apart, so a squad stacks into one column on arrival.",
        "tests/rts_move_order.rs",
    );
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
    pub fn nudge(&mut self, _wheel_notches: i32) {
        refuse(
            "CameraZoom::nudge",
            "the wheel does not move the camera, so zoom is stuck wherever it started.",
            "tests/rts_pointer.rs",
        );
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
/// `None` when the ray never meets the terrain within the camera's far
/// plane — a pointer aimed at the sky, which is a real thing a cursor
/// can do near the horizon and must not be answered with a fabricated
/// coordinate.
pub fn ground_under(_camera: &SceneCamera, _ndc: [f32; 2]) -> Option<Vec3> {
    refuse(
        "ground_under",
        "the pointer has no world position, so a right-click cannot name a destination.",
        "tests/rts_pointer.rs",
    );
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
    _camera: &SceneCamera,
    _units: &[(Entity, Vec3)],
    _corner_a: [f32; 2],
    _corner_b: [f32; 2],
) -> Vec<Entity> {
    refuse(
        "units_in_rect",
        "a drag selects nothing, so there is never anything for a right-click to command.",
        "tests/rts_pointer.rs",
    );
    Vec::new()
}

/// Register the command layer's systems in `Update`, in the order the
/// data flows: order → velocity → position → de-overlap. The test
/// drives this same registration, so the ordering under test is the
/// ordering that ships.
pub fn register(app: &mut App) {
    app.add_systems(
        Update,
        (
            steer_toward_order,
            advance_units.after(steer_toward_order),
            separate_units.after(advance_units),
        ),
    );
}

/// Surface an unimplemented function on the sacred error bus, naming
/// what breaks because of it and which red defines done.
///
/// Delete each call site as its function gains a body. A silent no-op
/// is exactly the swallowed refusal this project forbids: a body that
/// won't move, or a wheel that won't take, would read as a physics or
/// input bug rather than as work that hasn't been done.
///
/// Called directly from plain functions, which are invoked on a user
/// action and so are rare enough to be worth hearing about every time.
/// Per-tick systems go through `refuse_once`.
fn refuse(function: &str, effect: &str, red: &str) {
    crate::error::emit_region(
        crate::error::Severity::Error,
        format!("rts.{function}"),
        format!("`{function}` has no body yet"),
        format!("{effect} Implement `game::rts::{function}`; `{red}` defines done."),
    );
    crate::obs::emit(&format!("[rts.{function}] unimplemented — {effect}"));
}

/// `refuse`, but once per system instance rather than once per tick, so
/// a 600-tick run leaves one legible refusal per system instead of 1800
/// copies of it.
fn refuse_once(announced: &mut bool, system: &str, effect: &str, red: &str) {
    if *announced {
        return;
    }
    *announced = true;
    refuse(system, effect, red);
}
