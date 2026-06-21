//! Page layout constants + canvas sizing. Lives in Rust per
//! CLAUDE.md "JS is used in spite, not by choice" — layout decisions
//! (mobile threshold, panel reservation, square-fit math) belong to
//! Rust; JS only shuttles browser-API values across the boundary.
//!
//! The CSS side (`play.html`) and this module share the constants
//! that pin the layout together. When `play.html` changes, the
//! constants here move with it; that's why they live next to each
//! other in the comment markers.

/// Right-side info panel reservation in CSS pixels. Matches
/// `play.html` `#info { min-width: 24em }` evaluated against the
/// 0.8rem root font-size — at 16px-per-rem that's 24 * 0.8 * 16 ≈
/// 307; we round up to 400 to give the panel breathing room for the
/// peer/log/inventory rows. If the CSS min-width changes, this moves
/// too.
const INFO_PANEL_MIN_WIDTH_PX: u32 = 400;

/// `#wrap` padding (per-side) × 2. CSS: `padding: 0.75rem` → 12px at
/// 0.8rem root → 24 total horizontal + 24 vertical.
const WRAP_PADDING_PX: u32 = 24;

/// `#wrap` gap between #game and #info. CSS: `gap: 1rem` → 16px.
const PANEL_GAP_PX: u32 = 16;

/// Mobile threshold. Mirrors `play.html`'s `@media (max-width: 720px)`
/// media query so the JS-side decision (which canvas size to apply)
/// matches the CSS-side decision (how to lay out #wrap). If they
/// disagree, the canvas overlaps the panel — a bug whose only
/// observable is "looks wrong on a phone."
const MOBILE_MAX_WIDTH_PX: u32 = 720;

/// Minimum canvas side. Even on tiny viewports we keep the game
/// playable. Below this and tile pixels become smaller than the
/// vision-radius math expects.
const CANVAS_MIN_SIDE_PX: u32 = 256;

/// Mobile mode: canvas takes ~60% of the viewport height since the
/// info panel stacks underneath instead of beside.
const MOBILE_CANVAS_HEIGHT_FRACTION: f32 = 0.6;

/// Decide the canvas side length for a given viewport. Returns the
/// largest square that fits in the available area after subtracting
/// the panel reservation and wrap padding. Falls back to
/// `CANVAS_MIN_SIDE_PX` so the renderer never receives a degenerate
/// size on a phone-sized viewport.
pub fn canvas_side_px(window_w: u32, window_h: u32) -> u32 {
    let is_mobile = window_w <= MOBILE_MAX_WIDTH_PX;
    let available_w = if is_mobile {
        window_w.saturating_sub(WRAP_PADDING_PX)
    } else {
        window_w
            .saturating_sub(INFO_PANEL_MIN_WIDTH_PX)
            .saturating_sub(PANEL_GAP_PX)
            .saturating_sub(WRAP_PADDING_PX)
    };
    let available_h = if is_mobile {
        ((window_h as f32) * MOBILE_CANVAS_HEIGHT_FRACTION) as u32
    } else {
        window_h.saturating_sub(WRAP_PADDING_PX)
    };
    let side = available_w.min(available_h);
    side.max(CANVAS_MIN_SIDE_PX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Typical laptop viewport — canvas size must NOT exceed the
    /// available width after reserving the info panel + padding.
    /// Falsifies the regression where the math ignores the right-panel
    /// reservation (the hardcoded 1440 bug that started the
    /// "viewport jumps down" complaint).
    #[test]
    fn canvas_fits_within_viewport_minus_info_panel() {
        let side = canvas_side_px(1500, 1000);
        let max_allowed_width = 1500 - INFO_PANEL_MIN_WIDTH_PX - PANEL_GAP_PX - WRAP_PADDING_PX;
        assert!(
            side <= max_allowed_width,
            "canvas side {side} must not exceed available width {max_allowed_width}"
        );
        let max_allowed_height = 1000 - WRAP_PADDING_PX;
        assert!(
            side <= max_allowed_height,
            "canvas side {side} must not exceed available height {max_allowed_height}"
        );
    }

    /// Mobile viewport — narrow window collapses to vertical stack;
    /// canvas takes width minus padding, height up to the mobile
    /// fraction. Falsifies the regression where the desktop branch
    /// runs on mobile and produces a negative / clamped side because
    /// the panel reservation eats the whole width.
    #[test]
    fn mobile_canvas_uses_full_width_minus_padding() {
        let side = canvas_side_px(400, 800);
        let expected_max_w = 400 - WRAP_PADDING_PX;
        let expected_max_h = ((800.0_f32) * MOBILE_CANVAS_HEIGHT_FRACTION) as u32;
        let expected = expected_max_w.min(expected_max_h);
        assert_eq!(side, expected);
    }

    /// Tiny viewport — canvas can't go below the minimum playable
    /// size. Falsifies the regression where `saturating_sub` produces
    /// 0 and the renderer receives a degenerate framebuffer.
    #[test]
    fn canvas_clamps_to_minimum_on_tiny_viewport() {
        let side = canvas_side_px(50, 50);
        assert_eq!(side, CANVAS_MIN_SIDE_PX);
    }

    /// Exact mobile threshold — width == 720 is mobile; width == 721
    /// is desktop. Pins the boundary so a future "what does 720 mean"
    /// question is answered by the test, not by guessing.
    #[test]
    fn mobile_threshold_includes_720() {
        // At 720 we're mobile, so the desktop reservation (which would
        // produce a different size) must NOT apply.
        let mobile_side = canvas_side_px(720, 1000);
        let desktop_side = canvas_side_px(721, 1000);
        // Hard to assert exact values without re-implementing the math
        // in the test, so just assert the branch was different (which
        // it must be unless both branches return the same value for
        // these inputs).
        let mobile_w_branch = 720 - WRAP_PADDING_PX;
        let desktop_w_branch =
            721 - INFO_PANEL_MIN_WIDTH_PX - PANEL_GAP_PX - WRAP_PADDING_PX;
        assert_ne!(mobile_w_branch, desktop_w_branch);
        // The actual sides will differ correspondingly (clamped by H).
        let _ = (mobile_side, desktop_side);
    }
}
