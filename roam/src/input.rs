//! Keyboard → game-input mapping.
//!
//! One module, one mapping. Every surface that needs to know what the
//! player pressed this frame calls `FrameInput::read(ctx)`. No other
//! file reads `egui::Key::*` directly. When a new key binding lands
//! (zoom, inventory toggle, future debug overlay), it lands here and
//! nowhere else.

/// What the player asked for this frame, distilled into typed fields.
///
/// `move_bits` is the bitmask `world::step` expects (`INPUT_W` |
/// `INPUT_A` | `INPUT_S` | `INPUT_D`). The other fields are
/// edge-triggered — true the frame a key transitioned from up to
/// down, false otherwise — so holding the key down doesn't fire the
/// action repeatedly.
#[derive(Debug, Clone, Copy)]
pub struct FrameInput {
    pub move_bits: u32,
    pub spawn_pressed: bool,
    /// Tab key — toggle the full inventory panel above the hotbar.
    /// Hotbar itself is always visible; this just opens/closes the
    /// extended grid for the slots beyond the first 9.
    pub inventory_toggle_pressed: bool,
    /// `+` / `=` — step zoom in. Edge-triggered.
    pub zoom_in_pressed: bool,
    /// `-` — step zoom out. Edge-triggered.
    pub zoom_out_pressed: bool,
    /// Virtual-joystick origin (where the active touch / primary press
    /// went down) in screen pixels, `None` when no press is active.
    /// Carried so the UI can draw the joystick base where the thumb
    /// landed; the direction itself is already folded into `move_bits`.
    pub touch_origin: Option<(f32, f32)>,
    /// Live position of the active touch / primary press in screen
    /// pixels — the joystick knob. `None` when no press is active.
    pub touch_pos: Option<(f32, f32)>,
    pub dt_ms: f32,
}

/// Below this many screen pixels of drag from the touch origin, the
/// virtual joystick reads as centred — no movement. Keeps a resting
/// thumb (or a tap that was meant as a click) from creeping the player.
pub const TOUCH_DEADZONE_PX: f32 = 16.0;

/// Map a touch-drag vector (current pointer minus the press origin, in
/// screen pixels with **y pointing down**) onto the same 8-way
/// direction bitmask the keyboard feeds. This is the mobile analogue
/// of "which numpad key" — the drag direction picks one of the eight
/// octants; a drag shorter than `deadzone` reads as centred (`0`).
///
/// Pure function (no egui, no FFI) so the octant boundaries are unit-
/// testable on the host. `read` is the only caller in the wasm build.
pub fn drag_to_move_bits(dx: f32, dy: f32, deadzone: f32) -> u32 {
    use crate::world::{INPUT_A, INPUT_D, INPUT_S, INPUT_W};

    if dx * dx + dy * dy < deadzone * deadzone {
        return 0;
    }
    // atan2 with screen-down y: 0° = East, 90° = South, 180° = West,
    // 270° = North. Bias by half an octant (22.5°) before flooring so
    // each cardinal sits at the centre of its 45°-wide sector.
    let deg = (dy.atan2(dx).to_degrees() + 360.0) % 360.0;
    let sector = (((deg + 22.5) % 360.0) / 45.0) as u32;
    match sector {
        0 => INPUT_D,
        1 => INPUT_D | INPUT_S,
        2 => INPUT_S,
        3 => INPUT_S | INPUT_A,
        4 => INPUT_A,
        5 => INPUT_A | INPUT_W,
        6 => INPUT_W,
        7 => INPUT_W | INPUT_D,
        _ => 0,
    }
}

