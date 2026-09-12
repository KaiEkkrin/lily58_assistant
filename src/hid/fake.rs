//! Test doubles for the raw-HID layer.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::hid::transport::Transport;
use crate::protocol::*;

type Responder = Box<dyn FnMut(&Report) -> Vec<Report> + Send>;

/// Scripted transport. Each written report goes to the responder, and the reports
/// it returns are queued for reading. Reads never block: an empty queue reads as a timeout.
pub struct FakeTransport {
    responder: Responder,
    queue: VecDeque<Report>,
    written: Arc<Mutex<Vec<Report>>>,
    unplugged: Arc<Mutex<bool>>,
}

impl FakeTransport {
    pub fn new(responder: impl FnMut(&Report) -> Vec<Report> + Send + 'static) -> Self {
        Self::with_unplug_flag(responder, Arc::new(Mutex::new(false)))
    }

    /// While `*unplugged` is true every read and write fails, like a pulled USB cable.
    pub fn with_unplug_flag(
        responder: impl FnMut(&Report) -> Vec<Report> + Send + 'static,
        unplugged: Arc<Mutex<bool>>,
    ) -> Self {
        Self { responder: Box::new(responder), queue: VecDeque::new(), written: Arc::default(), unplugged }
    }

    /// Every report written so far (shared, so it stays readable after the transport moves).
    pub fn written(&self) -> Arc<Mutex<Vec<Report>>> {
        Arc::clone(&self.written)
    }

    /// Queues a report nobody asked for, e.g. a reply meant for another program.
    pub fn push_unsolicited(&mut self, report: Report) {
        self.queue.push_back(report);
    }

    fn check_plugged(&self) -> io::Result<()> {
        if *self.unplugged.lock().unwrap() {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "fake device unplugged"))
        } else {
            Ok(())
        }
    }
}

impl Transport for FakeTransport {
    fn write_report(&mut self, report: &Report) -> io::Result<()> {
        self.check_plugged()?;
        self.written.lock().unwrap().push(*report);
        let replies = (self.responder)(report);
        self.queue.extend(replies);
        Ok(())
    }

    fn read_report(&mut self, _timeout: Duration) -> io::Result<Option<Report>> {
        self.check_plugged()?;
        Ok(self.queue.pop_front())
    }
}

/// State of a simulated Vial keyboard. Fields are public so tests can poke them.
pub struct SimState {
    pub definition_xz: Vec<u8>,
    pub layers: u8,
    pub rows: u8,
    pub cols: u8,
    /// Big-endian u16 per key, layer → row → col (as the firmware's EEPROM buffer).
    pub keymap: Vec<u8>,
    pub vial_protocol: u32,
    pub unlocked: bool,
    pub unlock_in_progress: bool,
    pub unlock_counter: u8,
    pub unlock_keys: Vec<(u8, u8)>,
    /// The firmware's millisecond clock; tests advance it.
    pub clock_ms: u64,
    /// When the unlock countdown last moved (or started), per the firmware's `vial_unlock_timer`.
    pub unlock_timer_ms: u64,
    /// One bitmask per row; bit n = column n pressed.
    pub matrix: Vec<u32>,
    /// Number of reports received.
    pub requests: usize,
    /// Number of matrix-state requests received (a subset of `requests`).
    pub matrix_requests: usize,
}

/// A simulated Vial keyboard that answers like vial-qmk's via.c / vial.c.
#[derive(Clone)]
pub struct KeyboardSim {
    state: Arc<Mutex<SimState>>,
    unplugged: Arc<Mutex<bool>>,
}

pub const SMALL_DEFINITION: &str =
    r#"{"name":"Sim58","matrix":{"rows":2,"cols":3},"layouts":{"keymap":[["0,0","0,1","0,2"],["1,0","1,1","1,2"]]}}"#;

/// layer 0: A B C / MO(1) SPC LSFT; layer 1: 1 TRNS 3 / TRNS TRNS TRNS
pub const SMALL_KEYMAP: [u16; 12] = [
    0x0004, 0x0005, 0x0006, 0x5221, 0x002C, 0x00E1, //
    0x001E, 0x0001, 0x0020, 0x0001, 0x0001, 0x0001,
];

impl KeyboardSim {
    pub fn new(definition_json: &str, layers: u8, rows: u8, cols: u8, codes: &[u16], unlock_keys: &[(u8, u8)]) -> Self {
        let mut definition_xz = Vec::new();
        lzma_rs::xz_compress(&mut definition_json.as_bytes(), &mut definition_xz).unwrap();
        let state = SimState {
            definition_xz,
            layers,
            rows,
            cols,
            keymap: codes.iter().flat_map(|c| c.to_be_bytes()).collect(),
            vial_protocol: 6,
            unlocked: false,
            unlock_in_progress: false,
            unlock_counter: 0,
            unlock_keys: unlock_keys.to_vec(),
            clock_ms: 0,
            unlock_timer_ms: 0,
            matrix: vec![0; rows as usize],
            requests: 0,
            matrix_requests: 0,
        };
        Self { state: Arc::new(Mutex::new(state)), unplugged: Arc::new(Mutex::new(false)) }
    }

