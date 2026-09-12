//! The device worker: owns the keyboard connection on its own thread.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::hid::discover::{self, VialDevice};
use crate::hid::guard::ReadOnlyGuard;
use crate::hid::hidraw;
use crate::hid::transport::Transport;
use crate::keymap::Keymap;
use crate::layout::Layout;
use crate::vial::{self, MatrixState, VialClient, VialError};

pub const SCAN_INTERVAL: Duration = Duration::from_secs(1);
pub const HOLDER_CHECK_INTERVAL: Duration = Duration::from_secs(1);
pub const MATRIX_INTERVAL: Duration = Duration::from_millis(10);
pub const LOCKED_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// The firmware steps its 50-step countdown on a poll that comes more than 100 ms after the
/// last step, and restarts it on any poll sooner than that. 200 ms matches Vial's own GUI, so an
/// unlock takes about 10 s.
pub const UNLOCK_POLL_INTERVAL: Duration = Duration::from_millis(200);
const IDLE_POLL: Duration = Duration::from_millis(100);
/// How long to wait before retrying after a connected session fails for a reason other than
/// disconnection (e.g. a bad definition), so a deterministic failure doesn't spam reconnects.
pub const RETRY_AFTER_ERROR: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceInfo {
    pub dev_node: PathBuf,
    pub usb_dir: PathBuf,
    pub product: String,
    pub via_protocol: u16,
    pub vial_protocol: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceEvent {
    /// No Vial keyboard is plugged in.
    Waiting,
    /// The raw-HID node exists but we may not open it.
    NoAccess(PathBuf),
    /// Another program (e.g. Vial) has the keyboard open, so we send nothing.
    Paused { holders: Vec<String> },
    Resumed,
    Connected { info: DeviceInfo, layout: Layout, keymap: Keymap },
    Disconnected,
    Locked { unlock_keys: Vec<(u8, u8)> },
    Unlocking { counter: u8, unlock_keys: Vec<(u8, u8)> },
    Unlocked,
    Matrix { pressed: Vec<(u8, u8)>, released: Vec<(u8, u8)> },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCommand {
    Reload,
    StartUnlock,
}

/// Discovery and opening, abstracted so tests can plug in a simulated keyboard.
pub trait Connector: Send {
    fn find(&mut self) -> io::Result<Option<VialDevice>>;
    fn open(&mut self, dev: &VialDevice) -> io::Result<ReadOnlyGuard<Box<dyn Transport>>>;
    fn other_holders(&mut self, dev: &VialDevice) -> Vec<String>;
}

pub struct SystemConnector;

impl Connector for SystemConnector {
    fn find(&mut self) -> io::Result<Option<VialDevice>> {
        discover::find_vial_device(Path::new("/sys"), Path::new("/dev"))
    }

    fn open(&mut self, dev: &VialDevice) -> io::Result<ReadOnlyGuard<Box<dyn Transport>>> {
        Ok(hidraw::open_guarded(&dev.dev_node)?.boxed())
    }

    fn other_holders(&mut self, dev: &VialDevice) -> Vec<String> {
        discover::other_holders(Path::new("/proc"), &dev.dev_node, std::process::id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lock {
    Locked,
    Unlocking,
    Unlocked,
}

struct Session {
    dev: VialDevice,
    client: VialClient<Box<dyn Transport>>,
    vial_protocol: u32,
    lock: Lock,
    unlock_keys: Vec<(u8, u8)>,
    /// Layout and keymap have been sent to the UI. Stays false while an unlock is in
    /// progress, because the firmware ignores VIA reads until the unlock completes.
    loaded: bool,
    rows: u8,
    cols: u8,
    matrix: MatrixState,
    paused: bool,
    next_holder_check: Instant,
    next_lock_check: Instant,
    /// Unlock polls must stay `UNLOCK_POLL_INTERVAL` apart even when a command wakes the
    /// worker early, or the firmware restarts the countdown.
    next_unlock_poll: Instant,
    /// True once `load()` has pushed a `Connected` event, so `drop_session` knows whether the
    /// UI has anything to clear with `Disconnected`.
    connected_announced: bool,
}

enum Conn {
    /// `reported` is the last idle event sent, so it isn't repeated every scan.
    Idle { reported: Option<DeviceEvent> },
    Connected(Box<Session>),
}

pub struct Worker<C: Connector> {
    connector: C,
    events: Sender<DeviceEvent>,
    notify: Box<dyn Fn() + Send>,
    conn: Conn,
    /// The message of the last non-disconnect error reported for the current run of
    /// connection attempts, so a deterministic failure (e.g. a bad definition) isn't
    /// re-announced on every reconnect. Cleared whenever a session loads successfully.
    last_failure: Option<String>,
}

impl<C: Connector> Worker<C> {
    pub fn new(connector: C, events: Sender<DeviceEvent>, notify: Box<dyn Fn() + Send>) -> Self {
        Self { connector, events, notify, conn: Conn::Idle { reported: None }, last_failure: None }
    }

    /// Does one unit of work and returns how long to wait before the next.
    pub fn step(&mut self, now: Instant) -> Duration {
        let mut out = Vec::new();
        let wait = if let Conn::Connected(s) = &mut self.conn {
            match poll(s, &mut self.connector, now, &mut out) {
                Ok(wait) => {
                    if out.iter().any(|e| matches!(e, DeviceEvent::Connected { .. })) {
                        self.last_failure = None;
                    }
                    wait
                }
                Err(e) => {
                    let wait = if e.is_disconnect() { SCAN_INTERVAL } else { RETRY_AFTER_ERROR };
                    self.drop_session(e, &mut out);
                    wait
                }
            }
        } else {
            self.try_connect(now, &mut out)
        };
        self.emit_all(out);
        wait
    }

    pub fn handle(&mut self, cmd: DeviceCommand) {
        let Conn::Connected(s) = &mut self.conn else { return };
        let result = match cmd {
            DeviceCommand::Reload => {
                s.loaded = false; // the next step re-reads layout and keymap
                Ok(())
            }
            DeviceCommand::StartUnlock if s.lock == Lock::Locked && !s.paused => {
                let started = s.client.unlock_start();
                if started.is_ok() {
                    s.lock = Lock::Unlocking;
                }
                started
            }
            DeviceCommand::StartUnlock => Ok(()),
        };
        if let Err(e) = result {
            let mut out = Vec::new();
            self.drop_session(e, &mut out);
            self.emit_all(out);
        }
    }

    fn emit_all(&self, events: Vec<DeviceEvent>) {
        if events.is_empty() {
            return;
        }
        for event in events {
            let _ = self.events.send(event);
        }
        (self.notify)();
    }

    fn try_connect(&mut self, now: Instant, out: &mut Vec<DeviceEvent>) -> Duration {
        let idle = match self.connector.find() {
            Err(e) => DeviceEvent::Error(format!("scanning for the keyboard failed: {e}")),
            Ok(None) => {
                self.last_failure = None; // a keyboard plugged in later gets its errors shown
                DeviceEvent::Waiting
            }
            Ok(Some(dev)) => {
                let holders = self.connector.other_holders(&dev);
                if !holders.is_empty() {
                    DeviceEvent::Paused { holders }
                } else {
                    match self.connector.open(&dev) {
                        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => DeviceEvent::NoAccess(dev.dev_node.clone()),
                        Err(e) => DeviceEvent::Error(format!("cannot open {}: {e}", dev.dev_node.display())),
                        Ok(guard) => match start_session(dev, guard, now) {
                            Ok(session) => {
                                self.conn = Conn::Connected(Box::new(session));
                                return Duration::ZERO;
                            }
                            // Unplugged between discovery and the first reply.
                            Err(e) if e.is_disconnect() => {
                                log::info!("keyboard went away while connecting: {e}");
                                self.last_failure = None;
                                DeviceEvent::Waiting
                            }
                            Err(e) => DeviceEvent::Error(e.to_string()),
                        },
                    }
                }
            }
        };
        if let Conn::Idle { reported } = &mut self.conn
            && reported.as_ref() != Some(&idle)
        {
            *reported = Some(idle.clone());
            out.push(idle);
        }
        SCAN_INTERVAL
    }

    fn drop_session(&mut self, e: VialError, out: &mut Vec<DeviceEvent>) {
        // Only announce a disconnect if the UI was ever told we connected; a session that
        // never got past `load()` gave it nothing to clear.
        if matches!(&self.conn, Conn::Connected(s) if s.connected_announced) {
            out.push(DeviceEvent::Disconnected);
        }
        let reported = if e.is_disconnect() {
            log::info!("keyboard went away: {e}");
            self.last_failure = None;
            None
        } else {
            log::warn!("keyboard error: {e}");
            let msg = e.to_string();
            if self.last_failure.as_deref() != Some(msg.as_str()) {
                out.push(DeviceEvent::Error(msg.clone()));
                self.last_failure = Some(msg.clone());
            }
            Some(DeviceEvent::Error(msg))
        };
        self.conn = Conn::Idle { reported };
    }
}

/// The `Lock` state implied by a Vial unlock-status reply, shared by the initial connect
/// and by a resume (another program may have locked or unlocked the keyboard while paused).
fn lock_from_status(unlocked: bool, in_progress: bool) -> Lock {
    match (unlocked, in_progress) {
        (true, _) => Lock::Unlocked,
        (false, true) => Lock::Unlocking,
        (false, false) => Lock::Locked,
    }
}

fn start_session(dev: VialDevice, guard: ReadOnlyGuard<Box<dyn Transport>>, now: Instant) -> Result<Session, VialError> {
    let mut client = VialClient::new(guard);
    // Only Vial commands here: they are answered even while an unlock is in progress.
    let id = client.keyboard_id()?;
    vial::check_protocol(&id)?;
    let status = client.unlock_status()?;
    let lock = lock_from_status(status.unlocked, status.in_progress);
    Ok(Session {
        dev,
        client,
        vial_protocol: id.vial_protocol,
        lock,
        unlock_keys: status.keys,
        loaded: false,
        rows: 0,
        cols: 0,
        matrix: MatrixState::empty(0, 0),
        paused: false,
        next_holder_check: now,
        next_lock_check: now + LOCKED_CHECK_INTERVAL,
        next_unlock_poll: now,
        connected_announced: false,
    })
}

fn poll<C: Connector>(s: &mut Session, connector: &mut C, now: Instant, out: &mut Vec<DeviceEvent>) -> Result<Duration, VialError> {
    if now >= s.next_holder_check {
        s.next_holder_check = now + HOLDER_CHECK_INTERVAL;
        let holders = connector.other_holders(&s.dev);
        if !holders.is_empty() && !s.paused {
            s.paused = true;
            out.push(DeviceEvent::Paused { holders });
        } else if holders.is_empty() && s.paused {
            s.paused = false;
            // The other program may have locked, unlocked, or changed the keymap while it
            // held the device: re-read the lock state before anything else runs, so the
            // Unlocking branch below sees it and no VIA read is sent to a keyboard that
            // is mid-handshake or newly locked.
            let status = s.client.unlock_status()?;
            s.unlock_keys = status.keys;
            s.lock = lock_from_status(status.unlocked, status.in_progress);
            s.loaded = false;
            out.push(DeviceEvent::Resumed);
        }
    }
    if s.paused {
        return Ok(HOLDER_CHECK_INTERVAL);
    }

    if s.lock == Lock::Unlocking {
        if now < s.next_unlock_poll {
            return Ok(s.next_unlock_poll - now);
        }
        s.next_unlock_poll = now + UNLOCK_POLL_INTERVAL;
        let p = s.client.unlock_poll()?;
        if !p.unlocked {
            out.push(DeviceEvent::Unlocking { counter: p.counter, unlock_keys: s.unlock_keys.clone() });
            return Ok(UNLOCK_POLL_INTERVAL);
        }
        s.lock = Lock::Unlocked;
        out.push(DeviceEvent::Unlocked);
    }

    if !s.loaded {
        load(s, out)?;
    }

    if s.lock == Lock::Unlocked {
        let m = s.client.matrix_state(s.rows, s.cols)?;
        let pressed: Vec<_> = m.pressed().into_iter().filter(|&(r, c)| !s.matrix.is_pressed(r, c)).collect();
        let released: Vec<_> = s.matrix.pressed().into_iter().filter(|&(r, c)| !m.is_pressed(r, c)).collect();
        if !pressed.is_empty() || !released.is_empty() {
            out.push(DeviceEvent::Matrix { pressed, released });
        }
        s.matrix = m;
        return Ok(MATRIX_INTERVAL);
    }

    if now >= s.next_lock_check {
        s.next_lock_check = now + LOCKED_CHECK_INTERVAL;
        let status = s.client.unlock_status()?;
        s.unlock_keys = status.keys;
        if status.unlocked {
            s.lock = Lock::Unlocked;
            out.push(DeviceEvent::Unlocked);
            return Ok(Duration::ZERO);
        }
        if status.in_progress {
            s.lock = Lock::Unlocking;
            return Ok(Duration::ZERO);
        }
    }
    Ok(IDLE_POLL)
}

fn load(s: &mut Session, out: &mut Vec<DeviceEvent>) -> Result<(), VialError> {
    let via_protocol = s.client.via_protocol_version()?;
    let definition = s.client.definition()?;
    let layout = Layout::from_definition(&definition).map_err(|e| VialError::BadReply(e.to_string()))?;
    let layers = s.client.layer_count()?;
    let buf = s.client.keymap_buffer(Keymap::buffer_len(layers, layout.rows, layout.cols))?;
    let keymap = Keymap::from_buffer(layers, layout.rows, layout.cols, &buf).map_err(|e| VialError::BadReply(e.to_string()))?;
    (s.rows, s.cols) = (layout.rows, layout.cols);
    s.matrix = MatrixState::empty(s.rows, s.cols);
    s.loaded = true;
    s.connected_announced = true;
    let info = DeviceInfo {
        dev_node: s.dev.dev_node.clone(),
        usb_dir: s.dev.usb_dir.clone(),
        product: s.dev.product.clone(),
        via_protocol,
        vial_protocol: s.vial_protocol,
    };
    out.push(DeviceEvent::Connected { info, layout, keymap });
    out.push(match s.lock {
        Lock::Unlocked => DeviceEvent::Unlocked,
        _ => DeviceEvent::Locked { unlock_keys: s.unlock_keys.clone() },
    });
    Ok(())
}

/// Runs a `Worker` on its own thread. Dropping the returned sender stops it.
pub fn spawn(
    connector: impl Connector + 'static,
    events: Sender<DeviceEvent>,
    notify: impl Fn() + Send + 'static,
) -> Sender<DeviceCommand> {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let spawned = thread::Builder::new().name("device".into()).spawn(move || {
        let mut worker = Worker::new(connector, events, Box::new(notify));
        loop {
            let wait = worker.step(Instant::now());
            match cmd_rx.recv_timeout(wait) {
                Ok(cmd) => worker.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    if let Err(e) = spawned {
        // The closure, and the event sender with it, is dropped, so the UI sees the channel close.
        log::error!("cannot start the device thread: {e}");
    }
    cmd_tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::fake::{KeyboardSim, SMALL_KEYMAP};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};

    struct FakeConnector {
        sim: KeyboardSim,
        deny: bool,
        /// Unplug the keyboard as it is opened: it vanishes between discovery and the first read.
        vanish_on_open: bool,
        holders: Arc<Mutex<Vec<String>>>,
    }

    impl Connector for FakeConnector {
        fn find(&mut self) -> io::Result<Option<VialDevice>> {
            Ok((!self.sim.is_unplugged()).then(|| VialDevice {
                dev_node: "/dev/hidraw99".into(),
                usb_dir: "/sys/devices/fake".into(),
                vendor_id: 0x7171,
                product_id: 0x0012,
                product: "Sim58".into(),
            }))
        }

        fn open(&mut self, _: &VialDevice) -> io::Result<ReadOnlyGuard<Box<dyn Transport>>> {
            if self.deny {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            if self.vanish_on_open {
                self.sim.unplug();
            }
            Ok(ReadOnlyGuard::new(self.sim.transport()).boxed())
        }

        fn other_holders(&mut self, _: &VialDevice) -> Vec<String> {
            self.holders.lock().unwrap().clone()
        }
    }

    struct Harness {
        worker: Worker<FakeConnector>,
        rx: Receiver<DeviceEvent>,
        sim: KeyboardSim,
        holders: Arc<Mutex<Vec<String>>>,
        t0: Instant,
    }

    impl Harness {
        fn new(sim: KeyboardSim) -> Self {
            Self::build(sim, false)
        }

        fn build(sim: KeyboardSim, deny: bool) -> Self {
            let (tx, rx) = mpsc::channel();
            let holders = Arc::new(Mutex::new(Vec::new()));
            let connector = FakeConnector { sim: sim.clone(), deny, vanish_on_open: false, holders: Arc::clone(&holders) };
            Self { worker: Worker::new(connector, tx, Box::new(|| {})), rx, sim, holders, t0: Instant::now() }
        }

        /// Runs `n` steps at t0 + `at_ms` and returns the events they produced.
        fn steps(&mut self, n: usize, at_ms: u64) -> Vec<DeviceEvent> {
            self.sim.with(|s| s.clock_ms = at_ms);
            for _ in 0..n {
                self.worker.step(self.t0 + Duration::from_millis(at_ms));
            }
            self.rx.try_iter().collect()
        }

        /// Runs the worker as `spawn` does, from `from_ms` until `until_ms`, advancing the
        /// clock by each wait it asks for. Returns the events produced.
        fn run(&mut self, from_ms: u64, until_ms: u64) -> Vec<DeviceEvent> {
            let mut t = from_ms;
            while t < until_ms {
                self.sim.with(|s| s.clock_ms = t);
                let wait = self.worker.step(self.t0 + Duration::from_millis(t));
                t += (wait.as_millis() as u64).max(1);
            }
            self.rx.try_iter().collect()
        }
    }

    fn names(events: &[DeviceEvent]) -> Vec<&'static str> {
        events
            .iter()
            .map(|e| match e {
                DeviceEvent::Waiting => "Waiting",
                DeviceEvent::NoAccess(_) => "NoAccess",
                DeviceEvent::Paused { .. } => "Paused",
                DeviceEvent::Resumed => "Resumed",
                DeviceEvent::Connected { .. } => "Connected",
                DeviceEvent::Disconnected => "Disconnected",
                DeviceEvent::Locked { .. } => "Locked",
                DeviceEvent::Unlocking { .. } => "Unlocking",
                DeviceEvent::Unlocked => "Unlocked",
                DeviceEvent::Matrix { .. } => "Matrix",
                DeviceEvent::Error(_) => "Error",
            })
            .collect()
    }

    #[test]
    fn waiting_is_reported_once() {
        let sim = KeyboardSim::small();
        sim.unplug();
        let mut h = Harness::new(sim);
        assert_eq!(names(&h.steps(1, 0)), ["Waiting"]);
        assert!(h.steps(3, 0).is_empty());
    }

    #[test]
    fn permission_denied_is_no_access() {
        let mut h = Harness::build(KeyboardSim::small(), true);
        assert_eq!(h.steps(2, 0), vec![DeviceEvent::NoAccess("/dev/hidraw99".into())]);
    }

    #[test]
    fn connects_loads_and_reports_lock_state() {
        let mut h = Harness::new(KeyboardSim::small());
        let events = h.steps(2, 0);
        assert_eq!(names(&events), ["Connected", "Locked"]);
        let DeviceEvent::Connected { info, layout, keymap } = &events[0] else { unreachable!() };
        assert_eq!(layout.keys.len(), 6);
        assert_eq!(keymap.get(0, 1, 0), 0x5221);
        assert_eq!(info.vial_protocol, 6);
        assert_eq!(events[1], DeviceEvent::Locked { unlock_keys: vec![(1, 0), (1, 2)] });
    }

    #[test]
    fn unlocked_keyboard_streams_matrix_changes() {
        let sim = KeyboardSim::small();
        sim.with(|s| s.unlocked = true);
        let mut h = Harness::new(sim);
        assert_eq!(names(&h.steps(2, 0)), ["Connected", "Unlocked"]);
        h.sim.with(|s| s.matrix[0] = 0b010);
        assert_eq!(h.steps(1, 0), vec![DeviceEvent::Matrix { pressed: vec![(0, 1)], released: vec![] }]);
        assert!(h.steps(1, 0).is_empty());
        h.sim.with(|s| s.matrix[0] = 0);
        assert_eq!(h.steps(1, 0), vec![DeviceEvent::Matrix { pressed: vec![], released: vec![(0, 1)] }]);
    }

    #[test]
    fn unlock_handshake_then_matrix() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.sim.with(|s| s.matrix[1] = 0b101); // the user holds both unlock keys
        h.worker.handle(DeviceCommand::StartUnlock);
        let events = h.run(0, 12_000);
        let counters: Vec<u8> =
            events.iter().filter_map(|e| if let DeviceEvent::Unlocking { counter, .. } = e { Some(*counter) } else { None }).collect();
        assert_eq!(counters.last(), Some(&1), "the countdown must reach the end: {counters:?}");
        assert!(counters.windows(2).all(|w| w[1] < w[0]), "the countdown never restarts: {counters:?}");
        let unlocked = names(&events).iter().position(|&e| e == "Unlocked").expect("unlocked");
        assert_eq!(events[unlocked + 1], DeviceEvent::Matrix { pressed: vec![(1, 0), (1, 2)], released: vec![] });
    }

    #[test]
    fn early_wakeups_do_not_poll_the_unlock_too_soon() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.worker.handle(DeviceCommand::StartUnlock);
        assert_eq!(names(&h.steps(1, 0)), ["Unlocking"]);
        let before = h.sim.with(|s| s.requests);
        // A command (e.g. Reload) wakes the worker before its wait is over.
        h.worker.handle(DeviceCommand::Reload);
        assert!(h.steps(3, 50).is_empty());
        assert_eq!(h.sim.with(|s| s.requests), before, "no unlock poll within the interval");
        assert_eq!(names(&h.steps(1, UNLOCK_POLL_INTERVAL.as_millis() as u64)), ["Unlocking"]);
    }

    #[test]
    fn unlock_left_in_progress_defers_keymap_reads() {
        let sim = KeyboardSim::small();
        sim.with(|s| {
            s.unlock_in_progress = true;
            s.unlock_counter = 50;
            s.matrix[1] = 0b101;
        });
        let mut h = Harness::new(sim);
        let events = h.run(0, 12_000);
        let n = names(&events);
        let unlocked = n.iter().position(|&e| e == "Unlocked").expect("unlocked");
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..unlocked].iter().all(|&e| e == "Unlocking"));
        assert!(connected > unlocked);
        let DeviceEvent::Connected { keymap, .. } = &events[connected] else { unreachable!() };
        assert_eq!(keymap.get(0, 0, 0), 0x0004, "keymap must be read after the unlock, not as echoed zeros");
    }

    #[test]
    fn pauses_while_another_program_holds_the_device() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        *h.holders.lock().unwrap() = vec!["vial (4242)".into()];
        assert_eq!(h.steps(1, 1000), vec![DeviceEvent::Paused { holders: vec!["vial (4242)".into()] }]);
        let before = h.sim.with(|s| s.requests);
        assert!(h.steps(5, 1500).is_empty());
        assert_eq!(h.sim.with(|s| s.requests), before, "no traffic while paused");
        h.holders.lock().unwrap().clear();
        assert_eq!(names(&h.steps(1, 2000)), ["Resumed", "Connected", "Locked"]);
    }

    #[test]
    fn does_not_connect_while_another_program_holds_the_device() {
        let mut h = Harness::new(KeyboardSim::small());
        *h.holders.lock().unwrap() = vec!["vial (1)".into()];
        assert_eq!(names(&h.steps(2, 0)), ["Paused"]);
        assert_eq!(h.sim.with(|s| s.requests), 0);
    }

    #[test]
    fn unplug_reports_disconnect_then_waiting() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.sim.unplug();
        assert_eq!(names(&h.steps(1, 2000)), ["Disconnected"]); // the 2 s lock check hits the dead device
        assert_eq!(names(&h.steps(1, 2000)), ["Waiting"]);
    }

    #[test]
    fn old_protocol_is_an_error_reported_once() {
        let sim = KeyboardSim::small();
        sim.with(|s| s.vial_protocol = 5);
        let mut h = Harness::new(sim);
        let events = h.steps(3, 0);
        assert_eq!(names(&events), ["Error"]);
        assert!(matches!(&events[0], DeviceEvent::Error(m) if m.contains("too old")));
    }

    #[test]
    fn reload_re_reads_the_keymap() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.sim.with(|s| s.keymap[1] = 0x07); // layer 0, key (0,0): low byte → KC_D
        h.worker.handle(DeviceCommand::Reload);
        let events = h.steps(1, 0);
        assert_eq!(names(&events), ["Connected", "Locked"]);
        let DeviceEvent::Connected { keymap, .. } = &events[0] else { unreachable!() };
        assert_eq!(keymap.get(0, 0, 0), 0x0007);
    }

    #[test]
    fn resume_rereads_lock_state_after_vial_locks() {
        let sim = KeyboardSim::small();
        sim.with(|s| s.unlocked = true);
        let mut h = Harness::new(sim);
        assert_eq!(names(&h.steps(2, 0)), ["Connected", "Unlocked"]);

        *h.holders.lock().unwrap() = vec!["vial (4242)".into()];
        assert_eq!(names(&h.steps(1, 1000)), ["Paused"]);

        // Vial locks the keyboard again while we're paused and cannot see it.
        h.sim.with(|s| s.unlocked = false);
        h.holders.lock().unwrap().clear();
        assert_eq!(names(&h.steps(1, 2000)), ["Resumed", "Connected", "Locked"]);

        // Now Locked: the worker must not poll the matrix (it would read as echoed zeros).
        h.sim.with(|s| s.matrix[0] = 0b1);
        let before = h.sim.with(|s| s.matrix_requests);
        let events = h.steps(5, 2000);
        assert!(events.iter().all(|e| !matches!(e, DeviceEvent::Matrix { .. })), "{events:?}");
        assert_eq!(h.sim.with(|s| s.matrix_requests), before, "no matrix-state request while locked");
    }

    #[test]
    fn resume_into_unlock_in_progress_waits_for_the_unlock() {
        let mut h = Harness::new(KeyboardSim::small());
        assert_eq!(names(&h.steps(2, 0)), ["Connected", "Locked"]);

        *h.holders.lock().unwrap() = vec!["vial (7)".into()];
        assert_eq!(names(&h.steps(1, 1000)), ["Paused"]);

        // Vial starts (and leaves in progress) an unlock while we're paused.
        h.sim.with(|s| {
            s.unlock_in_progress = true;
            s.unlock_counter = 50;
            s.matrix[1] = 0b101; // the user is holding both unlock keys
        });
        h.holders.lock().unwrap().clear();

        let events = h.run(2000, 14_000);
        let n = names(&events);
        assert!(!n.contains(&"Error"), "{n:?}");
        let unlocked = n.iter().position(|&e| e == "Unlocked").expect("unlocked");
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..unlocked].iter().all(|&e| e == "Resumed" || e == "Unlocking"), "{n:?}");
        assert!(connected > unlocked);
        let DeviceEvent::Connected { keymap, .. } = &events[connected] else { unreachable!() };
        assert_eq!(keymap.get(0, 0, 0), 0x0004, "keymap must be read after the unlock completes");
    }

    #[test]
    fn persistent_load_error_is_reported_once() {
        // Rows/cols are set, but the keymap array is empty, so `Layout::from_definition`
        // fails deterministically every time `load()` runs.
        let bad_definition = r#"{"matrix":{"rows":2,"cols":3},"layouts":{"keymap":[]}}"#;
        let sim = KeyboardSim::new(bad_definition, 2, 2, 3, &SMALL_KEYMAP, &[(1, 0), (1, 2)]);
        let mut h = Harness::new(sim);

        let mut all = Vec::new();
        let mut wait = Duration::ZERO;
        for ms in [0u64, 5_000, 10_000, 15_000] {
            wait = h.worker.step(h.t0 + Duration::from_millis(ms));
            all.extend(h.rx.try_iter());
        }
        let n = names(&all);
        assert_eq!(n.iter().filter(|&&e| e == "Error").count(), 1, "{n:?}");
        assert!(!n.contains(&"Disconnected"), "{n:?}");
        assert_eq!(wait, RETRY_AFTER_ERROR);
    }

    #[test]
    fn keyboard_vanishing_before_the_first_reply_is_not_an_error() {
        let mut h = Harness::new(KeyboardSim::small());
        h.worker.connector.vanish_on_open = true;
        assert_eq!(names(&h.steps(2, 0)), ["Waiting"]);
    }

    #[test]
    fn an_identical_error_is_reported_again_after_replug() {
        let bad_definition = r#"{"matrix":{"rows":2,"cols":3},"layouts":{"keymap":[]}}"#;
        let sim = KeyboardSim::new(bad_definition, 2, 2, 3, &SMALL_KEYMAP, &[(1, 0), (1, 2)]);
        let mut h = Harness::new(sim);
        assert_eq!(names(&h.steps(2, 0)), ["Error"]);
        h.sim.unplug();
        assert_eq!(names(&h.steps(1, 5_000)), ["Waiting"]);
        h.sim.replug();
        assert_eq!(names(&h.steps(2, 6_000)), ["Error"], "a new keyboard gets its error shown");
    }
}
