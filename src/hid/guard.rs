//! The read-only guard: the only path from this program to the keyboard.

use std::io;
use std::time::Duration;

use crate::hid::transport::Transport;
use crate::protocol::*;

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error("read-only guard refused command {command:#04x} {sub:#04x} (this is a bug)")]
    Refused { command: u8, sub: u8 },
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// True only for commands this app may send. Everything else is refused,
/// including every keymap, macro, lighting, EEPROM and bootloader command.
pub fn is_allowed(report: &Report) -> bool {
    match report[0] {
        VIA_GET_PROTOCOL_VERSION | VIA_GET_KEYBOARD_VALUE | VIA_GET_LAYER_COUNT | VIA_GET_BUFFER => true,
        VIAL_PREFIX => matches!(
            report[1],
            VIAL_GET_KEYBOARD_ID
                | VIAL_GET_SIZE
                | VIAL_GET_DEF
                | VIAL_GET_UNLOCK_STATUS
                // The one deliberate exception: Vial's unlock handshake. It changes no keymap data.
                | VIAL_UNLOCK_START
                | VIAL_UNLOCK_POLL
        ),
        _ => false,
    }
}

pub struct ReadOnlyGuard<T: Transport> {
    inner: T,
}

impl<T: Transport> ReadOnlyGuard<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn send(&mut self, report: &Report) -> Result<(), GuardError> {
        if !is_allowed(report) {
            log::error!("read-only guard refused {:02x?}", &report[..2]);
            return Err(GuardError::Refused { command: report[0], sub: report[1] });
        }
        self.inner.write_report(report)?;
        Ok(())
    }

    pub fn recv(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        self.inner.read_report(timeout)
    }
}

impl<T: Transport + 'static> ReadOnlyGuard<T> {
    pub fn boxed(self) -> ReadOnlyGuard<Box<dyn Transport>> {
        ReadOnlyGuard { inner: Box::new(self.inner) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::fake::FakeTransport;

    #[test]
    fn exactly_the_allowlist_passes() {
        let mut passed = Vec::new();
        for cmd in 0..=255u8 {
            for sub in 0..=255u8 {
                if is_allowed(&report(&[cmd, sub])) {
                    passed.push((cmd, sub));
                }
            }
        }
        let mut expected = Vec::new();
        for cmd in [0x01u8, 0x02, 0x11, 0x12] {
            for sub in 0..=255u8 {
                expected.push((cmd, sub));
            }
        }
        for sub in [0x00u8, 0x01, 0x02, 0x05, 0x06, 0x07] {
            expected.push((0xFE, sub));
        }
        expected.sort();
        assert_eq!(passed, expected);
    }

    #[test]
    fn known_write_commands_are_refused() {
        // Values from vial-qmk via.h / vial.h, written out so this test does not trust our constants.
        let writes: [[u8; 2]; 15] = [
            [0x03, 0x00], // set_keyboard_value
            [0x05, 0x00], // dynamic_keymap_set_keycode
            [0x06, 0x00], // dynamic_keymap_reset
            [0x07, 0x00], // lighting/custom set_value
            [0x09, 0x00], // lighting/custom save
            [0x0A, 0x00], // eeprom_reset
            [0x0B, 0x00], // bootloader_jump
            [0x0F, 0x00], // macro_set_buffer
            [0x10, 0x00], // macro_reset
            [0x13, 0x00], // dynamic_keymap_set_buffer
            [0xFE, 0x04], // vial_set_encoder
            [0xFE, 0x08], // vial_lock
            [0xFE, 0x0B], // vial_qmk_settings_set
            [0xFE, 0x0C], // vial_qmk_settings_reset
            [0xFE, 0x0D], // vial_dynamic_entry_op
        ];
        for bytes in writes {
            assert!(!is_allowed(&report(&bytes)), "{bytes:02x?} must be refused");
        }
    }

    #[test]
    fn refused_report_never_reaches_transport() {
        let fake = FakeTransport::new(|_| vec![]);
        let written = fake.written();
        let mut guard = ReadOnlyGuard::new(fake);
        let err = guard.send(&report(&[0x05, 0, 0, 0, 0x00, 0x04])).unwrap_err();
        assert!(matches!(err, GuardError::Refused { command: 0x05, sub: 0 }));
        assert!(written.lock().unwrap().is_empty());
    }

    #[test]
    fn allowed_report_is_forwarded_and_reply_readable() {
        let fake = FakeTransport::new(|req| vec![*req]);
        let written = fake.written();
        let mut guard = ReadOnlyGuard::new(fake);
        guard.send(&report(&[VIA_GET_LAYER_COUNT])).unwrap();
        assert_eq!(written.lock().unwrap().len(), 1);
        let reply = guard.recv(std::time::Duration::ZERO).unwrap().unwrap();
        assert_eq!(reply[0], VIA_GET_LAYER_COUNT);
    }
}
