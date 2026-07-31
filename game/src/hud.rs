//! Bevy-owned heads-up controls, drawn through the same UI-overlay
//! quad pipeline as the D-pad (see `dpad.rs`). Rust owns the render —
//! no HTML/CSS overlay — so these are just more `DpadInstance` quads
//! appended after the D-pad each frame.
//!
//! Controls:
//!   - Settings gear (top-left): tap to open/close the settings panel.
//!     The only thing this module puts on the world view.
//!   - Settings panel: music mute toggle, music + SFX volume sliders,
//!     and a button opening the tree-tuning submenu (`tune_hud`).
//!
//! The music toggle used to sit in the bottom-left corner of the world
//! view. It moved inside the panel: settings belong in settings, and
//! that corner is the action bar's.
//!
//! Interaction is hit-tested against `gpu_web::touches()`. Taps are
//! rising-edge per-control (so a thumb resting on the D-pad doesn't
//! swallow a tap on a HUD button); the slider is a continuous drag.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::dpad::DpadInstance;
use crate::music::Music;
use crate::sfx::SfxMix;

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
/// plus, when open, the settings panel and its two slider tracks
/// (music on top, SFX below).
pub struct HudLayout {
    pub gear: Rect,
    pub panel_open: bool,
    pub panel: Rect,
    /// Mute/unmute, inside the panel.
    pub music_toggle: Rect,
    pub music_slider: Rect,
    pub sfx_slider: Rect,
    /// Opens the tree-tuning submenu.
    pub trees_button: Rect,
}

/// Compute the HUD geometry for a viewport. Buttons are pixel-sized and
/// pinned to corners with a safe margin (mirrors the D-pad); the panel
/// is a centred dialog sized in NDC fractions with two sliders stacked
/// (music on top, SFX below).
pub fn compute_layout(viewport: (u32, u32), panel_open: bool) -> HudLayout {
    let (w, h) = viewport;
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let ndc_x = 2.0 / w;
    let ndc_y = 2.0 / h;
    let half_x = BTN_HALF_PX * ndc_x;
    let half_y = BTN_HALF_PX * ndc_y;
    let inset_x = (MARGIN_PX + BTN_HALF_PX) * ndc_x;
    let inset_y = (MARGIN_PX + BTN_HALF_PX) * ndc_y;
    let gear = Rect {
        cx: -1.0 + inset_x,
        cy: 1.0 - inset_y,
        hx: half_x,
        hy: half_y,
    };
    // Centred modal: mute toggle, two slider tracks, then the submenu
    // button along the bottom.
    let panel = Rect { cx: 0.0, cy: 0.0, hx: 0.42, hy: 0.38 };
    // Laid out FROM the panel's bounds, not from magic numbers. The
    // toggle is pixel-sized so it stays square across aspect, which
    // makes it wide in NDC on a narrow phone — placing it by hand put
    // it outside the panel on a 390px screen.
    let pad_x = 0.03;
    let gap_x = 0.02;
    // A panel control can never be wider than half the panel.
    let toggle_hx = half_x.min(panel.hx * 0.25);
    let toggle_hy = half_y.min(panel.hy * 0.25);
    let toggle_cx = panel.cx - panel.hx + pad_x + toggle_hx;
    let music_toggle = Rect { cx: toggle_cx, cy: 0.20, hx: toggle_hx, hy: toggle_hy };
    // The music slider takes what is left of the row; the SFX slider
    // matches it so the two read as a pair.
    let slider_left = toggle_cx + toggle_hx + gap_x;
    let slider_right = panel.cx + panel.hx - pad_x;
    let slider_hx = (slider_right - slider_left) * 0.5;
    let slider_cx = slider_left + slider_hx;
    let music_slider = Rect { cx: slider_cx, cy: 0.20, hx: slider_hx, hy: 0.028 };
    let sfx_slider = Rect { cx: slider_cx, cy: 0.04, hx: slider_hx, hy: 0.028 };
    let trees_button = Rect { cx: panel.cx, cy: -0.20, hx: 0.24, hy: 0.055 };
    HudLayout {
        gear,
        panel_open,
        panel,
        music_toggle,
        music_slider,
        sfx_slider,
        trees_button,
    }
}

