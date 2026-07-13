//! Bevy-owned heads-up controls, drawn through the same UI-overlay
//! quad pipeline as the D-pad (see `dpad.rs`). Rust owns the render —
//! no HTML/CSS overlay — so these are just more `DpadInstance` quads
//! appended after the D-pad each frame.
//!
//! Controls:
//!   - Music toggle (bottom-left): tap to mute/unmute the track. Purple
//!     when playing, dim when muted.
//!   - Settings gear (top-left): tap to open/close the settings panel.
//!   - Settings panel: a modal backdrop with a horizontal volume
//!     slider; drag it to set the music level live.
//!
//! Interaction is hit-tested against `gpu_web::touches()`. Taps are
//! rising-edge per-control (so a thumb resting on the D-pad doesn't
//! swallow a tap on a HUD button); the slider is a continuous drag.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::dpad::DpadInstance;
use crate::music::Music;

/// Square button half-size in pixels — same in both axes so it renders
/// square regardless of aspect. Tap-sized.
const BTN_HALF_PX: f32 = 46.0;
/// Inset of a corner button from the viewport edge, pixels.
const MARGIN_PX: f32 = 26.0;

/// An NDC-space rectangle (centre + half-size), with a point test.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub cx: f32,
    pub cy: f32,
    pub hx: f32,
    pub hy: f32,
}

impl Rect {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        (p[0] - self.cx).abs() <= self.hx && (p[1] - self.cy).abs() <= self.hy
    }
}

/// Resolved on-screen geometry for one frame: the two corner buttons
/// plus, when open, the settings panel and its slider track.
pub struct HudLayout {
    pub music: Rect,
    pub gear: Rect,
    pub panel_open: bool,
    pub panel: Rect,
    pub slider: Rect,
}

/// Compute the HUD geometry for a viewport. Buttons are pixel-sized and
/// pinned to corners with a safe margin (mirrors the D-pad); the panel
/// is a centred dialog sized in NDC fractions.
pub fn compute_layout(viewport: (u32, u32), panel_open: bool) -> HudLayout {
    let (w, h) = viewport;
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let ndc_x = 2.0 / w;
    let ndc_y = 2.0 / h;
    let half_x = BTN_HALF_PX * ndc_x;
    let half_y = BTN_HALF_PX * ndc_y;
    let inset_x = (MARGIN_PX + BTN_HALF_PX) * ndc_x;
    let inset_y = (MARGIN_PX + BTN_HALF_PX) * ndc_y;
    let music = Rect {
        cx: -1.0 + inset_x,
        cy: -1.0 + inset_y,
        hx: half_x,
        hy: half_y,
    };
    let gear = Rect {
        cx: -1.0 + inset_x,
        cy: 1.0 - inset_y,
        hx: half_x,
        hy: half_y,
    };
    // Centred modal. Slider track sits inside it, a bit below centre.
    let panel = Rect { cx: 0.0, cy: 0.0, hx: 0.42, hy: 0.30 };
    let slider = Rect { cx: 0.0, cy: -0.06, hx: 0.32, hy: 0.028 };
    HudLayout { music, gear, panel_open, panel, slider }
}

/// Map a touch x-coordinate to a volume in [0,1] across the slider
/// track's width.
pub fn volume_from_x(slider: &Rect, x: f32) -> f32 {
    let left = slider.cx - slider.hx;
    ((x - left) / (2.0 * slider.hx)).clamp(0.0, 1.0)
}

/// HUD state carried between frames: whether the panel is open, and the
/// previous-frame coverage of each tap button for rising-edge taps.
#[derive(Resource, Default)]
pub struct Hud {
    pub panel_open: bool,
    pub prev_music: bool,
    pub prev_gear: bool,
}

pub fn setup_hud(mut commands: Commands) {
    commands.insert_resource(Hud::default());
}

/// Build the overlay quads for the current layout + music state. When
/// the panel is closed that's just the two corner buttons; open, it's
/// the backdrop, the slider track, its filled portion, and the knob.
pub fn build_quads(layout: &HudLayout, music: &Music) -> Vec<DpadInstance> {
    let mut out = Vec::with_capacity(6);
    // Music toggle — purple when playing, dim when muted.
    let (mc, ma) = if music.playing {
        ([0.55, 0.35, 0.85], 0.9)
    } else {
        ([0.28, 0.24, 0.32], 0.7)
    };
    out.push(DpadInstance {
        center_ndc: [layout.music.cx, layout.music.cy],
        half_size_ndc: [layout.music.hx, layout.music.hy],
        color: mc,
        alpha: ma,
    });
    // Settings gear — brighter when the panel is open.
    let gc = if layout.panel_open {
        [0.60, 0.62, 0.70]
    } else {
        [0.32, 0.33, 0.40]
    };
    out.push(DpadInstance {
        center_ndc: [layout.gear.cx, layout.gear.cy],
        half_size_ndc: [layout.gear.hx, layout.gear.hy],
        color: gc,
        alpha: 0.85,
    });
    if layout.panel_open {
        // Backdrop.
        out.push(DpadInstance {
            center_ndc: [layout.panel.cx, layout.panel.cy],
            half_size_ndc: [layout.panel.hx, layout.panel.hy],
            color: [0.06, 0.06, 0.10],
            alpha: 0.92,
        });
        // Slider track.
        out.push(DpadInstance {
            center_ndc: [layout.slider.cx, layout.slider.cy],
            half_size_ndc: [layout.slider.hx, layout.slider.hy],
            color: [0.20, 0.20, 0.26],
            alpha: 1.0,
        });
        // Filled portion, left edge → knob, showing the current level.
        let left = layout.slider.cx - layout.slider.hx;
        let knob_x = left + music.volume * 2.0 * layout.slider.hx;
        let fill_hx = ((knob_x - left) * 0.5).max(0.0);
        out.push(DpadInstance {
            center_ndc: [left + fill_hx, layout.slider.cy],
            half_size_ndc: [fill_hx, layout.slider.hy],
            color: [0.45, 0.35, 0.75],
            alpha: 1.0,
        });
        // Knob.
        let knob_hx = layout.slider.hy; // square-ish grip
        out.push(DpadInstance {
            center_ndc: [knob_x, layout.slider.cy],
            half_size_ndc: [knob_hx, layout.slider.hy * 1.8],
            color: [0.88, 0.88, 0.94],
            alpha: 1.0,
        });
    }
    out
}

