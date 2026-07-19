//! Interim visual proof of `terrain::height` — a top-down hillshade of
//! the heightfield, CPU-rendered to a PNG. NOT the merge-bar render
//! (that's the draped dev-grid in the game render, Slice 4/5); this is a
//! diagnostic so the field is visible at every slice.
//!
//! Centres on the real school stamp and outlines its footprint, so the
//! Slice 2 flat pad reads as a uniform plateau inside the outline with
//! relief only outside it.
//!
//!   cargo +nightly run --example heightfield_preview
//!   OUT=/path/foo.png SPAN=14400 cargo +nightly run --example heightfield_preview

use game::chunk::CHUNK_SIZE;
use game::terrain::height;

fn main() {
    // Locate the school (largest authored footprint) the way the tour does.
    let (bt, _failures) = cdda::load_building_templates();
    let num = bt.templates.len();
    let school = bt
        .half_extents
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .expect("no templates");
    let pad_half = cdda::BUILDING_FOOTPRINT_HALF.max(bt.half_extents[school]);
    let anchor = (1..400i32)
        .find_map(|r| {
            let mut hit = None;
            for x in -r..=r {
                for z in -r..=r {
                    if x.abs() != r && z.abs() != r {
                        continue;
                    }
                    if cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE).is_some()
                        && cdda::building_index(x, z, num) == school
                    {
                        hit = cdda::building_anchor_in_chunk(x, z, CHUNK_SIZE);
                    }
                }
            }
            hit
        })
        .expect("no school found");
    let (cx, cz) = (anchor.x, anchor.z);

    let size: usize = 700;
    let span: f32 = std::env::var("SPAN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(pad_half * 4.0); // pad plus generous relief around it
    let min = -span / 2.0;
    let px = span / size as f32;

    let (mut lx, mut ly, mut lz) = (-0.5f32, 1.0f32, -0.5f32);
    let ll = (lx * lx + ly * ly + lz * lz).sqrt();
    lx /= ll;
    ly /= ll;
    lz /= ll;

    // Pass 1: heights + range.
    let mut hs = vec![0f32; size * size];
    let (mut hmin, mut hmax) = (f32::MAX, f32::MIN);
    for j in 0..size {
        for i in 0..size {
            let x = cx + min + (i as f32 + 0.5) * px;
            let z = cz + min + (j as f32 + 0.5) * px;
            let h = height(x, z);
            hs[j * size + i] = h;
            hmin = hmin.min(h);
            hmax = hmax.max(h);
        }
    }

    // Pass 2: hillshade × height ramp, plus a footprint outline.
    let d = 4.0f32;
    let mut buf = vec![0u8; size * size * 4];
    for j in 0..size {
        for i in 0..size {
            let x = cx + min + (i as f32 + 0.5) * px;
            let z = cz + min + (j as f32 + 0.5) * px;

            let hx = height(x + d, z) - height(x - d, z);
            let hz = height(x, z + d) - height(x, z - d);
            let (nx, ny, nz) = (-hx / (2.0 * d), 1.0f32, -hz / (2.0 * d));
            let nl = (nx * nx + ny * ny + nz * nz).sqrt();
            let shade = ((nx * lx + ny * ly + nz * lz) / nl).max(0.0);
            let t = ((hs[j * size + i] - hmin) / (hmax - hmin)).clamp(0.0, 1.0);
            let ramp = |a: f32, b: f32| a + (b - a) * t;
            let sh = 0.35 + 0.65 * shade;

            // Footprint outline: a thin magenta frame at ±pad_half.
            let on_edge = {
                let ex = (x - cx).abs();
                let ez = (z - cz).abs();
                let w = px * 1.5;
                (ex <= pad_half && ez <= pad_half)
                    && ((ex - pad_half).abs() < w || (ez - pad_half).abs() < w)
            };
            let o = (j * size + i) * 4;
            if on_edge {
                buf[o] = 235;
                buf[o + 1] = 40;
                buf[o + 2] = 160;
            } else {
                buf[o] = (ramp(58.0, 190.0) * sh).clamp(0.0, 255.0) as u8;
                buf[o + 1] = (ramp(74.0, 180.0) * sh).clamp(0.0, 255.0) as u8;
                buf[o + 2] = (ramp(42.0, 120.0) * sh).clamp(0.0, 255.0) as u8;
            }
            buf[o + 3] = 255;
        }
    }

    let path = std::env::var("OUT").unwrap_or_else(|_| "heightfield.png".into());
    let file = std::fs::File::create(&path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&buf).unwrap();
    eprintln!(
        "wrote {path} ({size}x{size}); school anchor ({cx:.0},{cz:.0}) pad_half {pad_half:.0}; relief {hmin:.1}..{hmax:.1}"
    );
}
