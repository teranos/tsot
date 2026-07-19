//! game/src/tune_hud — in-game panel of quad-buttons for tuning the
//! tree renderer. Rust owns UI (game/CLAUDE.md); this is quads through
//! the existing UI overlay pipeline, no HTML, no new env crossings.
//!
//! Layout: top-left, vertical stack of rows. Each row:
//!
//!   [ - ]  label: value   [ + ]
//!
//! Tapping `-` or `+` nudges the param by its step. Held-tap throttles
//! (one fire per `HOLD_INTERVAL_FRAMES`). Values are formatted as
//! text-quads by `watermark::text_quads`.
//!
//! Wood-shape edits also drop the per-species mesh cache
//! (`tree_surface::invalidate_species_cache`) so the next frame's
//! `species_wood_mesh` regenerates with the new params.

use bevy_ecs::prelude::*;
use std::cell::RefCell;

use crate::dpad::DpadInstance;
use crate::tune::{self, TuneParams};
use crate::watermark::{GLYPH_H, GLYPH_W, text_quads};

// ---------- Field enum: one variant per tunable knob ----------

#[derive(Clone, Copy, Debug)]
enum Field {
    WoodVoxelRatio,
    WoodRfloorCoeff,
    WoodBlendCoeff,
    WoodAabbPad,
    WoodResMin,
    WoodResMax,
    RootAnkleY,
    RootReach,
    RootDepth,
    RootRaMult,
    RootRbMult,
    WindWoodSway,
    WindAmp,
    WindSpeed,
    WindLeafMult,
    LeafDensityMult,
    LeafSizeMult,
    LeafClusterMult,
    AutumnMult,
    DeadwoodMult,
}