thread_local! {
    static HUD_INSTANCES: RefCell<Vec<DpadInstance>> = const { RefCell::new(Vec::new()) };
}

/// Copy-out of the current HUD quads, appended after the D-pad by
/// render_web each frame.
pub fn current_instances() -> Vec<DpadInstance> {
    HUD_INSTANCES.with(|c| c.borrow().clone())
}

/// Poll touches, drive the toggles + slider, publish the render quads.
pub fn hud_input_system(mut hud: ResMut<Hud>, mut music: ResMut<Music>) {
    let viewport = crate::gpu_web::viewport_size();
    let layout = compute_layout(viewport, hud.panel_open);
    let touches = crate::gpu_web::touches();

    // Rising-edge taps, per control.
    let music_cov = touches.iter().any(|&p| layout.music.contains(p));
    let gear_cov = touches.iter().any(|&p| layout.gear.contains(p));
    if music_cov && !hud.prev_music {
        music.toggle();
    }
    if gear_cov && !hud.prev_gear {
        hud.panel_open = !hud.panel_open;
    }
    hud.prev_music = music_cov;
    hud.prev_gear = gear_cov;

    // Continuous slider drag while the panel is open.
    if hud.panel_open {
        for &p in &touches {
            if layout.slider.contains(p) {
                music.set_volume(volume_from_x(&layout.slider, p[0]));
            }
        }
    }

    // Render quads reflect the (possibly just-toggled) panel state.
    let render_layout = compute_layout(viewport, hud.panel_open);
    let quads = build_quads(&render_layout, &music);
    HUD_INSTANCES.with(|c| *c.borrow_mut() = quads);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::GameAudioHandle;

    fn music(playing: bool, volume: f32) -> Music {
        Music {
            handle: GameAudioHandle::from_raw_for_test(0),
            playing,
            volume,
        }
    }

    #[test]
    fn corner_buttons_stay_on_screen_on_portrait() {
        let l = compute_layout((390, 844), false);
        for (name, r) in [("music", l.music), ("gear", l.gear)] {
            assert!(r.cx - r.hx > -1.0, "{name} left off-screen");
            assert!(r.cx + r.hx < 1.0, "{name} right off-screen");
            assert!(r.cy - r.hy > -1.0, "{name} bottom off-screen");
            assert!(r.cy + r.hy < 1.0, "{name} top off-screen");
        }
        // Music is bottom, gear is top.
        assert!(l.music.cy < 0.0 && l.gear.cy > 0.0);
    }

    #[test]
    fn slider_x_maps_across_the_track() {
        let l = compute_layout((1920, 1080), true);
        let s = l.slider;
        assert!((volume_from_x(&s, s.cx - s.hx) - 0.0).abs() < 1e-6, "left = 0");
        assert!((volume_from_x(&s, s.cx + s.hx) - 1.0).abs() < 1e-6, "right = 1");
        assert!((volume_from_x(&s, s.cx) - 0.5).abs() < 1e-6, "centre = 0.5");
        // Past the ends clamps.
        assert_eq!(volume_from_x(&s, -2.0), 0.0);
        assert_eq!(volume_from_x(&s, 2.0), 1.0);
    }

    #[test]
    fn panel_adds_quads_only_when_open() {
        let closed = build_quads(&compute_layout((1920, 1080), false), &music(true, 0.5));
        let open = build_quads(&compute_layout((1920, 1080), true), &music(true, 0.5));
        assert_eq!(closed.len(), 2, "just the two corner buttons when closed");
        assert!(open.len() > closed.len(), "the panel adds slider quads");
    }

    #[test]
    fn muted_music_button_reads_dim() {
        let on = build_quads(&compute_layout((1920, 1080), false), &music(true, 0.5));
        let off = build_quads(&compute_layout((1920, 1080), false), &music(false, 0.5));
        // The first quad is the music button; muted alpha is lower.
        assert!(off[0].alpha < on[0].alpha);
    }
}
