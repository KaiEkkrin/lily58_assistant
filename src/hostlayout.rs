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
}
