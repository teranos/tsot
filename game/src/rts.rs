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
/// pressing the key", it is "was never ordered" — an unordered unit
/// holds position forever.
#[derive(Component, Clone, Copy, Debug)]
pub struct MoveOrder(pub Vec3);

/// Order → velocity. Points each ordered unit at its `MoveOrder` on the
/// XZ plane at `UNIT_SPEED`, and stops it inside `ARRIVE_RADIUS` so
/// arrived units don't jitter around the target forever.
pub fn steer_toward_order(
    _q: Query<(&Position, &MoveOrder, &mut Velocity), With<UnitMarker>>,
    mut announced: Local<bool>,
) {
    refuse_once(
        &mut announced,
        "steer_toward_order",
        "ordered units never receive a velocity, so a squad ignores every move order.",
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
    );
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

/// Surface an unimplemented system on the sacred error bus — once per
/// system instance, not once per tick, so a 600-tick run leaves one
/// legible refusal per system instead of 1800 copies of it.
///
/// Delete each call site as its system gains a body. A silent no-op
/// system is exactly the swallowed refusal this project forbids: units
/// that don't move would read as a physics bug rather than as work
/// that hasn't been done.
fn refuse_once(announced: &mut bool, system: &str, effect: &str) {
    if *announced {
        return;
    }
    *announced = true;
    crate::error::emit_region(
        crate::error::Severity::Error,
        format!("rts.{system}"),
        format!("`{system}` has no body yet"),
        format!(
            "{effect} Implement `game::rts::{system}`; \
             `tests/rts_move_order.rs` defines done."
        ),
    );
    crate::obs::emit(&format!("[rts.{system}] unimplemented — {effect}"));
}
