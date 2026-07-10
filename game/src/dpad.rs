// Bevy-owned mobile D-pad. Replaces the earlier HTML/CSS overlay
// that violated the "Rust owns render" axiom from game/CLAUDE.md.
//
// Data flow:
//   1. dpad_input_system polls the current viewport size from
//      gpu_web (via game_viewport_size env.*) and rebuilds the four
//      button rectangles in NDC whenever the viewport changes.
//      Rectangles are aspect-corrected so buttons render square
//      regardless of portrait/landscape.
//   2. It polls the current touch positions (up to 8) via
//      gpu_web::touches() → game_touch_state env.*. Each touch is
//      already in NDC (JS converts client → NDC before writing).
//   3. Point-in-rectangle test per (touch, button) pair sets each
//      button's `pressed` state; the union of pressed buttons'
//      bits is stored via input::set_touch_bits so the keyboard
//      input path ORs it in.
//   4. The four button instances (position + size + color modulated
//      by pressed state) are stashed in a thread_local; render_web
//      reads them after the world render and draws them via the
//      UI overlay pipeline.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::input;

/// Button half-size in pixels — same in both axes so buttons render
/// as squares regardless of viewport aspect. 50 px half → 100 px
/// full, tap-sized on phones and small enough that 3x3 fits in a
/// reasonable corner footprint.
const BUTTON_HALF_PX: f32 = 50.0;
/// Distance from the D-pad centre to each orthogonal button centre,
/// in pixels. Bigger than BUTTON_HALF_PX so buttons don't overlap.
const BUTTON_SPACING_PX: f32 = 96.0;
/// Corner button inset from the orthogonal spacing — the four
/// diagonals sit at (±SPACING, ±SPACING) from the centre.
const CORNER_SPACING_PX: f32 = 96.0;
/// Left-edge inset from the viewport's left edge, pixels.
const MARGIN_LEFT_PX: f32 = 20.0;
/// Bottom-edge inset from the viewport's bottom edge, pixels.
const MARGIN_BOTTOM_PX: f32 = 20.0;

/// One D-pad button as tracked between frames. bit is the
/// input::key::* value it contributes to the touch bitmask when
/// pressed.
#[derive(Clone, Copy, Debug)]
pub struct DpadButton {
    pub center_ndc: [f32; 2],
    pub half_size_ndc: [f32; 2],
    pub bit: u32,
    pub pressed: bool,
}

/// One rendered UI instance sent to the UI overlay pipeline. Layout
/// matches the WGSL `UiInstance` struct in scene::UI_SHADER_WGSL
/// exactly (32 bytes stride). Aligned trivially by field order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DpadInstance {
    pub center_ndc: [f32; 2],
    pub half_size_ndc: [f32; 2],
    pub color: [f32; 3],
    pub alpha: f32,
}

/// Bevy resource. `last_viewport` is what layout was built for —
/// re-derived if the actual viewport size changes. Eight buttons
/// in a 3x3 grid (centre empty): four orthogonal (W/A/S/D) plus
/// four diagonals (NW/NE/SW/SE) with OR'd input bits so a single
/// touch produces the expected diagonal motion.
#[derive(Resource)]
pub struct Dpad {
    pub last_viewport: (u32, u32),
    pub buttons: [DpadButton; 8],
}

impl Default for Dpad {
    fn default() -> Self {
        let mut d = Self {
            last_viewport: (0, 0),
            buttons: [DpadButton {
                center_ndc: [0.0, 0.0],
                half_size_ndc: [0.0, 0.0],
                bit: 0,
                pressed: false,
            }; 8],
        };
        rebuild_layout(&mut d, (1920, 1080));
        d
    }
}

pub fn setup_dpad(mut commands: Commands) {
    commands.insert_resource(Dpad::default());
}

