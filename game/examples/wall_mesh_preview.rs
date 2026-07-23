//! Diagnostic: dump the slice-2 wall mesh of a template as JSON lines
//! (one vertex / one triangle per line) for external visualisation —
//! the mesh-side sibling of `cdda`'s `wall_graph_dump`.
//!
//!   cargo run --example wall_mesh_preview            # P-shape fixture
//!   cargo run --example wall_mesh_preview house_01   # a corpus house

use game::wall_mesh::wall_graph_mesh;

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

fn main() {
    let t = match std::env::args().nth(1).as_deref() {
        Some("house_01") => cdda::house_template().expect("house_01 imports"),
        _ => cdda::mapgen_to_template(P_SHAPE, "p_shape", cdda::CDDA_TILE, 0)
            .expect("p_shape imports"),
    };
    let (verts, idx) = wall_graph_mesh(&t.walls);
    println!("{{\"verts\":{},\"tris\":{}}}", verts.len(), idx.len() / 3);
    for v in &verts {
        println!(
            "{{\"v\":[{},{},{}],\"n\":[{},{},{}]}}",
            v.pos[0], v.pos[1], v.pos[2], v.normal[0], v.normal[1], v.normal[2]
        );
    }
    for tri in idx.chunks(3) {
        println!("{{\"t\":[{},{},{}]}}", tri[0], tri[1], tri[2]);
    }
}
