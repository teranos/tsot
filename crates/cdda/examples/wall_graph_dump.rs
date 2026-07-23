//! Diagnostic: dump a template's `WallGraph` as JSON lines — one
//! object per node and per edge — for visual inspection of the wall
//! topology (slice 1 of walls-on-mesh, see `game/docs/RENDER.md`).
//!
//!   cargo run --example wall_graph_dump              # P-shape fixture
//!   cargo run --example wall_graph_dump house_01     # a real corpus house

use cdda::{CDDA_TILE, Template, mapgen_to_template};

const P_SHAPE: &str = r#"[{
    "om_terrain": "p_shape",
    "object": {
        "rows": [
            "wwwww",
            "o d w",
            "wdwdw",
            "  w w",
            "  w o",
            "  www"
        ],
        "terrain": {
            "w": "t_wall",
            "o": "t_window",
            "d": "t_door_c"
        }
    }
}]"#;

fn dump(label: &str, t: &Template) {
    println!("{{\"template\":\"{label}\",\"nodes\":{},\"edges\":{}}}", t.walls.nodes.len(), t.walls.edges.len());
    for (i, n) in t.walls.nodes.iter().enumerate() {
        println!(
            "{{\"node\":{i},\"x\":{},\"z\":{},\"kind\":\"{:?}\"}}",
            n.offset.x, n.offset.z, n.kind
        );
    }
    for (i, e) in t.walls.edges.iter().enumerate() {
        println!("{{\"edge\":{i},\"a\":{},\"b\":{}}}", e.a, e.b);
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("house_01") => {
            let t = cdda::house_template().expect("house_01 imports");
            dump("house_01", &t);
        }
        _ => {
            let t = mapgen_to_template(P_SHAPE, "p_shape", CDDA_TILE, 0).expect("p_shape imports");
            dump("p_shape", &t);
        }
    }
}