/// Map a touch x-coordinate to a volume in [0,1] across the slider
/// track's width.
pub fn volume_from_x(slider: &Rect, x: f32) -> f32 {
    let left = slider.cx - slider.hx;
    ((x - left) / (2.0 * slider.hx)).clamp(0.0, 1.0)
}

/// HUD state carried between frames: whether the panel is open, the
/// previous-frame coverage of each tap button for rising-edge taps,
/// and the previous ESC-key state so tapping ESC once closes the
/// settings panel.
#[derive(Resource, Default)]
pub struct Hud {
    pub panel_open: bool,
    /// Whether the tree-tuning submenu is showing. Off by default: it
    /// is a developer panel and it covered a third of a phone screen.
    pub tune_open: bool,
    pub prev_music: bool,
    pub prev_gear: bool,
    pub prev_trees: bool,
    pub prev_esc: bool,
}

pub fn setup_hud(mut commands: Commands) {
    commands.insert_resource(Hud::default());
}

/// Purple fill on the music track — same tint as the music toggle so
/// the eye reads them as one channel.
const MUSIC_FILL: [f32; 3] = [0.45, 0.35, 0.75];
/// Teal fill on the SFX track — distinct from the music purple.
const SFX_FILL: [f32; 3] = [0.30, 0.65, 0.55];

fn push_slider(
    out: &mut Vec<DpadInstance>,
    slider: &Rect,
    level: f32,
    fill_color: [f32; 3],
) {
    // Track.
    out.push(DpadInstance {
        center_ndc: [slider.cx, slider.cy],
        half_size_ndc: [slider.hx, slider.hy],
        color: [0.20, 0.20, 0.26],
        alpha: 1.0,
    });
    // Filled portion, left edge → knob, showing the current level.
    let left = slider.cx - slider.hx;
    let knob_x = left + level * 2.0 * slider.hx;
    let fill_hx = ((knob_x - left) * 0.5).max(0.0);
    out.push(DpadInstance {
        center_ndc: [left + fill_hx, slider.cy],
        half_size_ndc: [fill_hx, slider.hy],
        color: fill_color,
        alpha: 1.0,
    });
    // Knob — square-ish grip, slightly taller than the track.
    let knob_hx = slider.hy;
    out.push(DpadInstance {
        center_ndc: [knob_x, slider.cy],
        half_size_ndc: [knob_hx, slider.hy * 1.8],
        color: [0.88, 0.88, 0.94],
        alpha: 1.0,
    });
}

