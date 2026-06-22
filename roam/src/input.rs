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
    pub dt_ms: f32,
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

            FrameInput {
                move_bits: bits,
                spawn_pressed: i.key_pressed(Key::Num5),
                dt_ms: (i.stable_dt * 1000.0).min(100.0),
            }
        })
    }
}
