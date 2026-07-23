//! Discussion aid (scratch, not merged): the heightfield hillshade
//! around the school with TWO outlines — magenta = the footprint we
//! flatten today (full authored stamp incl. yard, TERRAIN.md decision
//! 5), cyan = the AABB hull of the building's sealing wall/window
//! props (the enclosed-space proxy a WallGraph would make exact).
//! The band between the outlines is yard that is currently a flat
//! plateau but could roll if terrain flattening became room-aware.
//!
//!   OUT=/path/foo.png cargo run --example room_hull_preview

use game::chunk::CHUNK_SIZE;
use game::terrain::height;

fn main() {
    let (bt, _failures) = cdda::load_building_templates();
    let num = bt.templates.len();
    // IDX picks the template (load order: garage 0, shed 1, daycare 2,
    // school 3, houses 4+); default = the school (largest footprint).
    let school = std::env::var("IDX")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| {
            bt.half_extents
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)
                .expect("no templates")
        });
    let pad_half = cdda::BUILDING_FOOTPRINT_HALF.max(bt.half_extents[school]);

    // Hull of the sealing structure: AABB over wall + window props.
    let (mut wx0, mut wx1, mut wz0, mut wz1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in &bt.templates[school].props {
        use cdda::PropKind::*;
        if !matches!(p.kind, Wall | WallNS | WallEW | Window | WindowNS | WindowEW) {
            continue;
        }
        let s = p.size.unwrap_or(bevy_math::Vec3::splat(cdda::CDDA_TILE));
        wx0 = wx0.min(p.offset.x - s.x * 0.5);
        wx1 = wx1.max(p.offset.x + s.x * 0.5);
        wz0 = wz0.min(p.offset.z - s.z * 0.5);
        wz1 = wz1.max(p.offset.z + s.z * 0.5);
    }

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
    let span: f32 = pad_half * 4.0;
    let min = -span / 2.0;
    let px = span / size as f32;

    let (mut lx, mut ly, mut lz) = (-0.5f32, 1.0f32, -0.5f32);
    let ll = (lx * lx + ly * ly + lz * lz).sqrt();
    lx /= ll;
    ly /= ll;
    lz /= ll;

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

            let w = px * 1.5;
            // Magenta: today's flatten rectangle (±pad_half).
            let on_pad_edge = {
                let ex = (x - cx).abs();
                let ez = (z - cz).abs();
                (ex <= pad_half && ez <= pad_half)
                    && ((ex - pad_half).abs() < w || (ez - pad_half).abs() < w)
            };
            // Cyan: the sealing-wall hull (room-aware flatten candidate).
            let on_hull_edge = {
                let rx = x - cx;
                let rz = z - cz;
                (rx >= wx0 - w && rx <= wx1 + w && rz >= wz0 - w && rz <= wz1 + w)
                    && ((rx - wx0).abs() < w
                        || (rx - wx1).abs() < w
                        || (rz - wz0).abs() < w
                        || (rz - wz1).abs() < w)
            };
            let o = (j * size + i) * 4;
            if on_hull_edge {
                buf[o] = 40;
                buf[o + 1] = 220;
                buf[o + 2] = 235;
            } else if on_pad_edge {
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

    let path = std::env::var("OUT").unwrap_or_else(|_| "room_hull.png".into());
    let file = std::fs::File::create(&path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size as u32, size as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&buf).unwrap();
    eprintln!(
        "wrote {path}; pad_half {pad_half:.0} vs wall hull x[{wx0:.0},{wx1:.0}] z[{wz0:.0},{wz1:.0}]; relief {hmin:.1}..{hmax:.1}"
    );
}
