//! Characters the host OS produces for HID usages, for the layouts we support.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostLayout {
    #[default]
    Gb,
    Us,
}

const US_SHIFTED_DIGITS: [char; 10] = ['!', '@', '#', '$', '%', '^', '&', '*', '(', ')'];
const GB_SHIFTED_DIGITS: [char; 10] = ['!', '"', '£', '$', '%', '^', '&', '*', '(', ')'];

impl HostLayout {
    /// Character for a HID keyboard usage with/without Shift; `None` for non-printing keys.
    pub fn char_for(self, usage: u8, shift: bool) -> Option<char> {
        use HostLayout::{Gb, Us};
        let (plain, shifted) = match usage {
            0x04..=0x1D => {
                let c = (b'a' + (usage - 0x04)) as char;
                (c, c.to_ascii_uppercase())
            }
            0x1E..=0x27 => {
                let i = (usage - 0x1E) as usize;
                let shifted = match self {
                    Us => US_SHIFTED_DIGITS[i],
                    Gb => GB_SHIFTED_DIGITS[i],
                };
                (b"1234567890"[i] as char, shifted)
            }
            0x2C => (' ', ' '),
            0x2D => ('-', '_'),
            0x2E => ('=', '+'),
            0x2F => ('[', '{'),
            0x30 => (']', '}'),
            // The kernel maps both KC_BSLS and KC_NUHS to KEY_BACKSLASH.
            0x31 | 0x32 => match self {
                Us => ('\\', '|'),
                Gb => ('#', '~'),
            },
            0x33 => (';', ':'),
            0x34 => match self {
                Us => ('\'', '"'),
                Gb => ('\'', '@'),
            },
            0x35 => match self {
                Us => ('`', '~'),
                Gb => ('`', '¬'),
            },
            0x36 => (',', '<'),
            0x37 => ('.', '>'),
            0x38 => ('/', '?'),
            0x54 => ('/', '/'),
            0x55 => ('*', '*'),
            0x56 => ('-', '-'),
            0x57 => ('+', '+'),
            0x59..=0x61 => {
                let c = (b'1' + (usage - 0x59)) as char;
                (c, c)
            }
            0x62 => ('0', '0'),
            0x63 => ('.', '.'),
            0x64 => match self {
                Us => ('<', '>'),
                Gb => ('\\', '|'),
            },
            0x67 => ('=', '='),
            _ => return None,
        };
        Some(if shift { shifted } else { plain })
    }

    /// Every HID usage and Shift state that types `c` on this layout, ascending by usage.
    ///
    /// A list rather than one answer, because a character can have several usages and a keymap
    /// may carry only some of them: on GB both `KC_BSLS` and `KC_NUHS` type `#`, and `/` exists
    /// on the main block and the keypad. Callers pick whichever their keymap actually has.
    pub fn usages_for(self, c: char) -> Vec<(u8, bool)> {
        let mut out = Vec::new();
        for usage in 0x04..=0x67u8 {
            let plain = self.char_for(usage, false);
            if plain == Some(c) {
                out.push((usage, false));
            } else if self.char_for(usage, true) == Some(c) {
                out.push((usage, true));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uk_layout() {
        let gb = HostLayout::Gb;
        assert_eq!(gb.char_for(0x04, false), Some('a'));
        assert_eq!(gb.char_for(0x04, true), Some('A'));
        assert_eq!(gb.char_for(0x1F, true), Some('"'));
        assert_eq!(gb.char_for(0x20, true), Some('£'));
        assert_eq!(gb.char_for(0x34, true), Some('@'));
        assert_eq!(gb.char_for(0x32, false), Some('#'));
        assert_eq!(gb.char_for(0x31, true), Some('~'));
        assert_eq!(gb.char_for(0x35, true), Some('¬'));
        assert_eq!(gb.char_for(0x64, false), Some('\\'));
        assert_eq!(gb.char_for(0x28, false), None); // Enter
    }

    #[test]
    fn us_layout() {
        let us = HostLayout::Us;
        assert_eq!(us.char_for(0x1F, true), Some('@'));
        assert_eq!(us.char_for(0x34, true), Some('"'));
        assert_eq!(us.char_for(0x31, false), Some('\\'));
        assert_eq!(us.char_for(0x2C, false), Some(' '));
    }

    #[test]
    fn default_is_gb() {
        assert_eq!(HostLayout::default(), HostLayout::Gb);
    }

    /// GB maps both `KC_BSLS` (0x31) and `KC_NUHS` (0x32) to `#`, and a keymap may carry only
    /// one of them — the reference board has no `KC_BSLS` anywhere. Returning a single answer
    /// would declare `#` untypeable and silently strip every heading from the Markdown drill.
    #[test]
    fn a_character_can_have_more_than_one_usage() {
        assert_eq!(HostLayout::Gb.usages_for('#'), vec![(0x31, false), (0x32, false)]);
        assert_eq!(HostLayout::Us.usages_for('*'), vec![(0x25, true), (0x55, false)], "Shift+8 and the keypad");
    }

    #[test]
    fn ordinary_characters_resolve_to_one_usage() {
        let gb = HostLayout::Gb;
        assert_eq!(gb.usages_for('a'), vec![(0x04, false)]);
        assert_eq!(gb.usages_for('A'), vec![(0x04, true)]);
        assert_eq!(gb.usages_for('£'), vec![(0x20, true)]);
        assert_eq!(gb.usages_for('@'), vec![(0x34, true)]);
        assert_eq!(gb.usages_for(' '), vec![(0x2C, false)], "the shifted duplicate is dropped");
        assert_eq!(gb.usages_for('\\'), vec![(0x64, false)], "KC_NUBS on GB; KC_BSLS types # there");
        assert_eq!(gb.usages_for('€'), vec![], "not on either layout");
    }

    /// The main keyboard usage comes before its keypad duplicate, so a hint points at the key
    /// a Lily58 actually has.
    #[test]
    fn the_main_usage_comes_before_the_keypad_duplicate() {
        assert_eq!(HostLayout::Gb.usages_for('/'), vec![(0x38, false), (0x54, false)]);
    }

    /// Whatever `char_for` produces must resolve back to that character. Not necessarily to the
    /// same usage: the keypad duplicates make that a deliberately weaker claim.
    #[test]
    fn every_character_round_trips() {
        for layout in [HostLayout::Gb, HostLayout::Us] {
            for usage in 0x04..=0x67u8 {
                for shift in [false, true] {
                    let Some(c) = layout.char_for(usage, shift) else { continue };
                    let found = layout.usages_for(c);
                    assert!(!found.is_empty(), "{layout:?} {usage:#04x} shift={shift} -> {c:?} resolves to nothing");
                    for (u, s) in found {
                        assert_eq!(layout.char_for(u, s), Some(c), "{layout:?} {u:#04x} shift={s}");
                    }
                }
            }
        }
    }
}