impl FrameInput {
    /// Read this frame's input from egui. Three keyboard schemes feed
    /// the same direction bitmask so the player picks whichever feels
    /// natural: WASD (gamer-default), arrow keys (discoverable),
    /// numpad / number row 1-9 (roguelike 8-way, diagonals on one key
    /// — egui collapses `Numpad1` into `Num1`). The numpad layout:
    ///
    /// ```text
    /// 7 8 9       NW N NE
    /// 4 5 6   →    W . E
    /// 1 2 3       SW S SE
    /// ```
    ///
    /// `5` is the "act on self" key — currently teleports to the
    /// spawn tile (same effect as the right-click spawn menu).
    pub fn read(ctx: &egui::Context) -> Self {
        use crate::world::{INPUT_A, INPUT_D, INPUT_S, INPUT_W};
        use egui::Key;

        ctx.input(|i| {
            let mut bits: u32 = 0;
            if i.key_down(Key::W) || i.key_down(Key::ArrowUp) || i.key_down(Key::Num8) {
                bits |= INPUT_W;
            }
            if i.key_down(Key::A) || i.key_down(Key::ArrowLeft) || i.key_down(Key::Num4) {
                bits |= INPUT_A;
            }
            if i.key_down(Key::S) || i.key_down(Key::ArrowDown) || i.key_down(Key::Num2) {
                bits |= INPUT_S;
            }
            if i.key_down(Key::D) || i.key_down(Key::ArrowRight) || i.key_down(Key::Num6) {
                bits |= INPUT_D;
            }
            if i.key_down(Key::Num7) {
                bits |= INPUT_W | INPUT_A;
            }
            if i.key_down(Key::Num9) {
                bits |= INPUT_W | INPUT_D;
            }
            if i.key_down(Key::Num1) {
                bits |= INPUT_S | INPUT_A;
            }
            if i.key_down(Key::Num3) {
                bits |= INPUT_S | INPUT_D;
            }

            // Touch / pointer-drag movement. A primary press becomes a
            // virtual joystick: the press position is the origin, the
            // live position is the stick, and the drag vector picks an
            // octant via `drag_to_move_bits`. This is how a phone with
            // no keyboard moves the player; on desktop a left-drag does
            // the same (right-drag still belongs to the context menu).
            let (touch_origin, touch_pos) =
                match (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.interact_pos()) {
                    (true, Some(o), Some(p)) => {
                        bits |= drag_to_move_bits(p.x - o.x, p.y - o.y, TOUCH_DEADZONE_PX);
                        (Some((o.x, o.y)), Some((p.x, p.y)))
                    }
                    _ => (None, None),
                };

            FrameInput {
                move_bits: bits,
                spawn_pressed: i.key_pressed(Key::Num5),
                inventory_toggle_pressed: i.key_pressed(Key::Tab),
                zoom_in_pressed: i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                zoom_out_pressed: i.key_pressed(Key::Minus),
                touch_origin,
                touch_pos,
                dt_ms: (i.stable_dt * 1000.0).min(100.0),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{INPUT_A, INPUT_D, INPUT_S, INPUT_W};

    #[test]
    fn deadzone_reads_as_centred() {
        // A drag shorter than the deadzone is no movement at all.
        assert_eq!(drag_to_move_bits(0.0, 0.0, TOUCH_DEADZONE_PX), 0);
        assert_eq!(drag_to_move_bits(5.0, -5.0, TOUCH_DEADZONE_PX), 0);
    }

    #[test]
    fn cardinals_map_to_single_axis() {
        // Screen y points down: +y is South, -y is North.
        assert_eq!(drag_to_move_bits(40.0, 0.0, TOUCH_DEADZONE_PX), INPUT_D);
        assert_eq!(drag_to_move_bits(-40.0, 0.0, TOUCH_DEADZONE_PX), INPUT_A);
        assert_eq!(drag_to_move_bits(0.0, 40.0, TOUCH_DEADZONE_PX), INPUT_S);
        assert_eq!(drag_to_move_bits(0.0, -40.0, TOUCH_DEADZONE_PX), INPUT_W);
    }

    #[test]
    fn diagonals_combine_two_axes() {
        assert_eq!(drag_to_move_bits(40.0, 40.0, TOUCH_DEADZONE_PX), INPUT_D | INPUT_S);
        assert_eq!(drag_to_move_bits(-40.0, 40.0, TOUCH_DEADZONE_PX), INPUT_S | INPUT_A);
        assert_eq!(drag_to_move_bits(-40.0, -40.0, TOUCH_DEADZONE_PX), INPUT_A | INPUT_W);
        assert_eq!(drag_to_move_bits(40.0, -40.0, TOUCH_DEADZONE_PX), INPUT_W | INPUT_D);
    }

    #[test]
    fn octant_centres_are_stable_just_off_axis() {
        // A few degrees off a cardinal still reads as that cardinal,
        // not a flickering diagonal — the half-octant bias guarantees it.
        assert_eq!(drag_to_move_bits(40.0, 6.0, TOUCH_DEADZONE_PX), INPUT_D);
        assert_eq!(drag_to_move_bits(40.0, -6.0, TOUCH_DEADZONE_PX), INPUT_D);
    }
}
