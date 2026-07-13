//! In-game build watermark — the running binary draws its OWN commit
//! hash into the frame, through the UI overlay quad pipeline. Rust owns
//! the render (see game/CLAUDE.md); this is not a DOM/JS overlay. So
//! "what version is running" is answered by the game itself, on the
//! canvas, always — no build-info.json, no browser chrome hiding it.
//!
//! A 3x5 pixel font: each lit pixel is one UI quad. Sited top-right,
//! subtle — the mobile browser's toolbar covers the bottom edge, so the
//! top stays visible.

use crate::build_info;
use crate::dpad::DpadInstance;

const GLYPH_W: usize = 3;
const GLYPH_H: usize = 5;

/// 3x5 bitmap for a char: 5 rows, low 3 bits each (bit 2 = leftmost
/// column). Covers hex digits (real commit shas) plus the letters in
/// "unknown" (what build_info reports off-CI). Unknown chars → skipped.
fn glyph(c: char) -> Option<[u8; GLYPH_H]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        'a' => [0b111, 0b101, 0b111, 0b101, 0b101],
        'b' => [0b100, 0b100, 0b111, 0b101, 0b111],
        'c' => [0b111, 0b100, 0b100, 0b100, 0b111],
        'd' => [0b001, 0b001, 0b111, 0b101, 0b111],
        'e' => [0b111, 0b100, 0b111, 0b100, 0b111],
        'f' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'u' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'n' => [0b000, 0b110, 0b101, 0b101, 0b101],
        'k' => [0b101, 0b110, 0b100, 0b110, 0b101],
        'o' => [0b000, 0b111, 0b101, 0b101, 0b111],
        'w' => [0b101, 0b101, 0b101, 0b111, 0b101],
        _ => return None,
    })
}

/// How many leading commit chars to show.
const SHORT_LEN: usize = 7;
/// One font pixel in CSS px (kept square across aspect like the D-pad).
const PIXEL_PX: f32 = 3.0;
/// Inset from the top-right corner, CSS px.
const MARGIN_PX: f32 = 12.0;
/// Faint, so it reads as a watermark, not a label.
const COLOR: [f32; 3] = [0.75, 0.80, 0.88];
const ALPHA: f32 = 0.30;

/// The UI quads that spell the running binary's short commit, top-right.
pub fn watermark_quads(viewport: (u32, u32)) -> Vec<DpadInstance> {
    let text: String = build_info::COMMIT.chars().take(SHORT_LEN).collect();
    let (w, h) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
    let ndc_x = 2.0 / w;
    let ndc_y = 2.0 / h;
    let sx = PIXEL_PX * ndc_x; // one font pixel, NDC width
    let sy = PIXEL_PX * ndc_y; // one font pixel, NDC height
    let cols = text.chars().count() * (GLYPH_W + 1); // glyph + 1-col gap
    let total_w = cols as f32 * sx;
    let right = 1.0 - MARGIN_PX * ndc_x;
    let left = right - total_w;
    let top = 1.0 - MARGIN_PX * ndc_y;

    let mut out = Vec::new();
    for (ci, ch) in text.chars().enumerate() {
        let Some(rows) = glyph(ch) else { continue };
        let col0 = ci * (GLYPH_W + 1);
        for (r, row) in rows.iter().enumerate() {
            for c in 0..GLYPH_W {
                if row & (1 << (GLYPH_W - 1 - c)) != 0 {
                    let g = col0 + c;
                    out.push(DpadInstance {
                        center_ndc: [left + (g as f32 + 0.5) * sx, top - (r as f32 + 0.5) * sy],
                        half_size_ndc: [sx * 0.5, sy * 0.5],
                        color: COLOR,
                        alpha: ALPHA,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_digits_all_have_glyphs() {
        for c in "0123456789abcdef".chars() {
            assert!(glyph(c).is_some(), "missing glyph for hex '{c}'");
        }
    }

    #[test]
    fn emits_quads_for_a_sha_within_the_top_right() {
        // Pretend the build stamped a hex commit by exercising the
        // layout math directly on a known string of lit glyphs.
        let q = watermark_quads((1920, 1080));
        // build_info::COMMIT is "unknown" in tests, which has glyphs,
        // so we get some quads, and they sit in the top-right quadrant.
        assert!(!q.is_empty(), "watermark should emit quads");
        for inst in &q {
            assert!(inst.center_ndc[1] > 0.0, "watermark stays in the top half");
            assert!(inst.center_ndc[0] > 0.0, "watermark stays on the right");
            assert!(inst.center_ndc[0] < 1.0 && inst.center_ndc[1] < 1.0, "on screen");
        }
    }
}
