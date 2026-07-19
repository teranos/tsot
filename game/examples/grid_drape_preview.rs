//! Interim visual proof of Slice 4 — the draped dev-grid. Reconstructs
//! the segment endpoints from `dev_grid_mesh`'s MeshInstances, iso-
//! projects them, and draws the wireframe. NOT the merge-bar render (that
//! is the grid in the real game render on lavapipe, Slice 5) — this just
//! makes the drape visible now. Vertical relief is exaggerated ×VEXAG so
//! the undulation reads; the flat school pad shows as a flat lattice.
//!
//!   cargo +nightly run --example grid_drape_preview

use game::scene::dev_grid_mesh;

const SIZE: usize = 820;

fn main() {
    // Straddle the school pad's +x edge: left half wholly on the flat pad,
    // right half rolling over open relief.
    let (cx, cz) = (13_708.0f32, 44_400.0f32);
    let grid = dev_grid_mesh(cx, cz);

    let vexag = 4.0f32;
    let (c, s) = (0.866_025f32, 0.5f32);
    let proj = |x: f32, y: f32, z: f32| {
        let (rx, rz) = (x - cx, z - cz);
        ((rx - rz) * c, (rx + rz) * s - y * vexag)
    };

    // Height range for colouring.
    let (mut hmin, mut hmax) = (f32::MAX, f32::MIN);
    for inst in &grid {
        hmin = hmin.min(inst.pos[1]);
        hmax = hmax.max(inst.pos[1]);
    }

    struct Seg {
        a: (f32, f32),
        b: (f32, f32),
        col: [u8; 3],
        depth: f32,
    }
    let mut segs = Vec::with_capacity(grid.len());
    let (mut minx, mut maxx, mut miny, mut maxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for inst in &grid {
        let [mx, my, mz] = inst.pos;
        let [dx, dy, dz] = [inst.axis[0], inst.axis[1], inst.axis[2]];
        let hl = inst.scale[1] * 0.5;
        let pa = proj(mx - dx * hl, my - dy * hl, mz - dz * hl);
        let pb = proj(mx + dx * hl, my + dy * hl, mz + dz * hl);
        for p in [pa, pb] {
            minx = minx.min(p.0);
            maxx = maxx.max(p.0);
            miny = miny.min(p.1);
            maxy = maxy.max(p.1);
        }
        // Colour by height: green low → tan high; a flat pad is one band.
        let t = ((my - hmin) / (hmax - hmin).max(1.0)).clamp(0.0, 1.0);
        let major = inst.color[0] > 0.3;
        let dim = if major { 1.0 } else { 0.6 };
        let col = [
            ((70.0 + 150.0 * t) * dim) as u8,
            ((95.0 + 105.0 * t) * dim) as u8,
            ((60.0 + 70.0 * t) * dim) as u8,
        ];
        segs.push(Seg { a: pa, b: pb, col, depth: mx + mz });
    }
    segs.sort_by(|p, q| p.depth.partial_cmp(&q.depth).unwrap()); // painter's: far first

    let margin = 40.0f32;
    let scale = ((SIZE as f32 - 2.0 * margin) / (maxx - minx))
        .min((SIZE as f32 - 2.0 * margin) / (maxy - miny));
    let to_px = |p: (f32, f32)| {
        (
            (margin + (p.0 - minx) * scale) as i32,
            (margin + (p.1 - miny) * scale) as i32,
        )
    };

    let mut buf = vec![0u8; SIZE * SIZE * 4];
    for px in buf.chunks_exact_mut(4) {
        px.copy_from_slice(&[16, 20, 26, 255]);
    }
    for seg in &segs {
        let (x0, y0) = to_px(seg.a);
        let (x1, y1) = to_px(seg.b);
        draw_line(&mut buf, x0, y0, x1, y1, seg.col);
    }

    let path = std::env::var("OUT").unwrap_or_else(|_| "grid_drape.png".into());
    let file = std::fs::File::create(&path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), SIZE as u32, SIZE as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&buf).unwrap();
    eprintln!("wrote {path}; {} segments, height {hmin:.0}..{hmax:.0}", grid.len());
}

fn draw_line(buf: &mut [u8], x0: i32, y0: i32, x1: i32, y1: i32, col: [u8; 3]) {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let (mut x, mut y, mut err) = (x0, y0, dx + dy);
    loop {
        if x >= 0 && y >= 0 && (x as usize) < SIZE && (y as usize) < SIZE {
            let o = ((y as usize) * SIZE + x as usize) * 4;
            buf[o] = col[0];
            buf[o + 1] = col[1];
            buf[o + 2] = col[2];
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
