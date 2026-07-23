//! CDDA char → prop-vocabulary mapping. Given a resolved char→id map
//! (from parse + palettes), decide what the char becomes in our
//! prop vocabulary.

use std::collections::HashMap;

use crate::template::{PropKind, TreeKind};

/// Glass windows — a light-blue thin panel sitting in the wall line.
pub(crate) const WINDOW_COLOR: [f32; 3] = [0.50, 0.68, 0.82];

/// Wall colour by material, so parametrized wall variation shows as
/// differently-coloured houses (brick/wood/concrete/log/…).
pub(crate) fn wall_color(id: &str) -> [f32; 3] {
    if id.contains("brick") {
        [0.55, 0.32, 0.27]
    } else if id.contains("concrete") || id.contains("thconc") || id.contains("cinder") {
        [0.56, 0.56, 0.60]
    } else if id.contains("metal") || id.contains("chain") {
        [0.46, 0.49, 0.53]
    } else if id.contains("log") {
        [0.40, 0.29, 0.17]
    } else if id.contains("glass") {
        [0.40, 0.55, 0.60]
    } else if id.contains("wood") || id.contains("wall_w") {
        [0.52, 0.40, 0.25]
    } else {
        [0.48, 0.47, 0.50] // generic
    }
}

/// Fence colour by material — chain-link reads grey/metal, wooden and
/// picket fences read warm brown. The default is a weathered wood.
pub(crate) fn fence_color(id: &str) -> [f32; 3] {
    if id.contains("chain") || id.contains("metal") {
        [0.46, 0.49, 0.53]
    } else if id.contains("picket") || id.contains("wood") {
        [0.52, 0.40, 0.25]
    } else {
        [0.42, 0.32, 0.20]
    }
}

/// Map a cell's char to (prop kind, optional colour) via the resolved
/// terrain char→id map. Walls carry a material colour. Unmapped → None.
///
/// Furniture is intentionally NOT spawned (no pickup mechanic yet), but
/// toilets are a specific carve-out: they're placed by the CDDA `f_toilet`
/// furniture id and matter for the compound to feel like a real place
/// (you can walk up to one).
pub(crate) fn cell_to_prop(
    ch: char,
    terrain: &HashMap<char, String>,
    furniture: &HashMap<char, String>,
) -> Option<(PropKind, Option<[f32; 3]>)> {
    if let Some(f) = furniture.get(&ch)
        && f.contains("toilet")
    {
        return Some((PropKind::Toilet, None));
    }
    // Every other furniture char is dropped for now.
    if let Some(t) = terrain.get(&ch) {
        // A window is a translucent glass panel that sits in (and
        // orients with) the wall run — see-through from outside, drawn
        // in its own alpha pass. Kept as the base Window kind here;
        // pass 2 orients it NS/EW to match its wall run.
        if t.contains("window") {
            return Some((PropKind::Window, Some(WINDOW_COLOR)));
        }
        // Fence before wall: a fence's terrain id can technically match
        // "wall" substrings in some corpora, but a fence is a
        // yard-bounding barrier — short, see-through, doesn't seal a
        // room. Kept as the base Fence kind; pass 2 orients it NS/EW.
        if t.contains("fence") && !t.contains("gate") {
            return Some((PropKind::Fence, Some(fence_color(t))));
        }
        if t.contains("wall") && !t.contains("gate") {
            return Some((PropKind::Wall, Some(wall_color(t))));
        }
    }
    None
}

