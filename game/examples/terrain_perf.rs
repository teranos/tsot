//! Measure the per-frame terrain geometry cost the render path pays.
//!   cargo +nightly run --release --example terrain_perf

use game::scene::{dev_grid_mesh, terrain_surface_mesh};
use game::terrain;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let (px, pz) = (18_000.0f32, 18_000.0f32);
    let _ = terrain::height(0.0, 0.0); // warm the template cache

    let n = 200;
    let mut segs = 0;
    let t = Instant::now();
    for _ in 0..n {
        segs = black_box(dev_grid_mesh(black_box(px), pz)).len();
    }
    let grid_us = t.elapsed().as_micros() as f64 / n as f64;

    let mut verts = 0;
    let t = Instant::now();
    for _ in 0..n {
        verts = black_box(terrain_surface_mesh(black_box(px), pz)).verts.len();
    }
    let surf_us = t.elapsed().as_micros() as f64 / n as f64;

    let m = 200_000;
    let mut acc = 0.0f32;
    let t = Instant::now();
    for i in 0..m {
        acc += terrain::height(px + i as f32 * 0.7, pz - i as f32 * 0.3);
    }
    let h_ns = t.elapsed().as_nanos() as f64 / m as f64;
    black_box(acc);

    println!("height():             {h_ns:6.0} ns/call");
    println!("dev_grid_mesh:        {grid_us:6.0} us/frame  ({segs} segments)");
    println!("terrain_surface_mesh: {surf_us:6.0} us/frame  ({verts} verts)");
    println!(
        "=> BEFORE (regen every frame): {:.2} ms/frame",
        (grid_us + surf_us) / 1000.0
    );

    // WITH the snap-key cache: the player walks at KEYBOARD_SPEED and the
    // geometry is regenerated only when the snap key changes.
    use game::scene::{grid_snap, surface_snap};
    let speed = 18.0f32; // physics::KEYBOARD_SPEED
    let frames = 600;
    let (mut x, mut z) = (px, pz);
    let (mut gkey, mut skey) = (None, None);
    let (mut rg, mut rs) = (0u32, 0u32);
    let t = Instant::now();
    for _ in 0..frames {
        x += speed * 0.8;
        z += speed * 0.6;
        let gk = grid_snap(x, z);
        if Some(gk) != gkey {
            black_box(dev_grid_mesh(x, z));
            gkey = Some(gk);
            rg += 1;
        }
        let sk = surface_snap(x, z);
        if Some(sk) != skey {
            black_box(terrain_surface_mesh(x, z));
            skey = Some(sk);
            rs += 1;
        }
    }
    let amort_us = t.elapsed().as_micros() as f64 / frames as f64;
    println!(
        "=> AFTER (snap-key cache, walking): {:.2} ms/frame amortized  ({rg} grid + {rs} surf regens / {frames} frames)",
        amort_us / 1000.0
    );
}
