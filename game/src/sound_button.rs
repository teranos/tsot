// Bevy-owned sound toggle button. Renders through the same UI
// overlay pipeline as the D-pad; single button in the top-right
// corner that toggles crate::audio::AUDIO_MUTED on a rising-edge
// touch (tap, not hold). Colour reflects state — green while
// sound is playing, red when muted.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::dpad::DpadInstance;

const BUTTON_HALF_PX: f32 = 40.0;
const MARGIN_TOP_PX: f32 = 20.0;
const MARGIN_RIGHT_PX: f32 = 20.0;

#[derive(Resource)]
pub struct SoundButton {
    pub last_viewport: (u32, u32),
    pub center_ndc: [f32; 2],
    pub half_size_ndc: [f32; 2],
    pub pressed: bool,
    pub was_pressed: bool,
}

impl Default for SoundButton {
    fn default() -> Self {
        let mut sb = Self {
            last_viewport: (0, 0),
            center_ndc: [0.0, 0.0],
            half_size_ndc: [0.0, 0.0],
            pressed: false,
            was_pressed: false,
        };
        rebuild_layout(&mut sb, (1920, 1080));
        sb
    }
}

fn rebuild_layout(sb: &mut SoundButton, viewport: (u32, u32)) {
    let (w, h) = viewport;
    if w == 0 || h == 0 {
        return;
    }
    let ndc_per_x_px = 2.0 / w as f32;
    let ndc_per_y_px = 2.0 / h as f32;
    let half_x = BUTTON_HALF_PX * ndc_per_x_px;
    let half_y = BUTTON_HALF_PX * ndc_per_y_px;
    let margin_r = MARGIN_RIGHT_PX * ndc_per_x_px;
    let margin_t = MARGIN_TOP_PX * ndc_per_y_px;
    // Top-right corner: NDC +1 is right, +1 is top.
    sb.center_ndc = [1.0 - margin_r - half_x, 1.0 - margin_t - half_y];
    sb.half_size_ndc = [half_x, half_y];
    sb.last_viewport = viewport;
}

pub fn setup_sound_button(mut commands: Commands) {
    commands.insert_resource(SoundButton::default());
}

thread_local! {
    static SOUND_INSTANCE: RefCell<DpadInstance> = const {
        RefCell::new(DpadInstance {
            center_ndc: [0.0, 0.0],
            half_size_ndc: [0.0, 0.0],
            color: [0.0, 0.0, 0.0],
            alpha: 0.0,
        })
    };
}

/// Copy-out of the current sound-button instance, for render_web to
/// upload alongside the D-pad instances.
pub fn current_instance() -> DpadInstance {
    SOUND_INSTANCE.with(|c| *c.borrow())
}

/// Hit-test touches, edge-detect mute toggle, publish UI instance.
pub fn sound_button_system(mut sb: ResMut<SoundButton>) {
    let viewport = crate::gpu_web::viewport_size();
    if viewport != sb.last_viewport && viewport.0 > 0 && viewport.1 > 0 {
        rebuild_layout(&mut sb, viewport);
    }
    let touches = crate::gpu_web::touches();
    let mut pressed_now = false;
    for touch in touches.iter().copied() {
        let dx = (touch[0] - sb.center_ndc[0]).abs();
        let dy = (touch[1] - sb.center_ndc[1]).abs();
        if dx <= sb.half_size_ndc[0] && dy <= sb.half_size_ndc[1] {
            pressed_now = true;
            break;
        }
    }
    // Rising-edge tap toggles mute — tap not hold.
    if pressed_now && !sb.was_pressed {
        let new_muted = !crate::audio::is_muted();
        crate::audio::set_muted(new_muted);
    }
    sb.pressed = pressed_now;
    sb.was_pressed = pressed_now;
    let muted = crate::audio::is_muted();
    let (color, alpha) = if muted {
        // Muted — soft red so state is unambiguous at a glance.
        ([0.85, 0.35, 0.4], 0.85)
    } else if sb.pressed {
        // Being tapped — bright white flash for feedback.
        ([0.95, 0.95, 1.0], 0.9)
    } else {
        // Playing — muted green.
        ([0.45, 0.8, 0.55], 0.8)
    };
    SOUND_INSTANCE.with(|c| {
        *c.borrow_mut() = DpadInstance {
            center_ndc: sb.center_ndc,
            half_size_ndc: sb.half_size_ndc,
            color,
            alpha,
        }
    });
}