/// Map a cell's char to a tree species via the resolved terrain map.
/// CDDA authors trees as `t_tree_*` terrain (`t_tree_apple`, `_pine`,
/// `_birch`, `_willow`, `_maple`, …); we fold its many species onto the
/// handful we render. Stumps map to `Stump`; non-tree terrain and the
/// `young` sapling variants we've no geometry for yet → None.
pub(crate) fn cell_to_tree(ch: char, terrain: &HashMap<char, String>) -> Option<TreeKind> {
    let t = terrain.get(&ch)?;
    // A cut stump is its own thing — matched first because `t_stump`
    // needn't contain "tree".
    if t.contains("stump") {
        return Some(TreeKind::Stump);
    }
    if !t.contains("tree") || t.contains("young") {
        return None;
    }
    // A dead snag is dead whatever it was; fungal growth is its own thing.
    if t.contains("dead") {
        return Some(TreeKind::Dead);
    }
    if t.contains("fungal") {
        return Some(TreeKind::Fungal);
    }
    // Fruit/nut orchard trees → the round fruit-tree form.
    let fruit = ["apple", "pear", "cherry", "peach", "plum", "apricot", "mulberry",
                 "walnut", "pecan", "hazelnut", "hickory", "chestnut", "elderberry"];
    Some(if fruit.iter().any(|f| t.contains(f)) {
        TreeKind::Apple
    } else if t.contains("pine") || t.contains("fir") || t.contains("conifer") || t.contains("spruce") {
        TreeKind::Pine
    } else if t.contains("birch") {
        TreeKind::Birch
    } else if t.contains("willow") {
        TreeKind::Willow
    } else if t.contains("maple") {
        TreeKind::Maple
    } else if t.contains("oak") || t.contains("elm") || t.contains("beech") {
        TreeKind::Oak
    } else {
        TreeKind::Generic
    })
}

/// What a wall-line cell IS — the per-cell kind the `WallGraph` node
/// carries. `None` = not part of the wall line at all.
///
/// Fences are DELIBERATELY excluded: they bound a yard, not a room. A
/// fenced-in area must stay exterior for flood-fill, or the building
/// walls facing that yard get misclassified as room-to-room dividers.
pub(crate) fn cell_wall_kind(
    ch: char,
    terrain: &HashMap<char, String>,
) -> Option<(crate::template::WallCellKind, Option<[f32; 3]>)> {
    use crate::template::WallCellKind;
    let t = terrain.get(&ch)?;
    if t.contains("fence") {
        return None;
    }
    if t.contains("window") {
        return Some((WallCellKind::Window, Some(WINDOW_COLOR)));
    }
    if t.contains("door") || t.contains("gate") {
        return Some((WallCellKind::Door, None));
    }
    if t.contains("wall") {
        return Some((WallCellKind::Solid, Some(wall_color(t))));
    }
    None
}

/// Does a char's resolved terrain id form part of the wall LINE — the
/// connective tissue that seals a building's interior? Walls, windows,
/// doors, gates all qualify (doors + gates don't render as a prop yet,
/// but they still complete the wall line for flood-fill). Delegates to
/// `cell_wall_kind` — ONE definition of the wall line, so the graph
/// and the flood-fill can never disagree about what seals a room.
pub(crate) fn is_wall_line_char(ch: char, terrain: &HashMap<char, String>) -> bool {
    cell_wall_kind(ch, terrain).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_map_to_the_prop_vocabulary() {
        let s = |v: &str| v.to_string();
        let terrain: HashMap<char, String> = [
            ('w', s("t_wall_log")),
            ('W', s("t_chainfence")),
            ('^', s("t_chaingate_c")),
            ('.', s("t_thconc_floor")),
        ]
        .into_iter()
        .collect();
        let furniture: HashMap<char, String> = [
            ('t', s("f_toilet")),
            ('b', s("f_bed")), // furniture that stays disabled
        ]
        .into_iter()
        .collect();

        let kind = |ch: char| cell_to_prop(ch, &terrain, &furniture).map(|(k, _)| k);
        assert_eq!(kind('w'), Some(PropKind::Wall));
        assert_eq!(kind('W'), Some(PropKind::Fence));
        assert_eq!(kind('^'), None); // gate skipped
        assert_eq!(kind('.'), None); // floor skipped
        assert_eq!(kind(' '), None); // unknown
        assert_eq!(kind('t'), Some(PropKind::Toilet)); // f_toilet carve-out
        assert_eq!(kind('b'), None); // other furniture disabled
        // Walls carry a material colour, and materials differ.
        assert!(cell_to_prop('w', &terrain, &furniture).unwrap().1.is_some());
        assert_ne!(wall_color("t_brick_wall"), wall_color("t_wall_log"));

        // A window becomes a translucent glass panel (its own kind),
        // tinted, sitting in the wall line.
        let win: HashMap<char, String> = [(':', s("t_window"))].into_iter().collect();
        assert_eq!(
            cell_to_prop(':', &win, &HashMap::new()),
            Some((PropKind::Window, Some(WINDOW_COLOR)))
        );
    }
}