    /// 2x3 matrix, 2 layers (`SMALL_KEYMAP`), unlock keys (1,0) + (1,2). Starts locked.
    pub fn small() -> Self {
        Self::new(SMALL_DEFINITION, 2, 2, 3, &SMALL_KEYMAP, &[(1, 0), (1, 2)])
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut SimState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    pub fn transport(&self) -> FakeTransport {
        let state = Arc::clone(&self.state);
        FakeTransport::with_unplug_flag(move |req| respond(&mut state.lock().unwrap(), req), Arc::clone(&self.unplugged))
    }

    pub fn unplug(&self) {
        *self.unplugged.lock().unwrap() = true;
    }

    /// Plugs the keyboard back in. Unlike real hardware, its state (e.g. unlocked) carries over.
    pub fn replug(&self) {
        *self.unplugged.lock().unwrap() = false;
    }

    pub fn is_unplugged(&self) -> bool {
        *self.unplugged.lock().unwrap()
    }
}

fn respond(s: &mut SimState, req: &Report) -> Vec<Report> {
    s.requests += 1;
    let mut r = *req;
    let unlock_subset = req[0] == VIAL_PREFIX
        && matches!(
            req[1],
            VIAL_GET_KEYBOARD_ID | VIAL_GET_SIZE | VIAL_GET_DEF | VIAL_GET_UNLOCK_STATUS | VIAL_UNLOCK_START | VIAL_UNLOCK_POLL
        );
    if s.unlock_in_progress && !unlock_subset {
        return vec![r]; // firmware skips everything else and echoes the request
    }
    match (req[0], req[1]) {
        (VIA_GET_PROTOCOL_VERSION, _) => (r[1], r[2]) = (0x00, 0x0C),
        (VIA_GET_LAYER_COUNT, _) => r[1] = s.layers,
        (VIA_GET_BUFFER, _) => {
            let offset = u16::from_be_bytes([req[1], req[2]]) as usize;
            let size = req[3] as usize;
            if size <= BUFFER_CHUNK {
                for (i, byte) in r[4..4 + size].iter_mut().enumerate() {
                    *byte = s.keymap.get(offset + i).copied().unwrap_or(0);
                }
            }
        }
        (VIA_GET_KEYBOARD_VALUE, VIA_SWITCH_MATRIX_STATE) => {
            s.matrix_requests += 1;
            if s.unlocked {
                // Locked: firmware skips and echoes, which reads as "nothing pressed".
                let row_size = (s.cols as usize).div_ceil(8);
                for (row, bits) in s.matrix.iter().enumerate() {
                    let be = bits.to_be_bytes();
                    r[2 + row * row_size..2 + (row + 1) * row_size].copy_from_slice(&be[4 - row_size..]);
                }
            }
        }
        (VIAL_PREFIX, VIAL_GET_KEYBOARD_ID) => {
            r = [0; REPORT_LEN];
            r[0..4].copy_from_slice(&s.vial_protocol.to_le_bytes());
            r[4..12].copy_from_slice(&0x0648_397D_5BFC_FD7E_u64.to_le_bytes());
        }
        (VIAL_PREFIX, VIAL_GET_SIZE) => r[0..4].copy_from_slice(&(s.definition_xz.len() as u32).to_le_bytes()),
        (VIAL_PREFIX, VIAL_GET_DEF) => {
            let start = (req[2] as usize | (req[3] as usize) << 8) * REPORT_LEN;
            if start < s.definition_xz.len() {
                let end = (start + REPORT_LEN).min(s.definition_xz.len());
                r[..end - start].copy_from_slice(&s.definition_xz[start..end]);
            }
        }
        (VIAL_PREFIX, VIAL_GET_UNLOCK_STATUS) => {
            r = [0xFF; REPORT_LEN];
            r[0] = s.unlocked as u8;
            r[1] = s.unlock_in_progress as u8;
            for (i, &(row, col)) in s.unlock_keys.iter().enumerate() {
                r[2 + i * 2] = row;
                r[3 + i * 2] = col;
            }
        }
        (VIAL_PREFIX, VIAL_UNLOCK_START) => {
            s.unlock_in_progress = true;
            s.unlock_counter = UNLOCK_COUNTER_MAX;
            s.unlock_timer_ms = s.clock_ms;
        }
        (VIAL_PREFIX, VIAL_UNLOCK_POLL) => {
            if s.unlock_in_progress {
                let holding = s.unlock_keys.iter().all(|&(row, col)| (s.matrix[row as usize] >> col) & 1 == 1);
                // Like vial.c: a poll within 100 ms of the last step resets the countdown too.
                if s.clock_ms - s.unlock_timer_ms > 100 && holding {
                    s.unlock_timer_ms = s.clock_ms;
                    s.unlock_counter -= 1;
                    if s.unlock_counter == 0 {
                        s.unlock_in_progress = false;
                        s.unlocked = true;
                    }
                } else {
                    s.unlock_counter = UNLOCK_COUNTER_MAX;
                }
            }
            (r[0], r[1], r[2]) = (s.unlocked as u8, s.unlock_in_progress as u8, s.unlock_counter);
        }
        _ => r[0] = 0xFF, // id_unhandled
    }
    vec![r]
}
