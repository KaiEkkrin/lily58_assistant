//! Typed Vial/VIA queries, all sent through the read-only guard.

use std::io;
use std::time::{Duration, Instant};

use crate::hid::guard::{GuardError, ReadOnlyGuard};
use crate::hid::transport::Transport;
use crate::protocol::*;

/// Upper bound on the compressed definition, to stop runaway reads on a bad size reply.
const MAX_DEFINITION_BYTES: u32 = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum VialError {
    #[error(transparent)]
    Guard(#[from] GuardError),
    #[error("keyboard I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("no reply from the keyboard to command {0:#04x}")]
    Timeout(u8),
    #[error("keyboard sent an unexpected reply: {0}")]
    BadReply(String),
    #[error("Vial protocol v{found} is too old; v{min} or newer is needed", min = MIN_VIAL_PROTOCOL)]
    ProtocolTooOld { found: u32 },
}

impl VialError {
    /// I/O failure: the keyboard was most likely unplugged.
    pub fn is_disconnect(&self) -> bool {
        matches!(self, VialError::Io(_) | VialError::Guard(GuardError::Io(_)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardId {
    pub vial_protocol: u32,
    pub uid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockStatus {
    pub unlocked: bool,
    pub in_progress: bool,
    /// Matrix positions the user must hold to unlock.
    pub keys: Vec<(u8, u8)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnlockPoll {
    pub unlocked: bool,
    pub in_progress: bool,
    /// Counts down from `UNLOCK_COUNTER_MAX` while the unlock keys are held.
    pub counter: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixState {
    rows: u8,
    cols: u8,
    bits: Vec<u32>,
}

impl MatrixState {
    pub fn empty(rows: u8, cols: u8) -> Self {
        Self { rows, cols, bits: vec![0; rows as usize] }
    }

    pub fn is_pressed(&self, row: u8, col: u8) -> bool {
        row < self.rows && col < self.cols && (self.bits[row as usize] >> col) & 1 == 1
    }

    pub fn pressed(&self) -> Vec<(u8, u8)> {
        (0..self.rows)
            .flat_map(|r| (0..self.cols).map(move |c| (r, c)))
            .filter(|&(r, c)| self.is_pressed(r, c))
            .collect()
    }
}

pub fn check_protocol(id: &KeyboardId) -> Result<(), VialError> {
    if id.vial_protocol < MIN_VIAL_PROTOCOL {
        return Err(VialError::ProtocolTooOld { found: id.vial_protocol });
    }
    Ok(())
}

pub struct VialClient<T: Transport> {
    guard: ReadOnlyGuard<T>,
    timeout: Duration,
    retry_delay: Duration,
}

impl<T: Transport> VialClient<T> {
    pub fn new(guard: ReadOnlyGuard<T>) -> Self {
        Self::with_timing(guard, Duration::from_millis(500), Duration::from_millis(500))
    }

    pub fn with_timing(guard: ReadOnlyGuard<T>, timeout: Duration, retry_delay: Duration) -> Self {
        Self { guard, timeout, retry_delay }
    }

    /// Sends `req` and returns the first reply `accept` agrees is ours. Stale reports are
    /// drained first. Retries once after `retry_delay` if nothing acceptable arrives in time.
    fn request(&mut self, req: Report, accept: impl Fn(&Report) -> bool) -> Result<Report, VialError> {
        for attempt in 0..2 {
            if attempt > 0 {
                std::thread::sleep(self.retry_delay);
            }
            for _ in 0..64 {
                if self.guard.recv(Duration::ZERO)?.is_none() {
                    break;
                }
            }
            self.guard.send(&req)?;
            let deadline = Instant::now() + self.timeout;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.guard.recv(left)? {
                    Some(reply) if accept(&reply) => return Ok(reply),
                    Some(_) if Instant::now() < deadline => continue, // a reply meant for another program
                    _ => break,
                }
            }
        }
        Err(VialError::Timeout(req[0]))
    }

    pub fn via_protocol_version(&mut self) -> Result<u16, VialError> {
        let r = self.request(report(&[VIA_GET_PROTOCOL_VERSION]), |r| r[0] == VIA_GET_PROTOCOL_VERSION)?;
        Ok(u16::from_be_bytes([r[1], r[2]]))
    }

    pub fn keyboard_id(&mut self) -> Result<KeyboardId, VialError> {
        // Vial replies overwrite the buffer, so they cannot be matched to the request.
        let r = self.request(report(&[VIAL_PREFIX, VIAL_GET_KEYBOARD_ID]), |_| true)?;
        Ok(KeyboardId {
            vial_protocol: u32::from_le_bytes(r[0..4].try_into().unwrap()),
            uid: u64::from_le_bytes(r[4..12].try_into().unwrap()),
        })
    }

    /// The keyboard's layout definition (xz-compressed JSON, read in 32-byte blocks).
    pub fn definition(&mut self) -> Result<serde_json::Value, VialError> {
        let r = self.request(report(&[VIAL_PREFIX, VIAL_GET_SIZE]), |_| true)?;
        let size = u32::from_le_bytes(r[0..4].try_into().unwrap());
        if size == 0 || size > MAX_DEFINITION_BYTES {
            return Err(VialError::BadReply(format!("definition size {size}")));
        }
        let size = size as usize;
        let mut xz = Vec::with_capacity(size);
        let mut page: u16 = 0;
        while xz.len() < size {
            let [lo, hi] = page.to_le_bytes();
            let r = self.request(report(&[VIAL_PREFIX, VIAL_GET_DEF, lo, hi]), |_| true)?;
            let take = (size - xz.len()).min(REPORT_LEN);
            xz.extend_from_slice(&r[..take]);
            page += 1;
        }
        let mut json = Vec::new();
        lzma_rs::xz_decompress(&mut xz.as_slice(), &mut json)
            .map_err(|e| VialError::BadReply(format!("definition does not decompress: {e:?}")))?;
        serde_json::from_slice(&json).map_err(|e| VialError::BadReply(format!("definition is not JSON: {e}")))
    }

    pub fn layer_count(&mut self) -> Result<u8, VialError> {
        let r = self.request(report(&[VIA_GET_LAYER_COUNT]), |r| r[0] == VIA_GET_LAYER_COUNT)?;
        if r[1] == 0 {
            return Err(VialError::BadReply("keyboard reports 0 layers".into()));
        }
        Ok(r[1])
    }

    pub fn keymap_buffer(&mut self, len: usize) -> Result<Vec<u8>, VialError> {
        let mut buf = Vec::with_capacity(len);
        while buf.len() < len {
            let [hi, lo] = (buf.len() as u16).to_be_bytes();
            let size = (len - buf.len()).min(BUFFER_CHUNK) as u8;
            let req = report(&[VIA_GET_BUFFER, hi, lo, size]);
            let r = self.request(req, |r| r[..4] == req[..4])?;
            buf.extend_from_slice(&r[4..4 + size as usize]);
        }
        Ok(buf)
    }

    pub fn unlock_status(&mut self) -> Result<UnlockStatus, VialError> {
        let r = self.request(report(&[VIAL_PREFIX, VIAL_GET_UNLOCK_STATUS]), |_| true)?;
        let keys = r[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[row, col]| (row, col))
            .filter(|&(row, col)| row != 0xFF && col != 0xFF)
            .collect();
        Ok(UnlockStatus { unlocked: r[0] != 0, in_progress: r[1] != 0, keys })
    }

    /// Starts Vial's unlock handshake. Once started, the firmware ignores VIA commands
    /// until the unlock completes or the keyboard is replugged.
    pub fn unlock_start(&mut self) -> Result<(), VialError> {
        self.request(report(&[VIAL_PREFIX, VIAL_UNLOCK_START]), |_| true)?;
        Ok(())
    }

    pub fn unlock_poll(&mut self) -> Result<UnlockPoll, VialError> {
        let r = self.request(report(&[VIAL_PREFIX, VIAL_UNLOCK_POLL]), |_| true)?;
        Ok(UnlockPoll { unlocked: r[0] != 0, in_progress: r[1] != 0, counter: r[2] })
    }

    /// Only meaningful while unlocked: a locked keyboard echoes the request, which reads as
    /// "nothing pressed".
    pub fn matrix_state(&mut self, rows: u8, cols: u8) -> Result<MatrixState, VialError> {
        let row_size = (cols as usize).div_ceil(8);
        if cols == 0 || cols > 32 || 2 + rows as usize * row_size > REPORT_LEN {
            return Err(VialError::BadReply(format!("a {rows}x{cols} matrix does not fit in one report")));
        }
        let r = self.request(report(&[VIA_GET_KEYBOARD_VALUE, VIA_SWITCH_MATRIX_STATE]), |r| {
            r[0] == VIA_GET_KEYBOARD_VALUE && r[1] == VIA_SWITCH_MATRIX_STATE
        })?;
        let bits = (0..rows as usize)
            .map(|row| r[2 + row * row_size..2 + (row + 1) * row_size].iter().fold(0u32, |acc, &b| (acc << 8) | b as u32))
            .collect();
        Ok(MatrixState { rows, cols, bits })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::fake::{FakeTransport, KeyboardSim, SMALL_DEFINITION};

    fn client(t: FakeTransport) -> VialClient<FakeTransport> {
        VialClient::with_timing(ReadOnlyGuard::new(t), Duration::from_millis(5), Duration::ZERO)
    }

    #[test]
    fn reads_ids_and_versions() {
        let mut c = client(KeyboardSim::small().transport());
        assert_eq!(c.via_protocol_version().unwrap(), 0x000C);
        let id = c.keyboard_id().unwrap();
        assert_eq!(id, KeyboardId { vial_protocol: 6, uid: 0x0648_397D_5BFC_FD7E });
        assert!(check_protocol(&id).is_ok());
        assert!(matches!(
            check_protocol(&KeyboardId { vial_protocol: 5, uid: 0 }),
            Err(VialError::ProtocolTooOld { found: 5 })
        ));
    }

    #[test]
    fn reads_definition_across_blocks() {
        let sim = KeyboardSim::small();
        assert!(sim.with(|s| s.definition_xz.len()) > REPORT_LEN, "needs a multi-block definition");
        let def = client(sim.transport()).definition().unwrap();
        assert_eq!(def, serde_json::from_str::<serde_json::Value>(SMALL_DEFINITION).unwrap());
    }

    #[test]
    fn reads_keymap_in_28_byte_chunks() {
        let codes: Vec<u16> = (0..240).collect();
        let sim = KeyboardSim::new(SMALL_DEFINITION, 4, 10, 6, &codes, &[]);
        let t = sim.transport();
        let written = t.written();
        let buf = client(t).keymap_buffer(480).unwrap();
        assert_eq!(buf, codes.iter().flat_map(|c| c.to_be_bytes()).collect::<Vec<u8>>());
        let w = written.lock().unwrap();
        assert_eq!(w.len(), 18);
        assert_eq!(w[17][..4], [VIA_GET_BUFFER, 0x01, 0xDC, 4]); // offset 476, last 4 bytes
    }

    #[test]
    fn layer_count_rejects_zero() {
        assert_eq!(client(KeyboardSim::small().transport()).layer_count().unwrap(), 2);
        let t = FakeTransport::new(|req| vec![*req]); // echo: layer count 0
        assert!(matches!(client(t).layer_count(), Err(VialError::BadReply(_))));
    }

    #[test]
    fn unlock_status_and_handshake() {
        let sim = KeyboardSim::small();
        let mut c = client(sim.transport());
        let status = c.unlock_status().unwrap();
        assert_eq!(status, UnlockStatus { unlocked: false, in_progress: false, keys: vec![(1, 0), (1, 2)] });

        sim.with(|s| s.matrix[1] = 0b101); // hold both unlock keys
        c.unlock_start().unwrap();
        let mut poll_after = |ms| {
            sim.with(|s| s.clock_ms += ms);
            c.unlock_poll().unwrap()
        };
        let mut last = poll_after(200);
        assert_eq!(last.counter, UNLOCK_COUNTER_MAX - 1);
        for _ in 0..60 {
            if last.unlocked {
                break;
            }
            last = poll_after(200);
        }
        assert!(last.unlocked && !last.in_progress);
    }

    #[test]
    fn unlock_poll_too_soon_restarts_the_countdown() {
        let sim = KeyboardSim::small();
        let mut c = client(sim.transport());
        sim.with(|s| s.matrix[1] = 0b101);
        c.unlock_start().unwrap();
        sim.with(|s| s.clock_ms += 200);
        assert_eq!(c.unlock_poll().unwrap().counter, UNLOCK_COUNTER_MAX - 1);
        sim.with(|s| s.clock_ms += 50); // keys still held, but too soon after the last step
        assert_eq!(c.unlock_poll().unwrap().counter, UNLOCK_COUNTER_MAX);
    }

    #[test]
    fn matrix_state_decodes_rows() {
        let sim = KeyboardSim::small();
        sim.with(|s| {
            s.unlocked = true;
            s.matrix = vec![0b010, 0b101];
        });
        let m = client(sim.transport()).matrix_state(2, 3).unwrap();
        assert_eq!(m.pressed(), vec![(0, 1), (1, 0), (1, 2)]);
        assert!(m.is_pressed(0, 1) && !m.is_pressed(0, 0) && !m.is_pressed(9, 9));
    }

    #[test]
    fn locked_matrix_reads_as_empty() {
        let sim = KeyboardSim::small();
        sim.with(|s| s.matrix = vec![0b111, 0b111]);
        let m = client(sim.transport()).matrix_state(2, 3).unwrap();
        assert!(m.pressed().is_empty());
    }

    #[test]
    fn wide_matrix_rows_are_big_endian() {
        let sim = KeyboardSim::new(SMALL_DEFINITION, 1, 2, 12, &[0; 24], &[]);
        sim.with(|s| {
            s.unlocked = true;
            s.matrix = vec![1 << 9, 1];
        });
        let m = client(sim.transport()).matrix_state(2, 12).unwrap();
        assert_eq!(m.pressed(), vec![(0, 9), (1, 0)]);
    }

    #[test]
    fn skips_replies_meant_for_other_programs() {
        let t = FakeTransport::new(|req| {
            if req[0] == VIA_GET_LAYER_COUNT {
                vec![report(&[VIA_GET_KEYBOARD_VALUE, 3]), report(&[VIA_GET_LAYER_COUNT, 4])]
            } else {
                vec![]
            }
        });
        assert_eq!(client(t).layer_count().unwrap(), 4);
    }

    #[test]
    fn drains_stale_reports_before_sending() {
        let mut t = KeyboardSim::small().transport();
        t.push_unsolicited(report(&[VIA_GET_LAYER_COUNT, 9]));
        assert_eq!(client(t).layer_count().unwrap(), 2);
    }

    #[test]
    fn times_out_after_one_retry() {
        let t = FakeTransport::new(|_| vec![]);
        let written = t.written();
        assert!(matches!(client(t).layer_count(), Err(VialError::Timeout(VIA_GET_LAYER_COUNT))));
        assert_eq!(written.lock().unwrap().len(), 2);
    }

    #[test]
    fn unplugged_device_is_a_disconnect() {
        let sim = KeyboardSim::small();
        let mut c = client(sim.transport());
        sim.unplug();
        assert!(c.layer_count().unwrap_err().is_disconnect());
    }
}