impl Field {
    fn all() -> &'static [Field] {
        use Field::*;
        &[
            WoodVoxelRatio, WoodRfloorCoeff, WoodBlendCoeff, WoodAabbPad,
            WoodResMin, WoodResMax,
            RootAnkleY, RootReach, RootDepth, RootRaMult, RootRbMult,
            WindWoodSway, WindAmp, WindSpeed, WindLeafMult,
            LeafDensityMult, LeafSizeMult, LeafClusterMult,
            AutumnMult, DeadwoodMult,
        ]
    }

    fn label(&self) -> &'static str {
        use Field::*;
        match self {
            WoodVoxelRatio => "wood voxel size",
            WoodRfloorCoeff => "wood limb floor",
            WoodBlendCoeff => "wood fork blend",
            WoodAabbPad => "wood bounds pad",
            WoodResMin => "wood res min",
            WoodResMax => "wood res max",
            RootAnkleY => "root ankle up",
            RootReach => "root reach out",
            RootDepth => "root reach down",
            RootRaMult => "root base fat",
            RootRbMult => "root tip fat",
            WindWoodSway => "wood sway",
            WindAmp => "wind amp",
            WindSpeed => "wind speed",
            WindLeafMult => "leaf sway mult",
            LeafDensityMult => "leaves per tip",
            LeafSizeMult => "leaf size",
            LeafClusterMult => "leaf spread",
            AutumnMult => "autumn turn",
            DeadwoodMult => "deadwood odds",
        }
    }

    fn read(&self, t: &TuneParams) -> f32 {
        use Field::*;
        match self {
            WoodVoxelRatio => t.wood_voxel_ratio,
            WoodRfloorCoeff => t.wood_rfloor_coeff,
            WoodBlendCoeff => t.wood_blend_coeff,
            WoodAabbPad => t.wood_aabb_pad,
            WoodResMin => t.wood_res_min as f32,
            WoodResMax => t.wood_res_max as f32,
            RootAnkleY => t.root_ankle_y,
            RootReach => t.root_reach,
            RootDepth => t.root_depth,
            RootRaMult => t.root_ra_mult,
            RootRbMult => t.root_rb_mult,
            WindWoodSway => t.wind_wood_sway,
            WindAmp => t.wind_amp,
            WindSpeed => t.wind_speed,
            WindLeafMult => t.wind_leaf_mult,
            LeafDensityMult => t.leaf_density_mult,
            LeafSizeMult => t.leaf_size_mult,
            LeafClusterMult => t.leaf_cluster_mult,
            AutumnMult => t.autumn_mult,
            DeadwoodMult => t.deadwood_mult,
        }
    }

    fn write(&self, t: &mut TuneParams, v: f32) {
        use Field::*;
        let (lo, hi) = self.range();
        let v = v.clamp(lo, hi);
        match self {
            WoodVoxelRatio => t.wood_voxel_ratio = v,
            WoodRfloorCoeff => t.wood_rfloor_coeff = v,
            WoodBlendCoeff => t.wood_blend_coeff = v,
            WoodAabbPad => t.wood_aabb_pad = v,
            WoodResMin => t.wood_res_min = v as u32,
            WoodResMax => t.wood_res_max = v as u32,
            RootAnkleY => t.root_ankle_y = v,
            RootReach => t.root_reach = v,
            RootDepth => t.root_depth = v,
            RootRaMult => t.root_ra_mult = v,
            RootRbMult => t.root_rb_mult = v,
            WindWoodSway => t.wind_wood_sway = v,
            WindAmp => t.wind_amp = v,
            WindSpeed => t.wind_speed = v,
            WindLeafMult => t.wind_leaf_mult = v,
            LeafDensityMult => t.leaf_density_mult = v,
            LeafSizeMult => t.leaf_size_mult = v,
            LeafClusterMult => t.leaf_cluster_mult = v,
            AutumnMult => t.autumn_mult = v,
            DeadwoodMult => t.deadwood_mult = v,
        }
    }

    fn step(&self) -> f32 {
        use Field::*;
        match self {
            WoodResMin | WoodResMax => 8.0,
            WindAmp => 0.5,
            RootReach | RootDepth => 0.5,
            _ => 0.05,
        }
    }

    fn range(&self) -> (f32, f32) {
        use Field::*;
        match self {
            WoodVoxelRatio => (0.05, 2.0),
            WoodRfloorCoeff => (0.0, 2.0),
            WoodBlendCoeff => (0.0, 2.0),
            WoodAabbPad => (0.5, 6.0),
            WoodResMin => (8.0, 192.0),
            WoodResMax => (16.0, 384.0),
            RootAnkleY => (0.0, 5.0),
            RootReach => (0.0, 20.0),
            RootDepth => (0.0, 20.0),
            RootRaMult => (0.1, 3.0),
            RootRbMult => (0.0, 3.0),
            WindWoodSway => (0.0, 2.0),
            WindAmp => (0.0, 40.0),
            WindSpeed => (0.0, 5.0),
            WindLeafMult => (0.0, 5.0),
            LeafDensityMult => (0.0, 3.0),
            LeafSizeMult => (0.0, 3.0),
            LeafClusterMult => (0.0, 3.0),
            AutumnMult => (0.0, 3.0),
            DeadwoodMult => (0.0, 3.0),
        }
    }

    fn invalidates_wood(&self) -> bool {
        use Field::*;
        matches!(
            self,
            WoodVoxelRatio
                | WoodRfloorCoeff
                | WoodBlendCoeff
                | WoodAabbPad
                | WoodResMin
                | WoodResMax
                | RootAnkleY
                | RootReach
                | RootDepth
                | RootRaMult
                | RootRbMult
        )
    }

    /// Human-readable formatted value. Ints for res; 2dp for floats.
    fn value_string(&self, t: &TuneParams) -> String {
        use Field::*;
        let v = self.read(t);
        match self {
            WoodResMin | WoodResMax => format!("{}", v as u32),
            _ => format!("{v:.2}"),
        }
    }
}

// ---------- Layout ----------

/// One font pixel in CSS px — same idea as watermark. Larger than the
/// watermark's 3 px so the panel is readable.
const PIXEL_PX: f32 = 2.5;
/// Row height in CSS px (glyph height * pixel + padding).
const ROW_H_PX: f32 = 20.0;
/// Panel margin from viewport edges.
const MARGIN_PX: f32 = 12.0;
/// `-` and `+` button size (square).
const BTN_HALF_PX: f32 = 9.0;
/// Space between the `-` button and the label; also label and `+`.
const GAP_PX: f32 = 6.0;
/// Panel background alpha.
const BG_ALPHA: f32 = 0.55;
/// Row background alpha (a very faint stripe).
const ROW_ALPHA: f32 = 0.15;

const LABEL_COLOR: [f32; 3] = [0.90, 0.92, 0.95];
const BTN_COLOR: [f32; 3] = [0.30, 0.35, 0.45];
const BTN_PRESSED_COLOR: [f32; 3] = [0.55, 0.65, 0.80];
const BG_COLOR: [f32; 3] = [0.05, 0.06, 0.09];

