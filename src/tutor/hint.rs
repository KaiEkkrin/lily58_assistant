//! Where a character lives on this keymap, and what to hold to reach it.

use crate::hostlayout::HostLayout;
use crate::keycodes::{self, Action};
use crate::keymap::Keymap;
use crate::tutor::fingers;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPath {
    /// The key that emits the character.
    pub key: (u8, u8),
    /// Keys to hold first: a layer key, a Shift key, or neither.
    pub hold: Vec<(u8, u8)>,
}

/// The cheapest way to type `c` on this keymap, or `None` if it can't be typed.
///
/// Cheapest means fewest keys held, tie-broken by the lower layer and then matrix order. Every
/// candidate is enumerated rather than taking the first match, because a preference only exists
/// if you can see them all: on the reference keymap `{` ties at one hold between Shift plus
/// layer 0's `[` and layer 1's dedicated `LSFT(KC_LBRC)`, and the lower layer wins over the
/// dedicated key. (`Keymap::find_position` still serves the OS-key inference it was written for;
/// it stops at the first hit and knows nothing about Shift.)
pub fn resolve(keymap: &Keymap, host: HostLayout, c: char) -> Option<KeyPath> {
    let usages = host.usages_for(c);
    if usages.is_empty() {
        return None;
    }
    let mut best: Option<(usize, u8, u8, u8, KeyPath)> = None;
    for layer in 0..keymap.layers() {
        for row in 0..keymap.rows() {
            for col in 0..keymap.cols() {
                let code = keymap.get(layer, row, col);
                let Some(basic) = keycodes::tap_basic(code) else { continue };
                let Some(&(_, shift)) = usages.iter().find(|&&(usage, _)| usage == basic) else { continue };
                // A keycode that carries Shift can only serve a character that wants Shift.
                // `LSFT(KC_EQL)` types `+`, never `=`, however well its usage matches — and on
                // this keymap it sits one layer *below* the real `=`, so without this it won the
                // lower-layer tie-break and the hint named LOWER for a RAISE-only character.
                if !shift && keycodes::adds_shift(code) {
                    continue;
                }
                let mut hold = Vec::new();
                if layer != 0 {
                    let Some(key) = layer_key(keymap, layer) else { continue };
                    hold.push(key);
                }
                if shift && !keycodes::adds_shift(code) {
                    let Some(key) = shift_key(keymap, (row, col)) else { continue };
                    hold.push(key);
                }
                let rank = (hold.len(), layer, row, col);
                if best.as_ref().is_none_or(|b| rank < (b.0, b.1, b.2, b.3)) {
                    best = Some((rank.0, rank.1, rank.2, rank.3, KeyPath { key: (row, col), hold }));
                }
            }
        }
    }
    best.map(|(.., path)| path)
}

/// A key on **layer 0** that turns `layer` on while it is held, preferring hold-to-use kinds.
///
/// Layer 0 only, deliberately. On the reference keymap `MO(3)` exists only on layers 1 and 2, so
/// searching every layer would name a key that does nothing from the base layer. Anything deeper
/// than one layer key from base is treated as unreachable, which fails closed.
fn layer_key(keymap: &Keymap, layer: u8) -> Option<(u8, u8)> {
    let rank = |code: u16| match keycodes::decode(code) {
        Action::Momentary(l) | Action::LayerTap { layer: l, .. } if l == layer => Some(0u8),
        Action::LayerMod { layer: l, .. } if l == layer => Some(1),
        Action::OneShotLayer(l) if l == layer => Some(2),
        Action::Toggle(l) | Action::TapToggle(l) | Action::To(l) if l == layer => Some(3),
        _ => None,
    };
    let mut best: Option<(u8, (u8, u8))> = None;
    for row in 0..keymap.rows() {
        for col in 0..keymap.cols() {
            if let Some(r) = rank(keymap.get(0, row, col))
                && best.is_none_or(|(br, _)| r < br)
            {
                best = Some((r, (row, col)));
            }
        }
    }
    best.map(|(_, key)| key)
}

