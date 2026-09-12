//! Conversions between HID keyboard usages, Linux evdev keycodes and egui keys.

use eframe::egui::Key;

/// Linux's HID keyboard-page usage → evdev keycode table
/// (`drivers/hid/hid-input.c`, `hid_keyboard[]`); 0 = unmapped.
#[rustfmt::skip]
const HID_TO_EVDEV: [u8; 256] = [
      0,  0,  0,  0, 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38,
     50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,  2,  3,
      4,  5,  6,  7,  8,  9, 10, 11, 28,  1, 14, 15, 57, 12, 13, 26,
     27, 43, 43, 39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64,
     65, 66, 67, 68, 87, 88, 99, 70,119,110,102,104,111,107,109,106,
    105,108,103, 69, 98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71,
     72, 73, 82, 83, 86,127,116,117,183,184,185,186,187,188,189,190,
    191,192,193,194,134,138,130,132,128,129,131,137,133,135,136,113,
    115,114,  0,  0,  0,121,  0, 89, 93,124, 92, 94, 95,  0,  0,  0,
    122,123, 90, 91, 85,  0,  0,  0,  0,  0,  0,  0,111,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,179,180,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,
      0,  0,  0,  0,  0,  0,  0,  0,111,  0,  0,  0,  0,  0,  0,  0,
     29, 42, 56,125, 97, 54,100,126,164,166,165,163,161,115,114,113,
    150,158,159,128,136,177,178,176,142,152,173,140,  0,  0,  0,  0,
];

pub fn hid_to_evdev(usage: u8) -> Option<u16> {
    match HID_TO_EVDEV[usage as usize] {
        0 => None,
        code => Some(code as u16),
    }
}

/// Keyboard-page usages QMK can send that the kernel maps to `code`.
/// QMK's 0xA5..=0xDF are its own media/mouse codes, not keyboard usages, so they are excluded.
pub fn evdev_to_hid(code: u16) -> Vec<u8> {
    if code == 0 {
        return Vec::new();
    }
    (0x04..=0xA4u8).chain(0xE0..=0xE7).filter(|&u| HID_TO_EVDEV[u as usize] as u16 == code).collect()
}

/// HID usages for an egui (physical) key. `Backslash` is ambiguous on ISO boards.
#[rustfmt::skip]
pub fn egui_key_to_hid(key: Key) -> Vec<u8> {
    let single = |u: u8| vec![u];
    match key {
        Key::A => single(0x04), Key::B => single(0x05), Key::C => single(0x06), Key::D => single(0x07),
        Key::E => single(0x08), Key::F => single(0x09), Key::G => single(0x0A), Key::H => single(0x0B),
        Key::I => single(0x0C), Key::J => single(0x0D), Key::K => single(0x0E), Key::L => single(0x0F),
        Key::M => single(0x10), Key::N => single(0x11), Key::O => single(0x12), Key::P => single(0x13),
        Key::Q => single(0x14), Key::R => single(0x15), Key::S => single(0x16), Key::T => single(0x17),
        Key::U => single(0x18), Key::V => single(0x19), Key::W => single(0x1A), Key::X => single(0x1B),
        Key::Y => single(0x1C), Key::Z => single(0x1D),
        Key::Num1 => single(0x1E), Key::Num2 => single(0x1F), Key::Num3 => single(0x20), Key::Num4 => single(0x21),
        Key::Num5 => single(0x22), Key::Num6 => single(0x23), Key::Num7 => single(0x24), Key::Num8 => single(0x25),
        Key::Num9 => single(0x26), Key::Num0 => single(0x27),
        Key::Enter => single(0x28), Key::Escape => single(0x29), Key::Backspace => single(0x2A),
        Key::Tab => single(0x2B), Key::Space => single(0x2C), Key::Minus => single(0x2D),
        Key::Equals => single(0x2E), Key::OpenBracket => single(0x2F), Key::CloseBracket => single(0x30),
        Key::Backslash => vec![0x31, 0x32], Key::Semicolon => single(0x33), Key::Quote => single(0x34),
        Key::Backtick => single(0x35), Key::Comma => single(0x36), Key::Period => single(0x37),
        Key::Slash => single(0x38),
        Key::F1 => single(0x3A), Key::F2 => single(0x3B), Key::F3 => single(0x3C), Key::F4 => single(0x3D),
        Key::F5 => single(0x3E), Key::F6 => single(0x3F), Key::F7 => single(0x40), Key::F8 => single(0x41),
        Key::F9 => single(0x42), Key::F10 => single(0x43), Key::F11 => single(0x44), Key::F12 => single(0x45),
        Key::Insert => single(0x49), Key::Home => single(0x4A), Key::PageUp => single(0x4B),
        Key::Delete => single(0x4C), Key::End => single(0x4D), Key::PageDown => single(0x4E),
        Key::ArrowRight => single(0x4F), Key::ArrowLeft => single(0x50), Key::ArrowDown => single(0x51),
        Key::ArrowUp => single(0x52), Key::IntlBackslash => single(0x64),
        Key::ControlLeft => single(0xE0), Key::ShiftLeft => single(0xE1), Key::AltLeft => single(0xE2),
        Key::SuperLeft => single(0xE3), Key::ControlRight => single(0xE4), Key::ShiftRight => single(0xE5),
        Key::AltRight => single(0xE6), Key::SuperRight => single(0xE7),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_table_spot_checks() {
        assert_eq!(hid_to_evdev(0x04), Some(30)); // KEY_A
        assert_eq!(hid_to_evdev(0x29), Some(1)); // KEY_ESC
        assert_eq!(hid_to_evdev(0x64), Some(86)); // KEY_102ND
        assert_eq!(hid_to_evdev(0xE1), Some(42)); // KEY_LEFTSHIFT
        assert_eq!(hid_to_evdev(0x00), None);
    }

    #[test]
    fn reverse_lookup_returns_all_candidates() {
        assert_eq!(evdev_to_hid(30), vec![0x04]);
        assert_eq!(evdev_to_hid(43), vec![0x31, 0x32]); // backslash and ISO #
        assert_eq!(evdev_to_hid(111), vec![0x4C, 0x9C]); // DELETE; 0xD8 is a QMK mouse key, excluded
        assert!(evdev_to_hid(0).is_empty());
    }

    #[test]
    fn egui_keys() {
        assert_eq!(egui_key_to_hid(Key::A), vec![0x04]);
        assert_eq!(egui_key_to_hid(Key::Num0), vec![0x27]);
        assert_eq!(egui_key_to_hid(Key::Backslash), vec![0x31, 0x32]);
        assert_eq!(egui_key_to_hid(Key::IntlBackslash), vec![0x64]);
        assert_eq!(egui_key_to_hid(Key::ShiftLeft), vec![0xE1]);
        assert!(egui_key_to_hid(Key::Copy).is_empty());
    }
}
