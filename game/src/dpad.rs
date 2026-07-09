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

/// Base half-size of a button in NDC-y. The x half-size is derived
/// as (this / aspect) so buttons render square.
const BASE_HALF_Y: f32 = 0.06;
/// Distance from the D-pad centre to each button centre in NDC-y.
const SPACING_Y: f32 = 0.15;
/// Where the D-pad cross sits in NDC. Bottom-left corner of the
/// viewport, offset in from the edges.
const CENTER_X: f32 = -0.82;
const CENTER_Y: f32 = -0.72;

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
/// re-derived if the actual viewport size changes.
#[derive(Resource)]
pub struct Dpad {
    pub last_viewport: (u32, u32),
    pub buttons: [DpadButton; 4],
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
            }; 4],
        };
        rebuild_layout(&mut d, (1920, 1080));
        d
    }
}

pub fn setup_dpad(mut commands: Commands) {
    commands.insert_resource(Dpad::default());
}

/// Recompute button rectangles for a given viewport. Aspect-corrects
/// so a button is a square in pixels rather than a square in NDC.
fn rebuild_layout(dpad: &mut Dpad, viewport: (u32, u32)) {
    let (w, h) = viewport;
    let aspect = if h == 0 { 1.0 } else { (w as f32 / h as f32).max(0.5) };
    let half_y = BASE_HALF_Y;
    let half_x = BASE_HALF_Y / aspect;
    let sp_y = SPACING_Y;
    let sp_x = SPACING_Y / aspect;
    // Order: W (up), A (left), S (down), D (right)
    dpad.buttons[0] = DpadButton {
        center_ndc: [CENTER_X, CENTER_Y + sp_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::W,
        pressed: false,
    };
    dpad.buttons[1] = DpadButton {
        center_ndc: [CENTER_X - sp_x, CENTER_Y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::A,
        pressed: false,
    };
    dpad.buttons[2] = DpadButton {
        center_ndc: [CENTER_X, CENTER_Y - sp_y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::S,
        pressed: false,
    };
    dpad.buttons[3] = DpadButton {
        center_ndc: [CENTER_X + sp_x, CENTER_Y],
        half_size_ndc: [half_x, half_y],
        bit: input::key::D,
        pressed: false,
    };
    dpad.last_viewport = viewport;
}

thread_local! {
    static DPAD_INSTANCES: RefCell<[DpadInstance; 4]> = const {
        RefCell::new([DpadInstance {
            center_ndc: [0.0, 0.0],
            half_size_ndc: [0.0, 0.0],
            color: [0.0, 0.0, 0.0],
            alpha: 0.0,
        }; 4])
    };
}

/// Copy-out of the current D-pad instances, for render_web to
/// upload each frame after the world pass.
pub fn current_instances() -> [DpadInstance; 4] {
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
    let mut instances = [DpadInstance::default(); 4];
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

    #[test]
    fn layout_is_aspect_corrected() {
        let mut d = Dpad {
            last_viewport: (0, 0),
            buttons: [DpadButton {
                center_ndc: [0.0, 0.0],
                half_size_ndc: [0.0, 0.0],
                bit: 0,
                pressed: false,
            }; 4],
        };
        rebuild_layout(&mut d, (1920, 1080));
        let landscape_half_x = d.buttons[0].half_size_ndc[0];
        rebuild_layout(&mut d, (1080, 1920));
        let portrait_half_x = d.buttons[0].half_size_ndc[0];
        // Portrait has aspect < 1 → half_x = BASE_HALF_Y / aspect is
        // larger than landscape.
        assert!(portrait_half_x > landscape_half_x);
    }
}
