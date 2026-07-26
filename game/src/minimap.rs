//! Fixed-scale, player-centered minimap, drawn through the same
//! UI-overlay quad pipeline as the D-pad/HUD (see `dpad.rs`). Pure
//! projection: render_web already holds the frame's `SceneSnapshot`,
//! so this just turns those world positions into more `DpadInstance`
//! quads — no bevy system, no input, no new render pass.
//!
//! Screen orientation matches the isometric camera's WASD convention
//! from `physics::keyboard_input` ("W is 'up on screen' ≈ world
//! (-X, 0, -Z)") rather than raw world axes, so the map doesn't feel
//! rotated relative to the movement the player already knows.

use bevy_math::Vec3;

use crate::dpad::DpadInstance;
use crate::scene::SceneSnapshot;

/// World-space radius the minimap covers around the player. Bigger
/// than the follow camera's `FOLLOW_HALF_EXTENT` (scene::camera) so
/// the minimap surfaces what's beyond the visible screen — the
/// "discovery" case flagged in that module's doc comment — not just
/// what's already on it.
const RANGE_WORLD: f32 = 4000.0;
/// Panel half-size in pixels (square panel).
const PANEL_HALF_PX: f32 = 70.0;
/// Inset from the top-right viewport corner, pixels.
const MARGIN_PX: f32 = 20.0;
/// Dot half-size in pixels.
const DOT_HALF_PX: f32 = 3.0;
/// Player's own dot is bigger so it reads as "you are here".
const PLAYER_DOT_HALF_PX: f32 = 4.0;

const PANEL_COLOR: [f32; 3] = [0.05, 0.05, 0.08];
const PANEL_ALPHA: f32 = 0.55;
const PLAYER_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
const NPC_COLOR: [f32; 3] = [0.9, 0.25, 0.2];
const PIN_COLOR: [f32; 3] = [0.9, 0.8, 0.2];

/// Unnormalised world directions for "right on screen" / "up on
/// screen" under the true-isometric camera, matching
/// `physics::keyboard_input`'s D/W bindings exactly (D ≈ (1,0,-1), W
/// ≈ (-1,0,-1)). Both have length sqrt(2); `project` divides by that.
const SCREEN_RIGHT: (f32, f32) = (1.0, -1.0);
const SCREEN_UP: (f32, f32) = (-1.0, -1.0);
const AXIS_LEN: f32 = std::f32::consts::SQRT_2;

/// World-space `delta` (relative to the player) → normalised
/// screen-space (right, up), each in roughly [-1, 1] once divided by
/// `RANGE_WORLD`. Returns `None` if `delta` falls outside the map's
/// range.
fn project(delta: Vec3) -> Option<(f32, f32)> {
    let right = (delta.x * SCREEN_RIGHT.0 + delta.z * SCREEN_RIGHT.1) / AXIS_LEN;
    let up = (delta.x * SCREEN_UP.0 + delta.z * SCREEN_UP.1) / AXIS_LEN;
    if right * right + up * up > RANGE_WORLD * RANGE_WORLD {
        return None;
    }
    Some((right / RANGE_WORLD, up / RANGE_WORLD))
}

