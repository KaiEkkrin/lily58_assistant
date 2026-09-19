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
/// Cheapest means, in order:
///
/// 1. **Fewest keys held.**
/// 2. **A layer key over Shift.** A layer key is a thumb and Shift is a pinky; pinky stretches
///    pull the hand out of position, so `{` is LOWER + `.` rather than Shift + `[`.
/// 3. **One finger from each hand over two of one hand.** Ordinary typing advice, and it decides
///    between two layer chords: whichever thumb leaves the pressing hand alone.
/// 4. **The lower layer, then matrix order** — nothing left to prefer, so this is for
///    determinism.
///
/// All four read the live keymap: 2 and 3 come from `keycodes::decode` and `fingers::spot` at
/// resolve time, so a remap in Vial changes the answer with no code change. Note that 2 outranks
/// 3 — `!` goes to LOWER + `a`, one-handed but at rest, rather than Shift + `1`, which uses both
/// hands but stretches the left pinky to the number row while the right holds Shift.
///
/// Every candidate is enumerated rather than taking the first match, because a preference only
/// exists if you can see them all. (`Keymap::find_position` still serves the OS-key inference it
/// was written for; it stops at the first hit and knows nothing about Shift.)
pub fn resolve(keymap: &Keymap, host: HostLayout, c: char) -> Option<KeyPath> {
    let usages = host.usages_for(c);
    if usages.is_empty() {
        return None;
    }
    let mut best: Option<(Rank, KeyPath)> = None;
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
                let needs_shift_key = shift && !keycodes::adds_shift(code);
                if needs_shift_key {
                    let Some(key) = shift_key(keymap, (row, col)) else { continue };
                    hold.push(key);
                }
                let rank = (hold.len(), needs_shift_key, one_handed((row, col), &hold), layer, row, col);
                if best.as_ref().is_none_or(|(b, _)| rank < *b) {
                    best = Some((rank, KeyPath { key: (row, col), hold }));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

/// Held keys, then Shift over a layer key, then one hand over two, then the lower layer and
/// matrix order. Ascending on every term, which is why the two preferences are stored as the
/// thing to avoid: `false` sorts before `true`.
type Rank = (usize, bool, bool, u8, u8, u8);

/// True when every key you hold is on the same hand as the key you press. Two fingers of one hand
/// is the more awkward chord, so this loses a tie. A chord of one key is not one-handed in the
/// sense that matters, and a position with no finger can't be judged.
fn one_handed(key: (u8, u8), hold: &[(u8, u8)]) -> bool {
    let hand = fingers::spot(key.0, key.1).map(|s| s.hand);
    hand.is_some() && !hold.is_empty() && hold.iter().all(|&(r, c)| fingers::spot(r, c).map(|s| s.hand) == hand)
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

    /// A thumb on a layer key beats a pinky on Shift when both cost one hold. `{` has three
    /// routes: Shift plus layer 0's `[` at (4, 0), layer 1's dedicated `LSFT(KC_LBRC)` at (8, 2)
    /// holding `MO(1)`, and layer 2's `[` plus Shift, which costs two. Of the two one-hold
    /// routes, the layer key is a thumb and Shift is a pinky, so the dedicated key wins.
    #[test]
    fn a_thumb_on_a_layer_key_beats_a_pinky_on_shift() {
        assert_eq!(path('{'), KeyPath { key: (8, 2), hold: vec![(4, 2)] });
        assert_eq!(path('}'), KeyPath { key: (8, 1), hold: vec![(4, 2)] });
    }

    /// `#` is only reachable as KC_NUHS on layer 1. This is the case that needs `usages_for` to
    /// return a list: the reference keymap has no KC_BSLS, which is the other GB usage for `#`.
    #[test]
    fn a_character_with_two_usages_finds_the_one_this_keymap_has() {
        assert_eq!(path('#'), KeyPath { key: (2, 5), hold: vec![(4, 2)] });
    }

    /// Avoiding the pinky outranks using both hands. `!` is `LSFT(KC_1)` on layer 1 at (2, 4) —
    /// the `a` key — and plain `KC_1` on layer 0 at (0, 4). Both cost one hold. The Shift route
    /// uses both hands but stretches the left pinky up to the number row while the right pinky
    /// holds Shift; the layer route is the left thumb plus the left pinky *at rest*. One hand,
    /// no stretch, and that is the more comfortable chord on this board.
    #[test]
    fn avoiding_the_pinky_outranks_using_both_hands() {
        assert_eq!(path('!'), KeyPath { key: (2, 4), hold: vec![(4, 2)] });
        assert_eq!(path('£'), KeyPath { key: (2, 2), hold: vec![(4, 2)] }, "the same shape one finger over");
    }

    /// Between two layer chords, one finger from each hand beats two fingers of one hand. No
    /// character on the reference keymap distinguishes the two rules — `+`'s cross-hand route is
    /// also the lower-layer one — so this builds the case: `a` sits on layer 1 under a left-hand
    /// key (same hand as `MO(1)`'s left thumb) and on layer 2 under another left-hand key (the
    /// opposite hand from `MO(2)`'s right thumb). The lower layer alone would pick the one-handed
    /// chord.
    #[test]
    fn between_layer_chords_one_finger_per_hand_wins() {
        let mut codes = [KC_NO; 3 * 10 * 6];
        let at = |layer: usize, row: usize, col: usize| (layer * 10 + row) * 6 + col;
        codes[at(0, 4, 2)] = 0x5221; // MO(1) on a left thumb
        codes[at(0, 9, 3)] = 0x5222; // MO(2) on a right thumb
        codes[at(1, 2, 4)] = 0x0004; // KC_A on a left-hand key: one-handed with MO(1)
        codes[at(2, 2, 3)] = 0x0004; // KC_A on a left-hand key: crosses hands with MO(2)
        let buf: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
        let km = Keymap::from_buffer(3, 10, 6, &buf).unwrap();
        assert_eq!(resolve(&km, HostLayout::Gb, 'a'), Some(KeyPath { key: (2, 3), hold: vec![(9, 3)] }));
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

    /// The reference keymap with one layer-0 key moved elsewhere.
    fn with_layer_key_moved(from: (u8, u8), to: (u8, u8)) -> Keymap {
        let km = reference_keymap();
        let mut buf = Vec::new();
        for layer in 0..km.layers() {
            for row in 0..km.rows() {
                for col in 0..km.cols() {
                    let code = match (layer, (row, col)) {
                        (0, at) if at == to => km.get(0, from.0, from.1),
                        (0, at) if at == from => KC_NO,
                        _ => km.get(layer, row, col),
                    };
                    buf.extend_from_slice(&code.to_be_bytes());
                }
            }
        }
        Keymap::from_buffer(km.layers(), km.rows(), km.cols(), &buf).unwrap()
    }

    /// None of this keymap is compiled in. Move `MO(1)` from the left thumb to the right half and
    /// the hint for `{` names the thumb it moved to — the preferences are computed from the keymap
    /// at resolve time, so a remap in Vial changes the answer without changing any code. The chord
    /// is one-handed now, which rule 3 dislikes, but rule 2 outranks it and Shift is still a pinky.
    #[test]
    fn hints_follow_a_remapped_layer_key() {
        let km = with_layer_key_moved((4, 2), (9, 2));
        assert_eq!(resolve(&km, HostLayout::Gb, '{'), Some(KeyPath { key: (8, 2), hold: vec![(9, 2)] }));
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
