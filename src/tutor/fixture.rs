//! The reference keyboard's keymap, for tests.
//!
//! Hint and generation tests are only worth much against a realistic keymap, and
//! `hid::fake::SMALL_KEYMAP` is a 2x3 toy. This parses the layer blocks a `--probe` run prints,
//! so the fixture stays human-readable and can be diffed against a fresh dump after a remap.

use crate::keycodes;
use crate::keymap::Keymap;

const PROBE_DUMP: &str = include_str!("../../tests/fixtures/lily58-keymap.txt");

pub fn reference_keymap() -> Keymap {
    keymap_from_probe(PROBE_DUMP)
}

/// Parses the `Layer N:` / `row R:` blocks `probe.rs` prints. Rows arrive layer-major, which is
/// the order `Keymap::from_buffer` wants.
pub fn keymap_from_probe(text: &str) -> Keymap {
    let mut codes: Vec<u16> = Vec::new();
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("row ") else { continue };
        let (_, cells) = rest.split_once(':').expect("a row line has a colon after the row number");
        codes.extend(cells.split_whitespace().map(code_for));
    }
    let layers = u8::try_from(codes.len() / (10 * 6)).expect("the dump is a whole number of layers");
    let buf: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
    Keymap::from_buffer(layers, 10, 6, &buf).expect("the dump is a whole number of 10x6 layers")
}

fn code_for(name: &str) -> u16 {
    if let Some(hex) = name.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad hex keycode {name:?}"));
    }
    if let Some(inner) = name.strip_prefix("LSFT(").and_then(|s| s.strip_suffix(')')) {
        return 0x0200 | code_for(inner);
    }
    if let Some(inner) = name.strip_prefix("LALT(").and_then(|s| s.strip_suffix(')')) {
        return 0x0400 | code_for(inner);
    }
    if let Some(layer) = name.strip_prefix("MO(").and_then(|s| s.strip_suffix(')')) {
        return 0x5220 + layer.parse::<u16>().unwrap_or_else(|_| panic!("bad layer in {name:?}"));
    }
    (0x00..=0xFFu16)
        .find(|&c| keycodes::basic_name(c as u8) == Some(name))
        .unwrap_or_else(|| panic!("unknown keycode name {name:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_reference_dump() {
        let km = reference_keymap();
        assert_eq!((km.layers(), km.rows(), km.cols()), (4, 10, 6));
        assert_eq!(km.get(0, 2, 4), 0x0004, "layer 0 row 2 is KC_G KC_F KC_D KC_S KC_A KC_LCTL, so col 4 is KC_A");
        assert_eq!(km.get(0, 4, 2), 0x5221, "MO(1) on the left thumb");
        assert_eq!(km.get(0, 9, 3), 0x5222, "MO(2) on the right thumb");
        assert_eq!(km.get(0, 4, 1), 0x002C, "KC_SPC on the big left thumb key");
        assert_eq!(km.get(0, 4, 5), 0x00AE, "KC_MPLY at the unpopulated left position");
        assert_eq!(km.get(1, 8, 2), 0x022F, "LSFT(KC_LBRC) = {{ on layer 1");
        assert_eq!(km.get(1, 2, 5), 0x0032, "KC_NUHS = # on layer 1");
        assert_eq!(km.get(3, 2, 3), 0x7847, "a raw RGB keycode passes through as hex");
        assert_eq!(km.get(1, 0, 0), 0x0001, "KC_TRNS");
        assert_eq!(km.get(1, 8, 5), 0x0000, "KC_NO");
    }
}