/// Build this frame's minimap quads: a background panel anchored to
/// the top-right corner, plus one dot per NPC / named pin / remote
/// peer within `RANGE_WORLD`, plus the player's own dot at the
/// panel's centre. Empty on a zero-sized viewport.
pub fn quads(snap: &SceneSnapshot, viewport: (u32, u32)) -> Vec<DpadInstance> {
    let (w, h) = viewport;
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let ndc_per_x_px = 2.0 / w as f32;
    let ndc_per_y_px = 2.0 / h as f32;
    let panel_half = [PANEL_HALF_PX * ndc_per_x_px, PANEL_HALF_PX * ndc_per_y_px];
    let center = [
        1.0 - MARGIN_PX * ndc_per_x_px - panel_half[0],
        1.0 - MARGIN_PX * ndc_per_y_px - panel_half[1],
    ];
    let dot_half = [DOT_HALF_PX * ndc_per_x_px, DOT_HALF_PX * ndc_per_y_px];
    let player_dot_half = [
        PLAYER_DOT_HALF_PX * ndc_per_x_px,
        PLAYER_DOT_HALF_PX * ndc_per_y_px,
    ];

    let mut out = vec![DpadInstance {
        center_ndc: center,
        half_size_ndc: panel_half,
        color: PANEL_COLOR,
        alpha: PANEL_ALPHA,
    }];

    let mut push_dot = |world: Vec3, color: [f32; 3], half: [f32; 2]| {
        if let Some((right, up)) = project(world - snap.player) {
            out.push(DpadInstance {
                center_ndc: [
                    center[0] + right * panel_half[0],
                    center[1] + up * panel_half[1],
                ],
                half_size_ndc: half,
                color,
                alpha: 1.0,
            });
        }
    };

    for pos in &snap.npcs {
        push_dot(*pos, NPC_COLOR, dot_half);
    }
    for pos in &snap.pins {
        push_dot(*pos, PIN_COLOR, dot_half);
    }
    for peer in &snap.remote_peers {
        push_dot(peer.pos, peer.color, dot_half);
    }
    push_dot(snap.player, PLAYER_COLOR, player_dot_half);

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::RemotePeerDot;

    fn empty_snapshot(player: Vec3) -> SceneSnapshot {
        SceneSnapshot {
            trees: Vec::new(),
            obstacles: Vec::new(),
            fires: Vec::new(),
            npcs: Vec::new(),
            pins: Vec::new(),
            trails: Vec::new(),
            remote_peers: Vec::new(),
            structures: Vec::new(),
            jukeboxes: Vec::new(),
            player,
        }
    }

    #[test]
    fn player_dot_sits_at_panel_center() {
        let snap = empty_snapshot(Vec3::new(1234.0, 0.0, -987.0));
        let out = quads(&snap, (1920, 1080));
        // Panel background + the player's own dot, nothing else.
        assert_eq!(out.len(), 2);
        let panel = out[0];
        let player_dot = out[1];
        assert_eq!(player_dot.center_ndc, panel.center_ndc);
        assert_eq!(player_dot.color, PLAYER_COLOR);
    }

    #[test]
    fn w_direction_maps_to_screen_up() {
        // physics::keyboard_input: W ≈ world (-1, 0, -1) — exactly
        // SCREEN_UP. An NPC sitting in that direction from the player
        // should land straight up from the panel centre (x unchanged).
        let mut snap = empty_snapshot(Vec3::ZERO);
        snap.npcs.push(Vec3::new(-1000.0, 0.0, -1000.0));
        let out = quads(&snap, (1920, 1080));
        let panel = out[0];
        let npc_dot = out
            .iter()
            .find(|q| q.color == NPC_COLOR)
            .expect("npc dot present");
        assert!((npc_dot.center_ndc[0] - panel.center_ndc[0]).abs() < 1e-5);
        assert!(npc_dot.center_ndc[1] > panel.center_ndc[1]);
    }

    #[test]
    fn d_direction_maps_to_screen_right() {
        // D ≈ world (1, 0, -1) — exactly SCREEN_RIGHT.
        let mut snap = empty_snapshot(Vec3::ZERO);
        snap.npcs.push(Vec3::new(1000.0, 0.0, -1000.0));
        let out = quads(&snap, (1920, 1080));
        let panel = out[0];
        let npc_dot = out
            .iter()
            .find(|q| q.color == NPC_COLOR)
            .expect("npc dot present");
        assert!(npc_dot.center_ndc[0] > panel.center_ndc[0]);
        assert!((npc_dot.center_ndc[1] - panel.center_ndc[1]).abs() < 1e-5);
    }

    #[test]
    fn entities_beyond_range_are_dropped() {
        let mut snap = empty_snapshot(Vec3::ZERO);
        snap.npcs.push(Vec3::new(0.0, 0.0, -(RANGE_WORLD * 10.0)));
        snap.pins.push(Vec3::new(RANGE_WORLD * 10.0, 0.0, 0.0));
        snap.remote_peers.push(RemotePeerDot {
            pos: Vec3::new(RANGE_WORLD * 10.0, 0.0, RANGE_WORLD * 10.0),
            color: [0.2, 0.6, 0.9],
        });
        let out = quads(&snap, (1920, 1080));
        // Only the panel background + the player's own dot survive.
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn panel_stays_in_top_right_on_any_viewport() {
        for viewport in [(1920, 1080), (1080, 1920), (390, 844)] {
            let snap = empty_snapshot(Vec3::ZERO);
            let out = quads(&snap, viewport);
            let panel = out[0];
            let right_edge = panel.center_ndc[0] + panel.half_size_ndc[0];
            let top_edge = panel.center_ndc[1] + panel.half_size_ndc[1];
            assert!(right_edge < 1.0, "{viewport:?}: panel right edge off-screen");
            assert!(top_edge < 1.0, "{viewport:?}: panel top edge off-screen");
            assert!(panel.center_ndc[0] > 0.0, "{viewport:?}: panel not on the right");
            assert!(panel.center_ndc[1] > 0.0, "{viewport:?}: panel not on top");
        }
    }

    #[test]
    fn zero_viewport_yields_no_quads() {
        let snap = empty_snapshot(Vec3::ZERO);
        assert!(quads(&snap, (0, 1080)).is_empty());
        assert!(quads(&snap, (1920, 0)).is_empty());
    }
}