/// A Shift key, preferring one on the opposite hand to `target` — the way you'd actually type it.
fn shift_key(keymap: &Keymap, target: (u8, u8)) -> Option<(u8, u8)> {
    let target_hand = fingers::spot(target.0, target.1).map(|s| s.hand);
    let mut fallback = None;
    for row in 0..keymap.rows() {
        for col in 0..keymap.cols() {
            let shifts = match keycodes::decode(keymap.get(0, row, col)) {
                Action::Basic(b) => b == 0xE1 || b == 0xE5,
                Action::ModTap { mods, .. } => mods & 0x02 != 0,
                _ => false,
            };
            if !shifts {
                continue;
            }
            let hand = fingers::spot(row, col).map(|s| s.hand);
            if hand.is_some() && hand != target_hand {
                return Some((row, col));
            }
            fallback.get_or_insert((row, col));
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycodes::{KC_NO, KC_TRNS};
    use crate::tutor::fixture::reference_keymap;

    fn path(c: char) -> KeyPath {
        resolve(&reference_keymap(), HostLayout::Gb, c).unwrap_or_else(|| panic!("{c:?} should resolve"))
    }

    #[test]
    fn equal_cost_paths_prefer_the_lower_layer_even_against_a_dedicated_key() {
        // `{` has three routes here. Shift plus layer 0's `[` at (4, 0) and layer 1's dedicated
        // LSFT(KC_LBRC) at (8, 2) holding MO(1) both cost one hold, so the lower layer wins.
        // Layer 2's `[` needs MO(2) and Shift, which is two.
        assert_eq!(path('{'), KeyPath { key: (4, 0), hold: vec![(8, 0)] });
    }

    /// `#` is only reachable as KC_NUHS on layer 1. This is the case that needs `usages_for` to
    /// return a list: the reference keymap has no KC_BSLS, which is the other GB usage for `#`.
    #[test]
    fn a_character_with_two_usages_finds_the_one_this_keymap_has() {
        assert_eq!(path('#'), KeyPath { key: (2, 5), hold: vec![(4, 2)] });
    }

    /// `!` is LSFT(KC_1) on layer 1 and plain KC_1 on layer 0. Both cost one hold, so the lower
    /// layer wins and the drill teaches Shift+1.
    #[test]
    fn equal_cost_paths_prefer_the_lower_layer() {
        assert_eq!(path('!'), KeyPath { key: (0, 4), hold: vec![(8, 0)] });
    }

    #[test]
    fn shift_comes_from_the_opposite_hand() {
        assert_eq!(path('A'), KeyPath { key: (2, 4), hold: vec![(8, 0)] }, "left-hand A, right Shift");
        assert_eq!(path('?'), KeyPath { key: (8, 1), hold: vec![(3, 5)] }, "right-hand /, left Shift");
    }

    #[test]
    fn unshifted_base_layer_keys_need_no_holds() {
        assert_eq!(path(' '), KeyPath { key: (4, 1), hold: vec![] });
        assert_eq!(path('['), KeyPath { key: (4, 0), hold: vec![] });
        assert_eq!(path('g'), KeyPath { key: (2, 0), hold: vec![] });
    }

    /// `=` is `KC_EQL` on layer 2 at (8, 3). Layer 1 has `LSFT(KC_EQL)` at the very same position,
    /// which types `+` — but it matches the same HID usage, so it used to win the lower-layer
    /// tie-break and the hint named LOWER for a character only RAISE can type. `=` and `+` came
    /// out with identical paths, which is impossible: one key plus one hold types one character.
    #[test]
    fn a_keycode_that_carries_shift_cannot_serve_an_unshifted_character() {
        assert_eq!(path('='), KeyPath { key: (8, 3), hold: vec![(9, 3)] }, "RAISE holds MO(2) at (9,3)");
        assert_eq!(path('+'), KeyPath { key: (8, 3), hold: vec![(4, 2)] }, "LOWER's pre-shifted key is right for +");
        assert_eq!(path('\\'), KeyPath { key: (8, 0), hold: vec![(9, 3)] });
        assert_eq!(path('|'), KeyPath { key: (8, 0), hold: vec![(4, 2)] });
    }

    /// Follow every hint literally — hold what it says, press what it says — and check the board
    /// emits the character that was asked for. A whole-keymap oracle rather than another
    /// hand-written expectation, and that is the point: it caught `=` and `\` without being told
    /// where to look, while the tests above, each pinning one character, passed straight over them.
    #[test]
    fn every_hint_types_the_character_it_was_asked_for() {
        let km = reference_keymap();
        let mut chars: Vec<char> = (0x20u8..0x7f).map(|b| b as char).collect();
        chars.extend(['£', '¬', '¦', '€']);
        let mut wrong: Vec<String> = Vec::new();
        for c in chars {
            let Some(path) = resolve(&km, HostLayout::Gb, c) else { continue };
            // What holding those keys down actually does, from the base layer.
            let (mut layer, mut shift) = (0u8, false);
            for &(row, col) in &path.hold {
                match keycodes::decode(km.get(0, row, col)) {
                    Action::Momentary(l) | Action::LayerTap { layer: l, .. } | Action::LayerMod { layer: l, .. } => layer = l,
                    Action::Basic(b) if b == 0xE1 || b == 0xE5 => shift = true,
                    Action::ModTap { mods, .. } if mods & 0x02 != 0 => shift = true,
                    other => panic!("{c:?}: told to hold ({row},{col}), which is {other:?} — not a layer key or Shift"),
                }
            }
            let code = km.get(layer, path.key.0, path.key.1);
            let typed = keycodes::tap_basic(code).and_then(|b| HostLayout::Gb.char_for(b, shift || keycodes::adds_shift(code)));
            if typed != Some(c) {
                wrong.push(format!("{c:?}: hold {:?} then {:?} types {typed:?}", path.hold, path.key));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("; "));
    }

    #[test]
    fn characters_this_keyboard_cannot_type_resolve_to_nothing() {
        assert_eq!(resolve(&reference_keymap(), HostLayout::Gb, '€'), None);
    }

    /// Layer keys are looked for on layer 0 only. On the reference keymap `MO(3)` exists solely
    /// on layers 1 and 2, so a wider search would name a key that does nothing from the base
    /// layer and hand out an impossible hint.
    #[test]
    fn characters_behind_a_second_layer_key_are_unreachable() {
        // 4 layers, 1 row, 2 cols. Layer 0: MO(1), KC_NO. Layer 1: TRNS, MO(3). Layer 3: TRNS, KC_A.
        let codes: [u16; 8] = [0x5221, KC_NO, KC_TRNS, 0x5223, KC_NO, KC_NO, KC_TRNS, 0x0004];
        let buf: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
        let km = Keymap::from_buffer(4, 1, 2, &buf).unwrap();
        assert_eq!(resolve(&km, HostLayout::Gb, 'a'), None);
    }
}