/// Recompute button rectangles for a given viewport. Uses pixel-based
/// sizing so buttons are always a fixed pixel size (square, tap-sized)
/// and the outermost buttons sit inside a safe margin from the
/// viewport edge — no clipping on narrow portrait aspects. Eight
/// buttons: N/S/E/W plus the four diagonals with OR'd bits.
fn rebuild_layout(dpad: &mut Dpad, viewport: (u32, u32)) {
    let (w, h) = viewport;
    if w == 0 || h == 0 {
        return;
    }
    // NDC spans 2 units for each viewport dimension. To convert pixel
    // distances into NDC units, divide by (dim/2).
    let ndc_per_x_px = 2.0 / w as f32;
    let ndc_per_y_px = 2.0 / h as f32;
    let half_x = BUTTON_HALF_PX * ndc_per_x_px;
    let half_y = BUTTON_HALF_PX * ndc_per_y_px;
    let sp_x = BUTTON_SPACING_PX * ndc_per_x_px;
    let sp_y = BUTTON_SPACING_PX * ndc_per_y_px;
    let corner_x = CORNER_SPACING_PX * ndc_per_x_px;
    let corner_y = CORNER_SPACING_PX * ndc_per_y_px;
    let margin_left = MARGIN_LEFT_PX * ndc_per_x_px;
    let margin_bottom = MARGIN_BOTTOM_PX * ndc_per_y_px;
    // Anchor so that the SW corner button's outer edge sits at the
    // safe-margin inset. D-pad centre is corner_spacing + half in
    // from that.
    let center_x = -1.0 + margin_left + half_x + corner_x;
    let center_y = -1.0 + margin_bottom + half_y + corner_y;
    // Diagonal bit compositions
    let nw = input::key::W | input::key::A;
    let ne = input::key::W | input::key::D;
    let sw = input::key::S | input::key::A;
    let se = input::key::S | input::key::D;
    // Layout order (arbitrary): N, E, S, W, NE, SE, SW, NW
    dpad.buttons[0] = DpadButton {
        center_ndc: [center_x, center_y + sp_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::W,
        pressed: false,
    };
    dpad.buttons[1] = DpadButton {
        center_ndc: [center_x + sp_x, center_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::D,
        pressed: false,
    };
    dpad.buttons[2] = DpadButton {
        center_ndc: [center_x, center_y - sp_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::S,
        pressed: false,
    };
    dpad.buttons[3] = DpadButton {
        center_ndc: [center_x - sp_x, center_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::A,
        pressed: false,
    };
    dpad.buttons[4] = DpadButton {
        center_ndc: [center_x + corner_x, center_y + corner_y],
        half_size_ndc: [half_x, half_y],
        bit: ne,
        pressed: false,
    };
    dpad.buttons[5] = DpadButton {
        center_ndc: [center_x + corner_x, center_y - corner_y],
        half_size_ndc: [half_x, half_y],
        bit: se,
        pressed: false,
    };
    dpad.buttons[6] = DpadButton {
        center_ndc: [center_x - corner_x, center_y - corner_y],
        half_size_ndc: [half_x, half_y],
        bit: sw,
        pressed: false,
    };
    dpad.buttons[7] = DpadButton {
        center_ndc: [center_x - corner_x, center_y + corner_y],
        half_size_ndc: [half_x, half_y],
        bit: nw,
        pressed: false,
    };
    dpad.last_viewport = viewport;
}

thread_local! {
    static DPAD_INSTANCES: RefCell<[DpadInstance; 8]> = const {
        RefCell::new([DpadInstance {
            center_ndc: [0.0, 0.0],
            half_size_ndc: [0.0, 0.0],
            color: [0.0, 0.0, 0.0],
            alpha: 0.0,
        }; 8])
    };
}

/// Copy-out of the current D-pad instances, for render_web to
/// upload each frame after the world pass.
pub fn current_instances() -> [DpadInstance; 8] {
    DPAD_INSTANCES.with(|c| *c.borrow())
}

/// Hit-test touches, update input, publish rendering instances.
/// Runs each frame.
pub fn dpad_input_system(mut dpad: ResMut<Dpad>) {
    let viewport = crate::gpu_web::viewport_size();
    if viewport != dpad.last_viewport && viewport.0 > 0 && viewport.1 > 0 {
        rebuild_layout(&mut dpad, viewport);
    }
    let touches = crate::gpu_web::touches();
    for btn in dpad.buttons.iter_mut() {
        btn.pressed = false;
    }
    let mut bits = 0u32;
    for touch in touches.iter().copied() {
        for btn in dpad.buttons.iter_mut() {
            let dx = (touch[0] - btn.center_ndc[0]).abs();
            let dy = (touch[1] - btn.center_ndc[1]).abs();
            if dx <= btn.half_size_ndc[0] && dy <= btn.half_size_ndc[1] {
                btn.pressed = true;
                bits |= btn.bit;
            }
        }
    }
    input::set_touch_bits(bits);
    let mut instances = [DpadInstance::default(); 8];
    for (i, btn) in dpad.buttons.iter().enumerate() {
        let (color, alpha) = if btn.pressed {
            ([0.55, 0.55, 0.6], 0.9)
        } else {
            ([0.12, 0.12, 0.16], 0.7)
        };
        instances[i] = DpadInstance {
            center_ndc: btn.center_ndc,
            half_size_ndc: btn.half_size_ndc,
            color,
            alpha,
        };
    }
    DPAD_INSTANCES.with(|c| *c.borrow_mut() = instances);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank_dpad() -> Dpad {
        Dpad {
            last_viewport: (0, 0),
            buttons: [DpadButton {
                center_ndc: [0.0, 0.0],
                half_size_ndc: [0.0, 0.0],
                bit: 0,
                pressed: false,
            }; 8],
        }
    }

    #[test]
    fn layout_is_pixel_sized() {
        let mut d = blank_dpad();
        rebuild_layout(&mut d, (1920, 1080));
        // 50 px half in a 1920-wide viewport → NDC 50 * 2 / 1920 = 0.0521.
        assert!((d.buttons[0].half_size_ndc[0] - 0.0520833).abs() < 1e-4);
        rebuild_layout(&mut d, (1080, 1920));
        // Narrower viewport → larger x-half in NDC.
        assert!(d.buttons[0].half_size_ndc[0] > 0.09);
    }

    #[test]
    fn all_buttons_stay_on_screen_on_portrait() {
        let mut d = blank_dpad();
        // iPhone-portrait-ish
        rebuild_layout(&mut d, (390, 844));
        for (i, btn) in d.buttons.iter().enumerate() {
            let left = btn.center_ndc[0] - btn.half_size_ndc[0];
            let bottom = btn.center_ndc[1] - btn.half_size_ndc[1];
            assert!(left > -1.0, "button {i} left edge {left} off-screen");
            assert!(
                bottom > -1.0,
                "button {i} bottom edge {bottom} off-screen"
            );
        }
    }

    #[test]
    fn diagonals_or_two_bits() {
        let d = blank_dpad();
        let d = {
            let mut d = d;
            rebuild_layout(&mut d, (1920, 1080));
            d
        };
        // Buttons 4-7 are diagonals — each carries two WASD bits.
        for i in 4..8 {
            let bits = d.buttons[i].bit;
            let count = bits.count_ones();
            assert_eq!(count, 2, "diagonal button {i} has bit={bits:04b}, expected 2 bits");
        }
    }
}