/// Throttle: once triggered, a button re-fires every N frames while
/// held. Also the initial-repeat delay is a separate longer wait so
/// tapping once doesn't spam.
const FIRST_REPEAT_FRAMES: u32 = 20;
const REPEAT_INTERVAL_FRAMES: u32 = 4;

#[derive(Copy, Clone, Debug)]
struct ButtonRect {
    center_ndc: [f32; 2],
    half_ndc: [f32; 2],
}

fn label_text(field: Field, params: &TuneParams) -> String {
    format!("{}:{}", field.label(), field.value_string(params))
}

fn ndc_per_px(vp: (u32, u32)) -> (f32, f32) {
    (2.0 / vp.0.max(1) as f32, 2.0 / vp.1.max(1) as f32)
}

/// One row's rects. Returns (`minus`, `plus`) button rects and the
/// origin (`left`, `top`) at which the label text is drawn.
fn row_rects(
    vp: (u32, u32),
    row_index: usize,
    label_len_chars: usize,
) -> (ButtonRect, ButtonRect, f32, f32) {
    let (dx, dy) = ndc_per_px(vp);
    let btn_hx = BTN_HALF_PX * dx;
    let btn_hy = BTN_HALF_PX * dy;
    let gap_x = GAP_PX * dx;
    let row_h = ROW_H_PX * dy;
    let px = PIXEL_PX * dx;
    let py = PIXEL_PX * dy;
    let label_w = text_width_ndc_from_chars(label_len_chars, px);
    // Panel origin: top-left of viewport, inset by MARGIN_PX.
    let panel_left = -1.0 + MARGIN_PX * dx;
    let panel_top = 1.0 - MARGIN_PX * dy;
    let row_top = panel_top - row_index as f32 * row_h;
    let row_center_y = row_top - row_h * 0.5;
    // `-` button at the leftmost column.
    let minus_cx = panel_left + btn_hx;
    // Label starts after the `-` button + a gap.
    let label_left = minus_cx + btn_hx + gap_x;
    // Label text glyph size = 5 rows tall × pixel_ndc_y. Center-vertical.
    let label_top = row_center_y + (GLYPH_H as f32) * py * 0.5;
    // `+` button at label_left + label_w + gap.
    let plus_cx = label_left + label_w + gap_x + btn_hx;
    (
        ButtonRect { center_ndc: [minus_cx, row_center_y], half_ndc: [btn_hx, btn_hy] },
        ButtonRect { center_ndc: [plus_cx, row_center_y], half_ndc: [btn_hx, btn_hy] },
        label_left,
        label_top,
    )
}

fn text_width_ndc_from_chars(n_chars: usize, pixel_ndc_x: f32) -> f32 {
    if n_chars == 0 {
        return 0.0;
    }
    (4 * n_chars - 1) as f32 * pixel_ndc_x
}

// ---------- State + system ----------

/// Bevy resource tracking hold-repeat per button.
#[derive(Resource)]
pub struct TuneHudState {
    /// Frames each (row, is_plus) button has been held. 0 = not held.
    /// Layout: `[row * 2 + is_plus_as_usize]`.
    held_frames: [u32; 64],
}

impl Default for TuneHudState {
    fn default() -> Self {
        // Std only impls Default for arrays up to length 32.
        Self { held_frames: [0u32; 64] }
    }
}

pub fn setup_tune_hud(mut commands: Commands) {
    commands.insert_resource(TuneHudState::default());
}

thread_local! {
    static INSTANCES: RefCell<Vec<DpadInstance>> = const { RefCell::new(Vec::new()) };
}

/// The rendering-side callback used by the main UI pass — every frame
/// after `tune_hud_system` runs.
pub fn current_instances() -> Vec<DpadInstance> {
    INSTANCES.with(|c| c.borrow().clone())
}

fn point_in_rect(p: [f32; 2], r: ButtonRect) -> bool {
    (p[0] - r.center_ndc[0]).abs() <= r.half_ndc[0]
        && (p[1] - r.center_ndc[1]).abs() <= r.half_ndc[1]
}

fn should_fire(prev_held: u32, next_held: u32) -> bool {
    // Edge — first frame held.
    if prev_held == 0 && next_held >= 1 {
        return true;
    }
    // Hold repeat — after the initial delay, every REPEAT_INTERVAL.
    if next_held > FIRST_REPEAT_FRAMES {
        let elapsed_since_delay = next_held - FIRST_REPEAT_FRAMES;
        return elapsed_since_delay.is_multiple_of(REPEAT_INTERVAL_FRAMES);
    }
    false
}