/// Build the overlay quads for the current layout + audio state. When
/// the panel is closed that's just the two corner buttons; open, it's
/// the backdrop plus a music slider (purple) and an SFX slider (teal).
pub fn build_quads(
    layout: &HudLayout,
    music: &Music,
    sfx: &SfxMix,
    tune_open: bool,
    viewport: (u32, u32),
) -> Vec<DpadInstance> {
    let mut out = Vec::with_capacity(16);
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
        // Music mute toggle, now a panel control. Purple when playing,
        // dim when muted.
        let (mc, ma) = if music.playing {
            ([0.55, 0.35, 0.85], 0.9)
        } else {
            ([0.28, 0.24, 0.32], 0.7)
        };
        out.push(DpadInstance {
            center_ndc: [layout.music_toggle.cx, layout.music_toggle.cy],
            half_size_ndc: [layout.music_toggle.hx, layout.music_toggle.hy],
            color: mc,
            alpha: ma,
        });
        push_slider(&mut out, &layout.music_slider, music.volume, MUSIC_FILL);
        push_slider(&mut out, &layout.sfx_slider, sfx.volume, SFX_FILL);
        // Submenu button: the tree-tuning panel. A developer control,
        // so it lives one level in rather than over the world.
        out.push(DpadInstance {
            center_ndc: [layout.trees_button.cx, layout.trees_button.cy],
            half_size_ndc: [layout.trees_button.hx, layout.trees_button.hy],
            color: if tune_open { [0.30, 0.42, 0.34] } else { [0.22, 0.23, 0.28] },
            alpha: 0.95,
        });
        let (vw, vh) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
        let (sx, sy) = (3.0 * 2.0 / vw, 3.0 * 2.0 / vh);
        let label = "trees";
        let width = label.chars().count() as f32 * 4.0 * sx;
        out.extend(crate::watermark::text_quads(
            label,
            layout.trees_button.cx - width * 0.5,
            layout.trees_button.cy + 3.0 * sy,
            [sx, sy],
            [0.85, 0.92, 0.86],
            1.0,
        ));
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

/// Poll touches + keys, drive the toggles + sliders, publish the
/// render quads. Both `Music` and `SfxMix` are `Option<ResMut>` so
/// this system runs regardless of setup ordering.
pub fn hud_input_system(
    mut hud: ResMut<Hud>,
    music: Option<ResMut<Music>>,
    sfx: Option<ResMut<SfxMix>>,
) {
    let (Some(mut music), Some(mut sfx)) = (music, sfx) else {
        return;
    };
    let viewport = crate::gpu_web::viewport_size();
    let layout = compute_layout(viewport, hud.panel_open);
    let touches = crate::gpu_web::touches();

    // Rising-edge taps, per control.
    // The music toggle only exists while the panel is open.
    let music_cov =
        hud.panel_open && touches.iter().any(|&p| layout.music_toggle.contains(p));
    let gear_cov = touches.iter().any(|&p| layout.gear.contains(p));
    let trees_cov =
        hud.panel_open && touches.iter().any(|&p| layout.trees_button.contains(p));
    if music_cov && !hud.prev_music {
        music.toggle();
    }
    if gear_cov && !hud.prev_gear {
        hud.panel_open = !hud.panel_open;
    }
    if trees_cov && !hud.prev_trees {
        hud.tune_open = !hud.tune_open;
    }
    hud.prev_music = music_cov;
    hud.prev_gear = gear_cov;
    hud.prev_trees = trees_cov;

    // ESC (rising edge) closes the settings panel. Reads the keyboard
    // bit exposed by the input shim; the touch path can't send ESC, so
    // this is desktop-only for now.
    let esc = (crate::input::state() & crate::input::key::ESC) != 0;
    if esc && !hud.prev_esc {
        // Innermost first: ESC closes the submenu before the panel.
        if hud.tune_open {
            hud.tune_open = false;
        } else if hud.panel_open {
            hud.panel_open = false;
        }
    }
    hud.prev_esc = esc;

    // Continuous slider drag while the panel is open — one channel
    // per slider so a thumb on one doesn't drag the other.
    if hud.panel_open {
        for &p in &touches {
            if layout.music_slider.contains(p) {
                music.set_volume(volume_from_x(&layout.music_slider, p[0]));
            } else if layout.sfx_slider.contains(p) {
                sfx.set_volume(volume_from_x(&layout.sfx_slider, p[0]));
            }
        }
    }

    // Render quads reflect the (possibly just-toggled) panel state.
    let render_layout = compute_layout(viewport, hud.panel_open);
    let quads = build_quads(&render_layout, &music, &sfx, hud.tune_open, viewport);
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

    fn sfx(volume: f32) -> SfxMix {
        SfxMix { volume }
    }

    #[test]
    fn the_gear_stays_on_screen_on_portrait() {
        let l = compute_layout((390, 844), false);
        let r = l.gear;
        assert!(r.cx - r.hx > -1.0, "gear left off-screen");
        assert!(r.cx + r.hx < 1.0, "gear right off-screen");
        assert!(r.cy - r.hy > -1.0, "gear bottom off-screen");
        assert!(r.cy + r.hy < 1.0, "gear top off-screen");
        assert!(l.gear.cy > 0.0, "the gear is the top-left control");
    }

    #[test]
    fn the_gear_is_the_only_thing_over_the_world() {
        // The music toggle used to sit bottom-left, which is where the
        // action bar now lives — two controls in one corner, both
        // tappable. Settings belong in settings.
        let l = compute_layout((390, 844), false);
        let bar = crate::actions::slot_rects((390, 844));
        for (i, b) in bar.iter().enumerate() {
            let overlap_x = (l.gear.cx - b.cx).abs() < (l.gear.hx + b.hx);
            let overlap_y = (l.gear.cy - b.cy).abs() < (l.gear.hy + b.hy);
            assert!(
                !(overlap_x && overlap_y),
                "the gear overlaps action slot {i}"
            );
        }
    }

    #[test]
    fn panel_controls_sit_inside_the_panel() {
        let l = compute_layout((390, 844), true);
        for (name, r) in [
            ("music toggle", l.music_toggle),
            ("music slider", l.music_slider),
            ("sfx slider", l.sfx_slider),
            ("trees button", l.trees_button),
        ] {
            assert!(
                (r.cx - l.panel.cx).abs() + r.hx <= l.panel.hx + 1e-6,
                "{name} escapes the panel horizontally"
            );
            assert!(
                (r.cy - l.panel.cy).abs() + r.hy <= l.panel.hy + 1e-6,
                "{name} escapes the panel vertically"
            );
        }
    }

    #[test]
    fn the_music_toggle_does_not_sit_on_its_own_slider() {
        let l = compute_layout((390, 844), true);
        let t = l.music_toggle;
        let sl = l.music_slider;
        assert!(
            (t.cx - sl.cx).abs() >= t.hx + sl.hx,
            "the mute button overlaps the volume slider"
        );
    }

    #[test]
    fn sliders_map_x_across_their_own_tracks() {
        let l = compute_layout((1920, 1080), true);
        for s in [l.music_slider, l.sfx_slider] {
            assert!((volume_from_x(&s, s.cx - s.hx) - 0.0).abs() < 1e-6, "left = 0");
            assert!((volume_from_x(&s, s.cx + s.hx) - 1.0).abs() < 1e-6, "right = 1");
            assert!((volume_from_x(&s, s.cx) - 0.5).abs() < 1e-6, "centre = 0.5");
            assert_eq!(volume_from_x(&s, -2.0), 0.0);
            assert_eq!(volume_from_x(&s, 2.0), 1.0);
        }
        // Sliders don't overlap — music is above SFX.
        assert!(l.music_slider.cy > l.sfx_slider.cy);
        let music_bot = l.music_slider.cy - l.music_slider.hy;
        let sfx_top = l.sfx_slider.cy + l.sfx_slider.hy;
        assert!(
            music_bot > sfx_top,
            "music slider bottom {music_bot} must sit above SFX slider top {sfx_top}"
        );
    }

    #[test]
    fn only_the_gear_is_drawn_over_the_world() {
        let closed =
            build_quads(
                &compute_layout((1920, 1080), false),
                &music(true, 0.5),
                &sfx(0.5),
                false,
                (1920, 1080),
            );
        let open =
            build_quads(
                &compute_layout((1920, 1080), true),
                &music(true, 0.5),
                &sfx(0.5),
                false,
                (1920, 1080),
            );
        // Closed, the gear is the only control. Open adds the backdrop,
        // the mute toggle, two sliders at 3 quads each, the submenu
        // button and its label.
        assert_eq!(closed.len(), 1, "something other than the gear is over the world");
        assert!(open.len() > closed.len() + 8);
    }

    #[test]
    fn muted_music_toggle_reads_dim() {
        // The toggle only exists while the panel is open now.
        let l = compute_layout((1920, 1080), true);
        let find = |playing: bool| {
            build_quads(&l, &music(playing, 0.5), &sfx(0.5), false, (1920, 1080))
                .into_iter()
                .find(|q| {
                    (q.center_ndc[0] - l.music_toggle.cx).abs() < 1e-6
                        && (q.center_ndc[1] - l.music_toggle.cy).abs() < 1e-6
                })
                .expect("the mute toggle is drawn while the panel is open")
        };
        assert!(find(false).alpha < find(true).alpha, "muted should read dimmer");
    }

    #[test]
    fn the_trees_submenu_button_reads_active_when_open() {
        let l = compute_layout((1920, 1080), true);
        let colour_of = |tune_open: bool| {
            build_quads(&l, &music(true, 0.5), &sfx(0.5), tune_open, (1920, 1080))
                .into_iter()
                .find(|q| {
                    (q.center_ndc[0] - l.trees_button.cx).abs() < 1e-6
                        && (q.center_ndc[1] - l.trees_button.cy).abs() < 1e-6
                })
                .expect("the submenu button is drawn")
                .color
        };
        assert_ne!(
            colour_of(true),
            colour_of(false),
            "the submenu button looks the same open and closed"
        );
    }
}
