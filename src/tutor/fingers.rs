//! Which finger each key belongs to: the basis for drill selection and the finger colouring.
//!
//! Hardwired for the Lily58's 10x6 matrix rather than derived from the layout geometry. The
//! columnar stagger is half the row pitch, so clustering keys by their `y` coordinate merges
//! the home and bottom rows, and a heuristic that misfires produces subtly wrong colours that
//! are hard to notice. `validate` checks this table against the layout the keyboard reported,
//! so a definition that doesn't match fails loudly instead.

use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finger {
    Pinky,
    Ring,
    Middle,
    Index,
    Thumb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Number,
    Top,
    Home,
    Bottom,
    Thumb,
}

/// How far a key is from the finger's resting column: the pinky's outer column and the index
/// finger's inner column are stretches, everything else is where the finger already sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Normal,
    Outward,
    Inward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub hand: Hand,
    pub finger: Finger,
    pub band: Band,
    pub reach: Reach,
}

pub const ROWS: u8 = 10;
pub const COLS: u8 = 6;

/// Columns from each half's outer edge inwards.
const COLUMNS: [(Finger, Reach); COLS as usize] = [
    (Finger::Pinky, Reach::Outward),
    (Finger::Pinky, Reach::Normal),
    (Finger::Ring, Reach::Normal),
    (Finger::Middle, Reach::Normal),
    (Finger::Index, Reach::Normal),
    (Finger::Index, Reach::Inward),
];

const BANDS: [Band; 4] = [Band::Number, Band::Top, Band::Home, Band::Bottom];

/// Positions the firmware's matrix supports that this build doesn't populate: the OLED screens
/// sit there, and the keymap still assigns them something (`KC_MPLY` and `KC_MUTE`).
const NO_FINGER: [(u8, u8); 2] = [(4, 5), (9, 0)];

/// Which finger reaches this key, or `None` for a position no finger types.
pub fn spot(row: u8, col: u8) -> Option<Spot> {
    if col >= COLS {
        return None;
    }
    let (hand, half_row) = match row {
        0..=4 => (Hand::Left, row),
        5..=9 => (Hand::Right, row - 5),
        _ => return None,
    };
    // The left half's columns run from the inside out, so its outer edge is column 5.
    let from_outside = if hand == Hand::Left { COLS - 1 - col } else { col };
    if let Some(&band) = BANDS.get(half_row as usize) {
        let (finger, reach) = COLUMNS[from_outside as usize];
        return Some(Spot { hand, finger, band, reach });
    }
    // The extras row: the key between the halves, then four thumbs.
    match (hand, col) {
        (Hand::Left, 0) | (Hand::Right, 5) => {
            Some(Spot { hand, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward })
        }
        (_, 1..=4) => Some(Spot { hand, finger: Finger::Thumb, band: Band::Thumb, reach: Reach::Normal }),
        _ => None,
    }
}

/// Checks the layout the keyboard reported against the table, both ways round. A mismatch means
/// a different board or a changed definition, and the tutor refuses to run rather than colour
/// keys wrongly.
pub fn validate(layout: &Layout) -> Result<(), String> {
    if layout.rows != ROWS || layout.cols != COLS {
        return Err(format!(
            "the typing tutor expects a {ROWS}x{COLS} matrix; this keyboard reports {}x{}",
            layout.rows, layout.cols
        ));
    }
    let mut present = [[false; COLS as usize]; ROWS as usize];
    for key in &layout.keys {
        if key.row >= ROWS || key.col >= COLS {
            return Err(format!("this keyboard has a key at ({}, {}), outside the {ROWS}x{COLS} matrix", key.row, key.col));
        }
        present[key.row as usize][key.col as usize] = true;
    }
    for row in 0..ROWS {
        for col in 0..COLS {
            match (spot(row, col).is_some(), present[row as usize][col as usize]) {
                (true, false) => return Err(format!("this keyboard has no key at ({row}, {col}), which the finger map expects")),
                (false, true) if !NO_FINGER.contains(&(row, col)) => {
                    return Err(format!("this keyboard has a key at ({row}, {col}) that the finger map doesn't know"));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    fn reference_layout() -> Layout {
        let def = serde_json::from_str(include_str!("../../tests/fixtures/lily58-definition.json")).unwrap();
        Layout::from_definition(&def).unwrap()
    }

    fn at(row: u8, col: u8) -> Spot {
        spot(row, col).unwrap_or_else(|| panic!("({row}, {col}) should have a spot"))
    }

    /// Landmarks read off the reference board's layer 0, which is
    /// `KC_G KC_F KC_D KC_S KC_A KC_LCTL` across row 2 (columns 0 to 5).
    #[test]
    fn home_row_matches_the_keys_it_types() {
        assert_eq!(at(2, 4), Spot { hand: Hand::Left, finger: Finger::Pinky, band: Band::Home, reach: Reach::Normal }); // A
        assert_eq!(at(2, 3), Spot { hand: Hand::Left, finger: Finger::Ring, band: Band::Home, reach: Reach::Normal }); // S
        assert_eq!(at(2, 2), Spot { hand: Hand::Left, finger: Finger::Middle, band: Band::Home, reach: Reach::Normal }); // D
        assert_eq!(at(2, 1), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Home, reach: Reach::Normal }); // F
        assert_eq!(at(2, 0), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Home, reach: Reach::Inward }); // G
        assert_eq!(at(2, 5), Spot { hand: Hand::Left, finger: Finger::Pinky, band: Band::Home, reach: Reach::Outward }); // LCTL
        // The right half mirrors it: `KC_QUOT KC_SCLN KC_L KC_K KC_J KC_H` across row 7.
        assert_eq!(at(7, 0), Spot { hand: Hand::Right, finger: Finger::Pinky, band: Band::Home, reach: Reach::Outward }); // '
        assert_eq!(at(7, 4), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Home, reach: Reach::Normal }); // J
        assert_eq!(at(7, 5), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Home, reach: Reach::Inward }); // H
    }

    /// Row 4 is `KC_LBRC KC_SPC MO(1) KC_LGUI KC_LALT KC_MPLY`: the inner bottom key, four
    /// thumbs, and one position this build doesn't populate (an OLED sits there).
    #[test]
    fn the_extras_row_is_thumbs_an_inner_key_and_one_unpopulated_position() {
        assert_eq!(at(4, 0), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward });
        assert_eq!(at(9, 5), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward });
        for col in 1..=4 {
            assert_eq!(at(4, col).finger, Finger::Thumb, "left thumb {col}");
            assert_eq!(at(9, col).finger, Finger::Thumb, "right thumb {col}");
        }
        assert_eq!(spot(4, 5), None, "the left OLED position is not a key on this build");
        assert_eq!(spot(9, 0), None, "the right OLED position is not a key on this build");
    }

    #[test]
    fn the_table_covers_the_definition_exactly() {
        let layout = reference_layout();
        assert_eq!(layout.keys.len(), 60);
        let spots = (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (r, c))).filter(|&(r, c)| spot(r, c).is_some()).count();
        assert_eq!(spots, 58, "58 real keys plus the 2 unpopulated positions is the 60 in the definition");
        assert_eq!(validate(&layout), Ok(()));
    }

    #[test]
    fn a_layout_that_does_not_match_is_rejected() {
        let mut layout = reference_layout();
        layout.keys.retain(|k| (k.row, k.col) != (2, 4));
        assert!(validate(&layout).unwrap_err().contains("(2, 4)"));

        let mut wrong_size = reference_layout();
        wrong_size.cols = 5;
        assert!(validate(&wrong_size).unwrap_err().contains("10x6"));
    }
}
