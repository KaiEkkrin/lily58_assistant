//! The keyboard's keymap as read with `VIA_GET_BUFFER`.

use crate::keycodes::{self, KC_NO, KC_TRNS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    layers: u8,
    rows: u8,
    cols: u8,
    codes: Vec<u16>,
}

#[derive(Debug, thiserror::Error)]
#[error("keymap buffer is {got} bytes but {layers}x{rows}x{cols} needs {want}")]
pub struct KeymapError {
    got: usize,
    want: usize,
    layers: u8,
    rows: u8,
    cols: u8,
}

/// Where an OS-reported key most likely is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHit {
    pub row: u8,
    pub col: u8,
    pub layer: u8,
    pub code: u16,
}

impl Keymap {
    pub fn buffer_len(layers: u8, rows: u8, cols: u8) -> usize {
        layers as usize * rows as usize * cols as usize * 2
    }

    pub fn from_buffer(layers: u8, rows: u8, cols: u8, buf: &[u8]) -> Result<Keymap, KeymapError> {
        let want = Self::buffer_len(layers, rows, cols);
        if buf.len() != want {
            return Err(KeymapError { got: buf.len(), want, layers, rows, cols });
        }
        let codes = buf.as_chunks::<2>().0.iter().map(|&b| u16::from_be_bytes(b)).collect();
        Ok(Keymap { layers, rows, cols, codes })
    }

    pub fn layers(&self) -> u8 {
        self.layers
    }

    pub fn rows(&self) -> u8 {
        self.rows
    }

    pub fn cols(&self) -> u8 {
        self.cols
    }

    pub fn get(&self, layer: u8, row: u8, col: u8) -> u16 {
        if layer >= self.layers || row >= self.rows || col >= self.cols {
            return KC_NO;
        }
        self.codes[(layer as usize * self.rows as usize + row as usize) * self.cols as usize + col as usize]
    }

    /// QMK lookup: the highest layer set in `mask` whose code is not `KC_TRNS`.
    pub fn resolve(&self, mask: u32, row: u8, col: u8) -> (u8, u16) {
        for layer in (0..self.layers.min(32)).rev() {
            if mask & (1u32 << layer) != 0 {
                let code = self.get(layer, row, col);
                if code != KC_TRNS {
                    return (layer, code);
                }
            }
        }
        (0, self.get(0, row, col))
    }

    /// First key (matrix order) whose resolved code sends one of `usages`, searching the
    /// layers in `mask`, then layer 0, then each other layer on its own.
    pub fn find_position(&self, mask: u32, usages: &[u8]) -> Option<KeyHit> {
        let masks = std::iter::once(mask).chain((0..self.layers.min(32)).map(|l| 1u32 << l));
        for m in masks {
            for row in 0..self.rows {
                for col in 0..self.cols {
                    let (layer, code) = self.resolve(m, row, col);
                    if keycodes::tap_basic(code).is_some_and(|b| usages.contains(&b)) {
                        return Some(KeyHit { row, col, layer, code });
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2 layers, 2 rows, 2 cols.
    // layer 0: A    B   / LSFT MO(1)
    // layer 1: F1  TRNS / TRNS TRNS
    const CODES: [u16; 8] = [0x0004, 0x0005, 0x00E1, 0x5221, 0x003A, 0x0001, 0x0001, 0x0001];

    fn keymap() -> Keymap {
        let buf: Vec<u8> = CODES.iter().flat_map(|c| c.to_be_bytes()).collect();
        Keymap::from_buffer(2, 2, 2, &buf).unwrap()
    }

    #[test]
    fn decodes_big_endian_layer_row_col() {
        let km = keymap();
        assert_eq!(km.get(0, 0, 1), 0x0005);
        assert_eq!(km.get(0, 1, 1), 0x5221);
        assert_eq!(km.get(1, 0, 0), 0x003A);
        assert_eq!(km.get(5, 0, 0), KC_NO);
        assert!(Keymap::from_buffer(2, 2, 2, &[0; 7]).is_err());
    }

    #[test]
    fn resolves_through_transparent_keys() {
        let km = keymap();
        assert_eq!(km.resolve(0b01, 0, 0), (0, 0x0004));
        assert_eq!(km.resolve(0b11, 0, 0), (1, 0x003A));
        assert_eq!(km.resolve(0b11, 0, 1), (0, 0x0005)); // TRNS on layer 1 falls to layer 0
    }

    #[test]
    fn finds_positions_on_active_layer_then_layer_zero_then_others() {
        let km = keymap();
        assert_eq!(km.find_position(0b01, &[0x05]), Some(KeyHit { row: 0, col: 1, layer: 0, code: 0x0005 }));
        assert_eq!(km.find_position(0b01, &[0x3A]), Some(KeyHit { row: 0, col: 0, layer: 1, code: 0x003A }));
        assert_eq!(km.find_position(0b11, &[0x04]), Some(KeyHit { row: 0, col: 0, layer: 0, code: 0x0004 }));
        assert_eq!(km.find_position(0b01, &[0x28]), None);
    }
}
