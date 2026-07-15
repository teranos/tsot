//! CDDA char → prop-vocabulary mapping. Given a resolved char→id map
//! (from parse + palettes), decide what the char becomes in our
//! prop vocabulary.

use std::collections::HashMap;

use crate::template::PropKind;

/// Glass windows — a light-blue thin panel sitting in the wall line.
pub(crate) const WINDOW_COLOR: [f32; 3] = [0.50, 0.68, 0.82];

/// Wall/fence colour by material, so parametrized wall variation shows
/// as differently-coloured houses (brick/wood/concrete/log/…).
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
    } else if id.contains("wood") || id.contains("wall_w") || id.contains("fence") {
        [0.52, 0.40, 0.25]
    } else {
        [0.48, 0.47, 0.50] // generic
    }
}

/// Map a cell's char to (prop kind, optional colour) via the resolved
/// terrain char→id map. Walls carry a material colour. Unmapped → None.
pub(crate) fn cell_to_prop(
    ch: char,
    terrain: &HashMap<char, String>,
) -> Option<(PropKind, Option<[f32; 3]>)> {
    if let Some(t) = terrain.get(&ch) {
        // A window is a translucent glass panel that sits in (and
        // orients with) the wall run — see-through from outside, drawn
        // in its own alpha pass. Kept as the base Window kind here;
        // pass 2 orients it NS/EW to match its wall run.
        if t.contains("window") {
            return Some((PropKind::Window, Some(WINDOW_COLOR)));
        }
        if (t.contains("wall") || t.contains("fence")) && !t.contains("gate") {
            return Some((PropKind::Wall, Some(wall_color(t))));
        }
    }
    None
}

/// Does a char's resolved terrain id form part of the wall LINE — the
/// connective tissue that seals a building's interior? Walls, windows,
/// doors, gates all qualify (doors + gates don't render as a prop yet,
/// but they still complete the wall line for flood-fill).
pub(crate) fn is_wall_line_char(ch: char, terrain: &HashMap<char, String>) -> bool {
    let Some(t) = terrain.get(&ch) else {
        return false;
    };
    t.contains("wall")
        || t.contains("fence")
        || t.contains("window")
        || t.contains("door")
        || t.contains("gate")
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

        let kind = |ch: char| cell_to_prop(ch, &terrain).map(|(k, _)| k);
        assert_eq!(kind('w'), Some(PropKind::Wall));
        assert_eq!(kind('W'), Some(PropKind::Wall));
        assert_eq!(kind('^'), None); // gate skipped
        assert_eq!(kind('.'), None); // floor skipped
        assert_eq!(kind(' '), None); // unknown
        // Walls carry a material colour, and materials differ.
        assert!(cell_to_prop('w', &terrain).unwrap().1.is_some());
        assert_ne!(wall_color("t_brick_wall"), wall_color("t_wall_log"));

        // A window becomes a translucent glass panel (its own kind),
        // tinted, sitting in the wall line.
        let win: HashMap<char, String> = [(':', s("t_window"))].into_iter().collect();
        assert_eq!(
            cell_to_prop(':', &win),
            Some((PropKind::Window, Some(WINDOW_COLOR)))
        );
    }
}
