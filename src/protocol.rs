//! VIA / Vial raw-HID command bytes (vial-qmk `quantum/via.h`, `quantum/vial.h`).
//!
//! Only commands this app is allowed to send are named here. Write commands are
//! deliberately absent: see `hid::guard`.

pub const REPORT_LEN: usize = 32;
pub type Report = [u8; REPORT_LEN];

// VIA commands (first byte). These replies echo the request bytes.
pub const VIA_GET_PROTOCOL_VERSION: u8 = 0x01;
pub const VIA_GET_KEYBOARD_VALUE: u8 = 0x02;
pub const VIA_GET_LAYER_COUNT: u8 = 0x11;
pub const VIA_GET_BUFFER: u8 = 0x12;
pub const VIAL_PREFIX: u8 = 0xFE;

/// `id_get_keyboard_value` sub-command: switch matrix state.
pub const VIA_SWITCH_MATRIX_STATE: u8 = 0x03;

// Vial sub-commands (second byte after VIAL_PREFIX). Replies overwrite the buffer.
pub const VIAL_GET_KEYBOARD_ID: u8 = 0x00;
pub const VIAL_GET_SIZE: u8 = 0x01;
pub const VIAL_GET_DEF: u8 = 0x02;
pub const VIAL_GET_UNLOCK_STATUS: u8 = 0x05;
pub const VIAL_UNLOCK_START: u8 = 0x06;
pub const VIAL_UNLOCK_POLL: u8 = 0x07;

/// Largest keymap chunk per `VIA_GET_BUFFER` request (firmware accepts size <= 28).
pub const BUFFER_CHUNK: usize = 28;
/// vial-qmk's `VIAL_UNLOCK_COUNTER_MAX`.
pub const UNLOCK_COUNTER_MAX: u8 = 50;
/// First Vial protocol version that uses QMK's current keycode numbering.
pub const MIN_VIAL_PROTOCOL: u32 = 6;

/// A zero-padded report starting with `bytes`.
pub fn report(bytes: &[u8]) -> Report {
    let mut r = [0u8; REPORT_LEN];
    r[..bytes.len()].copy_from_slice(bytes);
    r
}
