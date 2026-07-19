//! Measure the per-frame terrain geometry cost the render path pays.
//! The draped-bar grid is gone (it's painted in the ground shader now, at
//! zero geometry cost), so the only per-frame geometry is the surface —
//! and it's cached on a snap key, so it regenerates only on a cell cross.
//!   cargo +nightly run --release --example terrain_perf

use game::scene::{surface_snap, terrain_surface_mesh};
use game::terrain;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let (px, pz) = (18_000.0f32, 18_000.0f32);
    let _ = terrain::height(0.0, 0.0); // warm the template cache

    let n = 200;
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
    println!("terrain_surface_mesh: {surf_us:6.0} us/frame  ({verts} verts) — on a cell cross only");

    // Amortized surface cost while walking at KEYBOARD_SPEED (regenerated
    // only when the surface snap key changes).
    let speed = 18.0f32;
    let frames = 600;
    let (mut x, mut z) = (px, pz);
    let mut skey = None;
    let mut regens = 0u32;
    let t = Instant::now();
    for _ in 0..frames {
        x += speed * 0.8;
        z += speed * 0.6;
        let sk = surface_snap(x, z);
        if Some(sk) != skey {
            black_box(terrain_surface_mesh(x, z));
            skey = Some(sk);
            regens += 1;
        }
    }
    let amort_us = t.elapsed().as_micros() as f64 / frames as f64;
    println!(
        "=> surface amortized while walking: {:.2} ms/frame  ({regens} regens / {frames} frames); grid = 0 (shader)",
        amort_us / 1000.0
    );
}