pub fn tune_hud_system(mut state: ResMut<TuneHudState>) {
    let vp = crate::gpu_web::viewport_size();
    if vp.0 == 0 || vp.1 == 0 {
        return;
    }
    let touches = crate::gpu_web::touches();
    let pointer = crate::input::pointer_ndc();
    let wheel_delta = crate::input::wheel_delta();
    let fields = Field::all();
    let mut params = tune::get();
    let mut wood_dirty = false;
    let mut out = Vec::with_capacity(fields.len() * (2 + 24));

    let (dx, dy) = ndc_per_px(vp);
    let px = PIXEL_PX * dx;
    let py = PIXEL_PX * dy;
    let row_h = ROW_H_PX * dy;
    let btn_hx = BTN_HALF_PX * dx;

    // Panel background: sized to enclose all rows and the widest label.
    // Width heuristic — pick the widest label text and add button + gaps.
    let widest_chars = fields
        .iter()
        .map(|f| label_text(*f, &params).chars().count())
        .max()
        .unwrap_or(0);
    let (_m, _p, _l, _lt) = row_rects(vp, 0, widest_chars);
    let panel_left = -1.0 + MARGIN_PX * dx;
    let panel_right =
        panel_left + btn_hx * 2.0 + GAP_PX * dx + text_width_ndc_from_chars(widest_chars, px)
            + GAP_PX * dx + btn_hx * 2.0;
    let panel_top = 1.0 - MARGIN_PX * dy;
    let panel_bottom = panel_top - fields.len() as f32 * row_h;
    let bg_cx = (panel_left + panel_right) * 0.5;
    let bg_cy = (panel_top + panel_bottom) * 0.5;
    out.push(DpadInstance {
        center_ndc: [bg_cx, bg_cy],
        half_size_ndc: [(panel_right - panel_left) * 0.5, (panel_top - panel_bottom) * 0.5],
        color: BG_COLOR,
        alpha: BG_ALPHA,
    });

    // Which row does the pointer sit over, if any? Pointer arrives in
    // NDC; only y-bucketing matters (rows are stacked vertically), and
    // we also gate on x so hovering outside the panel doesn't count.
    let hovered_row = pointer.and_then(|p| {
        if p[0] < panel_left || p[0] > panel_right {
            return None;
        }
        for row_index in 0..fields.len() {
            let row_top_ndc = panel_top - row_index as f32 * row_h;
            let row_bot_ndc = row_top_ndc - row_h;
            if p[1] <= row_top_ndc && p[1] >= row_bot_ndc {
                return Some(row_index);
            }
        }
        None
    });

    // Wheel adjusts the hovered row's field. Sensitivity: one notch =
    // one step. Ignored if pointer isn't over a row (wheel elsewhere
    // is free to be consumed by future subsystems).
    if wheel_delta != 0
        && let Some(row_index) = hovered_row
    {
        let field = fields[row_index];
        let step = field.step();
        let v = field.read(&params) + (wheel_delta as f32) * step;
        field.write(&mut params, v);
        if field.invalidates_wood() {
            wood_dirty = true;
        }
    }

    for (row_index, field) in fields.iter().enumerate() {
        let label = label_text(*field, &params);
        let (minus, plus, label_left, label_top) = row_rects(vp, row_index, label.chars().count());

        // Hit-test against every touch.
        let minus_pressed = touches.iter().any(|t| point_in_rect(*t, minus));
        let plus_pressed = touches.iter().any(|t| point_in_rect(*t, plus));

        // Held-frame accounting.
        let idx_m = row_index * 2;
        let idx_p = row_index * 2 + 1;
        let prev_m = state.held_frames[idx_m];
        let prev_p = state.held_frames[idx_p];
        let next_m = if minus_pressed { prev_m + 1 } else { 0 };
        let next_p = if plus_pressed { prev_p + 1 } else { 0 };
        state.held_frames[idx_m] = next_m;
        state.held_frames[idx_p] = next_p;

        if should_fire(prev_m, next_m) {
            let step = field.step();
            let v = field.read(&params) - step;
            field.write(&mut params, v);
            if field.invalidates_wood() {
                wood_dirty = true;
            }
        }
        if should_fire(prev_p, next_p) {
            let step = field.step();
            let v = field.read(&params) + step;
            field.write(&mut params, v);
            if field.invalidates_wood() {
                wood_dirty = true;
            }
        }

        // Row stripe. When the pointer hovers this row, brighter tint
        // and higher alpha so the "wheel target" is unambiguous.
        let is_hovered = hovered_row == Some(row_index);
        out.push(DpadInstance {
            center_ndc: [bg_cx, minus.center_ndc[1]],
            half_size_ndc: [(panel_right - panel_left) * 0.5, row_h * 0.45],
            color: if is_hovered { [0.20, 0.35, 0.55] } else { BG_COLOR },
            alpha: if is_hovered { 0.45 } else { ROW_ALPHA },
        });

        // `-` button.
        out.push(DpadInstance {
            center_ndc: minus.center_ndc,
            half_size_ndc: minus.half_ndc,
            color: if minus_pressed { BTN_PRESSED_COLOR } else { BTN_COLOR },
            alpha: 0.85,
        });
        // `+` button.
        out.push(DpadInstance {
            center_ndc: plus.center_ndc,
            half_size_ndc: plus.half_ndc,
            color: if plus_pressed { BTN_PRESSED_COLOR } else { BTN_COLOR },
            alpha: 0.85,
        });

        // Label + value text.
        let re_label = label_text(*field, &params); // reflect just-adjusted value
        out.extend(text_quads(&re_label, label_left, label_top, [px, py], LABEL_COLOR, 0.95));

        // "-" glyph inside minus button, "+" glyph inside plus button.
        // Centered: place the top of a single glyph so its centre sits
        // at button centre. Glyph is 3 px wide × 5 px tall.
        let glyph_top = |cy: f32| cy + (GLYPH_H as f32) * py * 0.5;
        let glyph_left = |cx: f32| cx - (GLYPH_W as f32) * px * 0.5;
        out.extend(text_quads(
            "-",
            glyph_left(minus.center_ndc[0]),
            glyph_top(minus.center_ndc[1]),
            [px, py],
            LABEL_COLOR,
            0.95,
        ));
        out.extend(text_quads(
            "+",
            glyph_left(plus.center_ndc[0]),
            glyph_top(plus.center_ndc[1]),
            [px, py],
            LABEL_COLOR,
            0.95,
        ));
    }

    if params != tune::get() {
        tune::set(params);
        if wood_dirty {
            crate::tree_surface::invalidate_species_cache();
        }
    }

    INSTANCES.with(|c| *c.borrow_mut() = out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_reads_writes_within_range() {
        // Round-trip every field through write/read at midpoint; the
        // stored value comes back unchanged (or clamped identically).
        for f in Field::all() {
            let (lo, hi) = f.range();
            let mid = (lo + hi) * 0.5;
            let mut p = TuneParams::defaults();
            f.write(&mut p, mid);
            let got = f.read(&p);
            let expected = match f {
                Field::WoodResMin | Field::WoodResMax => (mid as u32) as f32,
                _ => mid,
            };
            let tol = (hi - lo) * 0.01;
            assert!(
                (got - expected).abs() <= tol,
                "field {:?} round-trip: wrote {mid}, read {got} (expected {expected})",
                f
            );
        }
    }

    #[test]
    fn wood_shape_fields_invalidate_cache() {
        // Every field named `wood_*` or `root_*` must signal wood-cache
        // invalidation — else regeneration would silently skip and the
        // slider would look broken.
        for f in Field::all() {
            let name = format!("{:?}", f);
            if name.starts_with("Wood") || name.starts_with("Root") {
                assert!(f.invalidates_wood(), "{name} should invalidate wood cache");
            } else {
                assert!(!f.invalidates_wood(), "{name} should NOT invalidate wood cache");
            }
        }
    }

    #[test]
    fn should_fire_edge_and_hold_repeat() {
        // Edge trigger on first press.
        assert!(should_fire(0, 1));
        // Held for less than initial delay: no fire.
        for held in 2..=FIRST_REPEAT_FRAMES {
            assert!(!should_fire(held - 1, held), "held={held}");
        }
        // After initial delay: fire every REPEAT_INTERVAL_FRAMES.
        let after = FIRST_REPEAT_FRAMES + REPEAT_INTERVAL_FRAMES;
        assert!(should_fire(after - 1, after));
    }
}
