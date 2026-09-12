# Lily58 Assistant Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A read-only desktop app that draws the user's Lily58 (Vial firmware) on screen, highlights pressed keys, shows the last key typed, and tracks the active layer, reading the keymap live from the keyboard.

**Architecture:** One Rust binary. A device worker thread owns the keyboard's raw-HID node (`/dev/hidrawN`). Every outgoing report passes through `ReadOnlyGuard`, whose allowlist admits only read commands plus Vial's two unlock-handshake commands. Three input tiers feed one `AppState`: egui key events while focused; `/dev/input` via evdev once the optional udev rule is installed; and Vial matrix polling once the keyboard is unlocked. An eframe/egui UI draws the keyboard from the layout definition the keyboard itself supplies.

**Tech Stack:** Rust 2024 edition, eframe/egui 0.36.2, evdev 0.13.2, lzma-rs 0.3.0, serde/serde_json/toml, thiserror/anyhow, log/env_logger, libc (only for `poll(2)`), tempfile (tests only).

**Spec:** `docs/superpowers/specs/2026-09-12-lily58-assistant-design.md`

## Global Constraints

- Edition 2024; `rust-version = "1.95"` in `Cargo.toml` (eframe 0.36.2's minimum). Fedora 44 ships rustc 1.98.1.
- No crate that links a system C library (no `hidapi`, `libudev`, `xz2`). `libc` is used only for `poll(2)`.
- Every byte sent to the keyboard goes through `hid::guard::ReadOnlyGuard`. The allowlist is exactly: `0x01`, `0x02` (any sub-byte), `0x11`, `0x12`, and `0xFE` followed by one of `0x00 0x01 0x02 0x05 0x06 0x07`. **Never add constants or code for any other command.**
- hidraw I/O: write 33 bytes (`0x00` report number, then the 32-byte report); reads return 32 bytes.
- Keyboard identity: USB serial contains `vial:f64c2b3c`; raw-HID report descriptor has usage page `0xFF60`, usage `0x61`. The Lily58's USB ID `7171:0012` appears only in the optional evdev udev rule.
- Vial protocol must be ≥ 6 (QMK's current keycode numbering).
- Replies: VIA commands (`0x01 0x02 0x11 0x12`) are echoed, so validate them by their leading bytes. Vial (`0xFE`) replies overwrite the buffer and **cannot** be validated.
- While a Vial unlock is in progress the firmware ignores every VIA command and echoes it back. It also answers the matrix-state request with an echo while locked, which reads as "nothing pressed". Nothing can cancel an unlock except completing it or replugging the keyboard.
- Timing: reply timeout 500 ms, one retry after 500 ms; hotplug scan 1 s; other-holder check 1 s; matrix poll 10 ms; locked-status check 2 s; unlock poll 50 ms; tapping term 200 ms.
- Config: `$XDG_CONFIG_HOME/lily58-assistant/config.toml`, falling back to `~/.config/lily58-assistant/config.toml`. Keys: `host_layout` (`"gb"` default, or `"us"`) and `tri_layer` (default `[1, 2, 3]`; `[]` disables emulation). A missing file means defaults.
- Out of scope: always-on-top, keymap caching, rotary encoders, any keymap write.
- Log with the `log` macros; users enable output with `RUST_LOG=debug`.
- `cargo test` must pass without the keyboard or a display. The Task 10 fixture capture and the Task 15 checks are the only steps that need hardware.

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock`, `.gitignore` | Crate definition, pinned dependencies |
| `.github/workflows/ci.yml` | Build, test and clippy on the latest Ubuntu runner image |
| `src/main.rs` | CLI entry: GUI by default, `--probe [--dump-definition FILE]` |
| `src/lib.rs` | Module list |
| `src/protocol.rs` | VIA/Vial command bytes, `Report` type, `report()` helper |
| `src/hid/mod.rs` | Raw-HID submodules |
| `src/hid/transport.rs` | `Transport` trait (32-byte report I/O) |
| `src/hid/guard.rs` | `ReadOnlyGuard` and `is_allowed` |
| `src/hid/fake.rs` | Test-only `FakeTransport` and `KeyboardSim` (simulated Vial firmware) |
| `src/hid/discover.rs` | Find the Vial hidraw node via sysfs; find other processes holding it |
| `src/hid/hidraw.rs` | `/dev/hidrawN` transport; only constructor returns a guarded handle |
| `src/vial.rs` | `VialClient`: typed Vial/VIA queries |
| `src/keycodes.rs` | QMK keycode decoding and labels |
| `src/layout.rs` | KLE layout → key geometry with matrix positions |
| `src/keymap.rs` | Keymap grid, transparency resolution, OS-key → position search |
| `src/layers.rs` | `LayerTracker` (QMK layer semantics) |
| `src/hostlayout.rs` | HID usage + shift → character, for gb/us |
| `src/hidmap.rs` | HID usage ↔ evdev keycode; egui key → HID usage |
| `src/input/mod.rs` | `OsKey`, `InputMsg` |
| `src/input/evdev.rs` | Find the keyboard's `/dev/input/eventN` nodes; reader threads |
| `src/input/focused.rs` | egui events → `OsKey`/text |
| `src/probe.rs` | `--probe` diagnostics |
| `src/device.rs` | Device worker thread: connect, load, poll, unlock, pause, hotplug |
| `src/state.rs` | `AppState`: held keys, last key, active layer, tier |
| `src/config.rs` | Config file loading |
| `src/hints.rs` | Setup instructions (single source for UI and README) |
| `src/ui/mod.rs` | eframe `App`, event plumbing |
| `src/ui/keyboard.rs` | Keyboard drawing, keycap labels |
| `src/ui/status.rs` | Status bar |
| `src/ui/dialogs.rs` | Unlock window, tiers/hints window |
| `README.md` | User documentation |
| `docs/manual-test-checklist.md` | Hardware/desktop checks |
| `tests/fixtures/lily58-definition.json` | Definition captured from the real keyboard |

---

### Task 1: Scaffold and smoke build

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/main.rs`, `src/lib.rs`, `.github/workflows/ci.yml`

**Interfaces:**
- Produces: crate `lily58_assistant` (lib) and binary `lily58-assistant`; every dependency used later is declared here, so this build also proves they all compile on this distro.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "lily58-assistant"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"
description = "Read-only on-screen companion for learning a Lily58 (Vial) keyboard"

[dependencies]
anyhow = "1.0.104"
eframe = "0.36.2"
env_logger = "0.11.11"
evdev = "0.13.2"
libc = "0.2.189"
log = "0.4.34"
lzma-rs = "0.3.0"
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.151"
thiserror = "2.0.20"
toml = "1.1.6"

[dev-dependencies]
tempfile = "3.27.0"
```

- [ ] **Step 2: Write `.gitignore`**

```
/target
```

- [ ] **Step 3: Write `src/lib.rs`**

```rust
//! Lily58 Assistant: a read-only on-screen companion for a Vial keyboard.
```

- [ ] **Step 4: Write the smoke-test `src/main.rs`**

```rust
use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lily58 Assistant")
            .with_app_id("lily58-assistant")
            .with_inner_size([960.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native("Lily58 Assistant", options, Box::new(|_cc| Ok(Box::new(Smoke))))
}

struct Smoke;

impl eframe::App for Smoke {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Lily58 Assistant: smoke build");
        });
    }
}
```

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: `Finished` with no errors. If a crate fails because a system `-dev` package is missing, install it (`sudo dnf install <pkg>`), rebuild, and **write down the package name**; it goes into the README in Task 13. Expect none: winit loads the Wayland/X11 libraries at runtime.

- [ ] **Step 6: Run the window**

Run: `timeout 5 cargo run; echo "exit=$?"`
Expected: a window titled "Lily58 Assistant" showing the heading appears on the desktop, and the command prints `exit=124` (killed by `timeout`). If the window fails to open with a wgpu or Vulkan error, switch renderer: change the dependency to `eframe = { version = "0.36.2", features = ["glow"] }`, add `renderer: eframe::Renderer::Glow,` to `NativeOptions`, and note this in the README in Task 13.

- [ ] **Step 7: Write `.github/workflows/ci.yml`**

```yaml
name: CI
on: [push, pull_request]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - run: cargo build --locked
      - run: cargo test --locked
      - run: cargo clippy --locked --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/main.rs src/lib.rs .github/workflows/ci.yml
git commit -m "chore: scaffold eframe app, pin dependencies, add CI"
```

---

### Task 2: Protocol constants and the read-only guard

**Files:**
- Create: `src/protocol.rs`, `src/hid/mod.rs`, `src/hid/transport.rs`, `src/hid/guard.rs`, `src/hid/fake.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `protocol::{REPORT_LEN: usize = 32, Report = [u8; 32], report(&[u8]) -> Report}` and the command constants below.
  - `hid::transport::Transport` trait: `write_report(&mut self, &Report) -> io::Result<()>`, `read_report(&mut self, Duration) -> io::Result<Option<Report>>` (`None` = timeout); also implemented for `Box<T>`.
  - `hid::guard::{is_allowed(&Report) -> bool, ReadOnlyGuard<T>, GuardError}`: `ReadOnlyGuard::new(T)`, `send(&mut self, &Report) -> Result<(), GuardError>`, `recv(&mut self, Duration) -> io::Result<Option<Report>>`, `boxed(self) -> ReadOnlyGuard<Box<dyn Transport>>`.
  - Test-only `hid::fake::FakeTransport`: `new(responder)`, `with_unplug_flag(responder, Arc<Mutex<bool>>)`, `written() -> Arc<Mutex<Vec<Report>>>`, `push_unsolicited(Report)`.

- [ ] **Step 1: Write `src/protocol.rs`**

```rust
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
```

- [ ] **Step 2: Write `src/hid/transport.rs`**

```rust
use std::io;
use std::time::Duration;

use crate::protocol::Report;

/// A raw-HID channel carrying fixed 32-byte reports.
/// Only `ReadOnlyGuard` should hold one; see `guard.rs`.
pub trait Transport: Send {
    fn write_report(&mut self, report: &Report) -> io::Result<()>;
    /// Waits up to `timeout` for one input report; `Ok(None)` on timeout.
    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn write_report(&mut self, report: &Report) -> io::Result<()> {
        (**self).write_report(report)
    }

    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        (**self).read_report(timeout)
    }
}
```

- [ ] **Step 3: Write `src/hid/fake.rs`**

```rust
//! Test doubles for the raw-HID layer.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::hid::transport::Transport;
use crate::protocol::Report;

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
```

- [ ] **Step 4: Write `src/hid/mod.rs` and register modules**

`src/hid/mod.rs`:

```rust
//! Raw-HID access. Everything that reaches the keyboard goes through `guard::ReadOnlyGuard`.

#[cfg(test)]
pub mod fake;
pub mod guard;
pub mod transport;
```

Append to `src/lib.rs`:

```rust
pub mod hid;
pub mod protocol;
```

- [ ] **Step 5: Write the failing guard tests in `src/hid/guard.rs`**

```rust
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
```

- [ ] **Step 6: Run the tests to verify they fail**

Run: `cargo test --lib hid::guard`
Expected: compile errors: `is_allowed`, `ReadOnlyGuard`, `GuardError` not found.

- [ ] **Step 7: Implement the guard (top of `src/hid/guard.rs`, above the tests)**

```rust
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
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib hid::guard`
Expected: 4 passed.

- [ ] **Step 9: Commit**

```bash
git add src/protocol.rs src/hid src/lib.rs
git commit -m "feat: read-only guard with exhaustive allowlist tests"
```

---

### Task 3: Device discovery and the hidraw transport

**Files:**
- Create: `src/hid/discover.rs`, `src/hid/hidraw.rs`
- Modify: `src/hid/mod.rs`

**Interfaces:**
- Consumes: `Transport`, `ReadOnlyGuard` (Task 2).
- Produces:
  - `hid::discover::VialDevice { dev_node: PathBuf, usb_dir: PathBuf, vendor_id: u16, product_id: u16, product: String }` (`Debug, Clone, PartialEq, Eq`).
  - `find_vial_device(sys_root: &Path, dev_root: &Path) -> io::Result<Option<VialDevice>>`.
  - `usb_device_dir(&Path) -> Option<PathBuf>`.
  - `is_vial_raw_interface(&[u8]) -> bool`.
  - `other_holders(proc_root: &Path, dev_node: &Path, own_pid: u32) -> Vec<String>` (entries look like `"vial (1234)"`).
  - `VIAL_SERIAL_MARKER: &str`.
  - `hid::hidraw::{Hidraw, open_guarded(&Path) -> io::Result<ReadOnlyGuard<Hidraw>>}`. `Hidraw` has no other constructor.

- [ ] **Step 1: Write failing tests at the bottom of `src/hid/discover.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    // Real descriptors read from the Lily58 (sysfs report_descriptor of hidraw15 / hidraw14).
    const RAW_DESCRIPTOR: [u8; 34] = [
        0x06, 0x60, 0xff, 0x09, 0x61, 0xa1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95, 0x20, 0x75,
        0x08, 0x81, 0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95, 0x20, 0x75, 0x08, 0x91, 0x02, 0xc0,
    ];
    const KEYBOARD_DESCRIPTOR: [u8; 7] = [0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0xc0];

    fn fake_sys(serial: &str) -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let usb = root.join("devices/usb1/1-3");
        let hid_kbd = usb.join("1-3:1.0/0003:7171:0012.0001");
        let hid_raw = usb.join("1-3:1.1/0003:7171:0012.0002");
        fs::create_dir_all(&hid_kbd).unwrap();
        fs::create_dir_all(&hid_raw).unwrap();
        fs::write(usb.join("idVendor"), "7171\n").unwrap();
        fs::write(usb.join("idProduct"), "0012\n").unwrap();
        fs::write(usb.join("serial"), format!("{serial}\n")).unwrap();
        fs::write(usb.join("product"), "Lily58 Pro R2G\n").unwrap();
        fs::write(hid_kbd.join("report_descriptor"), KEYBOARD_DESCRIPTOR).unwrap();
        fs::write(hid_raw.join("report_descriptor"), RAW_DESCRIPTOR).unwrap();
        for (name, target) in [("hidraw14", &hid_kbd), ("hidraw15", &hid_raw)] {
            let dir = root.join("class/hidraw").join(name);
            fs::create_dir_all(&dir).unwrap();
            symlink(target, dir.join("device")).unwrap();
        }
        t
    }

    #[test]
    fn finds_the_raw_interface_not_the_keyboard_interface() {
        let sys = fake_sys("vial:f64c2b3c");
        let dev = find_vial_device(sys.path(), Path::new("/dev")).unwrap().unwrap();
        assert_eq!(dev.dev_node, PathBuf::from("/dev/hidraw15"));
        assert_eq!((dev.vendor_id, dev.product_id), (0x7171, 0x0012));
        assert_eq!(dev.product, "Lily58 Pro R2G");
        assert_eq!(dev.usb_dir, fs::canonicalize(sys.path().join("devices/usb1/1-3")).unwrap());
    }

    #[test]
    fn ignores_devices_without_the_vial_serial() {
        let sys = fake_sys("0123456789");
        assert_eq!(find_vial_device(sys.path(), Path::new("/dev")).unwrap(), None);
    }

    #[test]
    fn missing_hidraw_class_means_no_device() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(find_vial_device(t.path(), Path::new("/dev")).unwrap(), None);
    }

    #[test]
    fn descriptor_parsing() {
        assert!(is_vial_raw_interface(&RAW_DESCRIPTOR));
        assert!(!is_vial_raw_interface(&KEYBOARD_DESCRIPTOR));
        assert!(!is_vial_raw_interface(&[0x06, 0x60])); // truncated
    }

    #[test]
    fn finds_other_processes_holding_the_node() {
        let t = tempfile::tempdir().unwrap();
        let proc_root = t.path();
        for (pid, name, target) in [(100, "vial", "/dev/hidraw15"), (200, "me", "/dev/hidraw15"), (300, "bash", "/dev/null")] {
            let fd = proc_root.join(pid.to_string()).join("fd");
            fs::create_dir_all(&fd).unwrap();
            fs::write(proc_root.join(pid.to_string()).join("comm"), format!("{name}\n")).unwrap();
            symlink(target, fd.join("3")).unwrap();
        }
        let holders = other_holders(proc_root, Path::new("/dev/hidraw15"), 200);
        assert_eq!(holders, vec!["vial (100)".to_string()]);
    }
}
```

- [ ] **Step 2: Register the modules and run the tests to verify they fail**

Replace `src/hid/mod.rs` with:

```rust
//! Raw-HID access. Everything that reaches the keyboard goes through `guard::ReadOnlyGuard`.

pub mod discover;
#[cfg(test)]
pub mod fake;
pub mod guard;
pub mod hidraw;
pub mod transport;
```

Create an empty `src/hid/hidraw.rs` for now.

Run: `cargo test --lib hid::discover`
Expected: compile errors: `find_vial_device` etc. not found.

- [ ] **Step 3: Implement `src/hid/discover.rs` (above the tests)**

```rust
//! Finds the keyboard's Vial raw-HID node through sysfs.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Vial firmware puts this in the USB serial number.
pub const VIAL_SERIAL_MARKER: &str = "vial:f64c2b3c";
const RAW_USAGE_PAGE: u32 = 0xFF60;
const RAW_USAGE: u32 = 0x61;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VialDevice {
    /// `/dev/hidrawN`
    pub dev_node: PathBuf,
    /// Canonical sysfs directory of the USB device (the one holding `idVendor` and `serial`).
    pub usb_dir: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
}

/// First Vial raw-HID interface. `sys_root` is normally `/sys`, `dev_root` `/dev`.
pub fn find_vial_device(sys_root: &Path, dev_root: &Path) -> io::Result<Option<VialDevice>> {
    let class = sys_root.join("class/hidraw");
    let mut names: Vec<_> = match fs::read_dir(&class) {
        Ok(entries) => entries.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    names.sort();
    for name in names {
        let Ok(hid_dir) = fs::canonicalize(class.join(&name).join("device")) else { continue };
        let Ok(descriptor) = fs::read(hid_dir.join("report_descriptor")) else { continue };
        if !is_vial_raw_interface(&descriptor) {
            continue;
        }
        let Some(usb_dir) = usb_device_dir(&hid_dir) else { continue };
        if !read_trimmed(&usb_dir.join("serial")).unwrap_or_default().contains(VIAL_SERIAL_MARKER) {
            continue;
        }
        return Ok(Some(VialDevice {
            dev_node: dev_root.join(&name),
            vendor_id: read_hex(&usb_dir.join("idVendor")).unwrap_or(0),
            product_id: read_hex(&usb_dir.join("idProduct")).unwrap_or(0),
            product: read_trimmed(&usb_dir.join("product")).unwrap_or_default(),
            usb_dir,
        }));
    }
    Ok(None)
}

/// Walks up from a sysfs device path to the USB device directory (the one with `idVendor`).
pub fn usb_device_dir(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|p| p.join("idVendor").is_file()).map(Path::to_path_buf)
}

/// True if a HID report descriptor declares usage page 0xFF60 with usage 0x61 (QMK raw HID).
pub fn is_vial_raw_interface(descriptor: &[u8]) -> bool {
    let (mut page, mut saw_page, mut saw_usage) = (0u32, false, false);
    let mut i = 0;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        if prefix == 0xFE {
            // Long item: 0xFE, data size, tag, data.
            i += 3 + *descriptor.get(i + 1).unwrap_or(&0) as usize;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let data = descriptor.get(i + 1..i + 1 + size).unwrap_or(&[]);
        let value = data.iter().rev().fold(0u32, |acc, &b| (acc << 8) | b as u32);
        match prefix & 0xFC {
            0x04 => {
                // Usage Page (global item)
                page = value;
                saw_page |= page == RAW_USAGE_PAGE;
            }
            0x08 => saw_usage |= page == RAW_USAGE_PAGE && value == RAW_USAGE, // Usage (local item)
            _ => {}
        }
        i += 1 + size;
    }
    saw_page && saw_usage
}

/// Processes other than `own_pid` that hold `dev_node` open, as `"name (pid)"`.
/// Processes of other users are unreadable and silently skipped.
pub fn other_holders(proc_root: &Path, dev_node: &Path, own_pid: u32) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(procs) = fs::read_dir(proc_root) else { return found };
    for entry in procs.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == own_pid {
            continue;
        }
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|target| target == dev_node)) {
            let name = read_trimmed(&entry.path().join("comm")).unwrap_or_else(|| "?".into());
            found.push(format!("{name} ({pid})"));
        }
    }
    found.sort();
    found
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

fn read_hex(path: &Path) -> Option<u16> {
    u16::from_str_radix(&read_trimmed(path)?, 16).ok()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib hid::discover`
Expected: 5 passed.

- [ ] **Step 5: Implement `src/hid/hidraw.rs`**

This is thin OS glue with no unit test; Task 10's `--probe` exercises it against the real keyboard.

```rust
//! `/dev/hidrawN` transport.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use crate::hid::guard::ReadOnlyGuard;
use crate::hid::transport::Transport;
use crate::protocol::{REPORT_LEN, Report};

/// Deliberately has no public constructor: `open_guarded` is the only way to get one,
/// and it hands the transport straight to the read-only guard.
pub struct Hidraw {
    file: File,
}

pub fn open_guarded(path: &Path) -> io::Result<ReadOnlyGuard<Hidraw>> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    Ok(ReadOnlyGuard::new(Hidraw { file }))
}

impl Transport for Hidraw {
    fn write_report(&mut self, report: &Report) -> io::Result<()> {
        // hidraw wants the report number first; QMK's raw-HID interface uses unnumbered reports (0).
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(report);
        let n = self.file.write(&buf)?;
        if n != buf.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, format!("short hidraw write ({n} bytes)")));
        }
        Ok(())
    }

    fn read_report(&mut self, timeout: Duration) -> io::Result<Option<Report>> {
        let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `pfd` is a valid pollfd for the duration of the call, and nfds is 1.
        let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
        if rc < 0 {
            let err = io::Error::last_os_error();
            return if err.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(err) };
        }
        if rc == 0 {
            return Ok(None);
        }
        if pfd.revents & libc::POLLIN == 0 {
            // POLLHUP / POLLERR / POLLNVAL without data: the device went away.
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "hidraw device gone"));
        }
        let mut report = [0u8; REPORT_LEN];
        let n = self.file.read(&mut report)?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "hidraw device gone"));
        }
        Ok(Some(report))
    }
}
```

- [ ] **Step 6: Build and run all tests**

Run: `cargo test`
Expected: all tests pass; no warnings about `hidraw.rs`.

- [ ] **Step 7: Commit**

```bash
git add src/hid
git commit -m "feat: sysfs discovery of the Vial hidraw node and guarded hidraw transport"
```

---

### Task 4: QMK keycode decoding

**Files:**
- Create: `src/keycodes.rs`
- Modify: `src/lib.rs` (add `pub mod keycodes;`)

**Interfaces:**
- Produces:
  - `keycodes::{KC_NO: u16 = 0, KC_TRNS: u16 = 1, Action, decode(u16) -> Action}`.
  - `label(u16) -> String`, `basic_name(u8) -> Option<&'static str>`, `mods_name(u8) -> String`.
  - `tap_basic(u16) -> Option<u8>`: the HID usage sent when the key is tapped; `None` for non-sending keys and for `KC_NO`/`KC_TRNS`.
  - `adds_shift(u16) -> bool`.
  - `Action` variants: `Basic(u8)`, `Modded { mods, basic }`, `ModTap { mods, basic }`, `LayerTap { layer, basic }`, `LayerMod { layer, mods }`, `To(u8)`, `Momentary(u8)`, `DefaultLayer(u8)`, `Toggle(u8)`, `OneShotLayer(u8)`, `OneShotMod(u8)`, `TapToggle(u8)`, `PersistentDefault(u8)`, `TriLayerLower`, `TriLayerUpper`, `Other(u16)`.

- [ ] **Step 1: Write failing tests at the bottom of `src/keycodes.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_qmk_ranges() {
        let cases: [(u16, Action); 15] = [
            (0x0004, Action::Basic(0x04)),                              // KC_A
            (0x0106, Action::Modded { mods: 0x01, basic: 0x06 }),       // LCTL(KC_C)
            (0x121E, Action::Modded { mods: 0x12, basic: 0x1E }),       // RSFT(KC_1)
            (0x2204, Action::ModTap { mods: 0x02, basic: 0x04 }),       // MT(MOD_LSFT, KC_A)
            (0x422C, Action::LayerTap { layer: 2, basic: 0x2C }),       // LT(2, KC_SPC)
            (0x5021, Action::LayerMod { layer: 1, mods: 0x01 }),        // LM(1, MOD_LCTL)
            (0x5203, Action::To(3)),
            (0x5221, Action::Momentary(1)),
            (0x5240, Action::DefaultLayer(0)),
            (0x5262, Action::Toggle(2)),
            (0x5281, Action::OneShotLayer(1)),
            (0x52A2, Action::OneShotMod(0x02)),
            (0x52C1, Action::TapToggle(1)),
            (0x7C77, Action::TriLayerLower),
            (0x7C00, Action::Other(0x7C00)),                            // QK_BOOT
        ];
        for (code, action) in cases {
            assert_eq!(decode(code), action, "{code:#06x}");
        }
    }

    #[test]
    fn labels() {
        assert_eq!(label(0x0004), "KC_A");
        assert_eq!(label(0x0001), "KC_TRNS");
        assert_eq!(label(0x0106), "LCTL(KC_C)");
        assert_eq!(label(0x121E), "RSFT(KC_1)");
        assert_eq!(label(0x2204), "MT(LSFT,KC_A)");
        assert_eq!(label(0x422C), "LT(2,KC_SPC)");
        assert_eq!(label(0x5221), "MO(1)");
        assert_eq!(label(0x7C78), "TL_UPPR");
        assert_eq!(label(0x7C00), "0x7c00");
        assert_eq!(mods_name(0x03), "LCTL|LSFT");
    }

    #[test]
    fn tap_basic_and_shift() {
        assert_eq!(tap_basic(0x0004), Some(0x04));
        assert_eq!(tap_basic(0x021E), Some(0x1E)); // KC_EXLM = LSFT(KC_1)
        assert_eq!(tap_basic(0x422C), Some(0x2C));
        assert_eq!(tap_basic(KC_NO), None);
        assert_eq!(tap_basic(KC_TRNS), None);
        assert_eq!(tap_basic(0x5221), None);
        assert!(adds_shift(0x021E));
        assert!(!adds_shift(0x0106));
    }
}
```

- [ ] **Step 2: Add `pub mod keycodes;` to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib keycodes`
Expected: compile errors (items not found).

- [ ] **Step 3: Implement `src/keycodes.rs` (above the tests)**

```rust
//! QMK keycodes as stored by Vial protocol v6+ (vial-qmk `quantum/keycodes.h`).

pub const KC_NO: u16 = 0x0000;
pub const KC_TRNS: u16 = 0x0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Basic(u8),
    Modded { mods: u8, basic: u8 },
    ModTap { mods: u8, basic: u8 },
    LayerTap { layer: u8, basic: u8 },
    LayerMod { layer: u8, mods: u8 },
    To(u8),
    Momentary(u8),
    DefaultLayer(u8),
    Toggle(u8),
    OneShotLayer(u8),
    OneShotMod(u8),
    TapToggle(u8),
    PersistentDefault(u8),
    TriLayerLower,
    TriLayerUpper,
    Other(u16),
}

pub fn decode(code: u16) -> Action {
    let low = (code & 0xFF) as u8;
    let low5 = (code & 0x1F) as u8;
    let mods8 = ((code >> 8) & 0x1F) as u8;
    match code {
        0x0000..=0x00FF => Action::Basic(low),
        0x0100..=0x1FFF => Action::Modded { mods: mods8, basic: low },
        0x2000..=0x3FFF => Action::ModTap { mods: mods8, basic: low },
        0x4000..=0x4FFF => Action::LayerTap { layer: ((code >> 8) & 0x0F) as u8, basic: low },
        0x5000..=0x51FF => Action::LayerMod { layer: ((code >> 5) & 0x0F) as u8, mods: low5 },
        0x5200..=0x521F => Action::To(low5),
        0x5220..=0x523F => Action::Momentary(low5),
        0x5240..=0x525F => Action::DefaultLayer(low5),
        0x5260..=0x527F => Action::Toggle(low5),
        0x5280..=0x529F => Action::OneShotLayer(low5),
        0x52A0..=0x52BF => Action::OneShotMod(low5),
        0x52C0..=0x52DF => Action::TapToggle(low5),
        0x52E0..=0x52FF => Action::PersistentDefault(low5),
        0x7C77 => Action::TriLayerLower,
        0x7C78 => Action::TriLayerUpper,
        _ => Action::Other(code),
    }
}

/// QMK-style name, e.g. `KC_A`, `LCTL(KC_C)`, `LT(2,KC_SPC)`, `MO(1)`; unknown codes as hex.
pub fn label(code: u16) -> String {
    match decode(code) {
        Action::Basic(b) => basic_name(b).map(str::to_owned).unwrap_or_else(|| format!("{code:#06x}")),
        Action::Modded { mods, basic } => format!("{}({})", mods_name(mods), label(basic as u16)),
        Action::ModTap { mods, basic } => format!("MT({},{})", mods_name(mods), label(basic as u16)),
        Action::LayerTap { layer, basic } => format!("LT({layer},{})", label(basic as u16)),
        Action::LayerMod { layer, mods } => format!("LM({layer},{})", mods_name(mods)),
        Action::To(l) => format!("TO({l})"),
        Action::Momentary(l) => format!("MO({l})"),
        Action::DefaultLayer(l) => format!("DF({l})"),
        Action::Toggle(l) => format!("TG({l})"),
        Action::OneShotLayer(l) => format!("OSL({l})"),
        Action::OneShotMod(m) => format!("OSM({})", mods_name(m)),
        Action::TapToggle(l) => format!("TT({l})"),
        Action::PersistentDefault(l) => format!("PDF({l})"),
        Action::TriLayerLower => "TL_LOWR".into(),
        Action::TriLayerUpper => "TL_UPPR".into(),
        Action::Other(c) => format!("{c:#06x}"),
    }
}

/// 5-bit QMK modifier mask (bit 4 = right-hand) as `LCTL|LSFT` etc.
pub fn mods_name(mods: u8) -> String {
    let side = if mods & 0x10 != 0 { 'R' } else { 'L' };
    let parts: Vec<String> = [(0x01, "CTL"), (0x02, "SFT"), (0x04, "ALT"), (0x08, "GUI")]
        .into_iter()
        .filter(|(bit, _)| mods & bit != 0)
        .map(|(_, name)| format!("{side}{name}"))
        .collect();
    if parts.is_empty() { "NONE".into() } else { parts.join("|") }
}

/// The HID usage a key sends when tapped, if it sends one.
pub fn tap_basic(code: u16) -> Option<u8> {
    match decode(code) {
        Action::Basic(b)
        | Action::Modded { basic: b, .. }
        | Action::ModTap { basic: b, .. }
        | Action::LayerTap { basic: b, .. }
            if b >= 0x04 =>
        {
            Some(b)
        }
        _ => None,
    }
}

/// True if the keycode itself applies Shift (e.g. `KC_EXLM` = `LSFT(KC_1)`).
pub fn adds_shift(code: u16) -> bool {
    matches!(decode(code), Action::Modded { mods, .. } if mods & 0x02 != 0)
}

pub fn basic_name(code: u8) -> Option<&'static str> {
    BASIC_NAMES.iter().find(|(c, _)| *c == code).map(|(_, name)| *name)
}

/// Short QMK names for the basic range, generated from vial-qmk `quantum/keycodes.h`.
const BASIC_NAMES: &[(u8, &str)] = &[
    (0x00, "KC_NO"), (0x01, "KC_TRNS"), (0x04, "KC_A"), (0x05, "KC_B"), (0x06, "KC_C"), (0x07, "KC_D"),
    (0x08, "KC_E"), (0x09, "KC_F"), (0x0A, "KC_G"), (0x0B, "KC_H"), (0x0C, "KC_I"), (0x0D, "KC_J"),
    (0x0E, "KC_K"), (0x0F, "KC_L"), (0x10, "KC_M"), (0x11, "KC_N"), (0x12, "KC_O"), (0x13, "KC_P"),
    (0x14, "KC_Q"), (0x15, "KC_R"), (0x16, "KC_S"), (0x17, "KC_T"), (0x18, "KC_U"), (0x19, "KC_V"),
    (0x1A, "KC_W"), (0x1B, "KC_X"), (0x1C, "KC_Y"), (0x1D, "KC_Z"), (0x1E, "KC_1"), (0x1F, "KC_2"),
    (0x20, "KC_3"), (0x21, "KC_4"), (0x22, "KC_5"), (0x23, "KC_6"), (0x24, "KC_7"), (0x25, "KC_8"),
    (0x26, "KC_9"), (0x27, "KC_0"), (0x28, "KC_ENT"), (0x29, "KC_ESC"), (0x2A, "KC_BSPC"), (0x2B, "KC_TAB"),
    (0x2C, "KC_SPC"), (0x2D, "KC_MINS"), (0x2E, "KC_EQL"), (0x2F, "KC_LBRC"), (0x30, "KC_RBRC"),
    (0x31, "KC_BSLS"), (0x32, "KC_NUHS"), (0x33, "KC_SCLN"), (0x34, "KC_QUOT"), (0x35, "KC_GRV"),
    (0x36, "KC_COMM"), (0x37, "KC_DOT"), (0x38, "KC_SLSH"), (0x39, "KC_CAPS"), (0x3A, "KC_F1"),
    (0x3B, "KC_F2"), (0x3C, "KC_F3"), (0x3D, "KC_F4"), (0x3E, "KC_F5"), (0x3F, "KC_F6"), (0x40, "KC_F7"),
    (0x41, "KC_F8"), (0x42, "KC_F9"), (0x43, "KC_F10"), (0x44, "KC_F11"), (0x45, "KC_F12"),
    (0x46, "KC_PSCR"), (0x47, "KC_SCRL"), (0x48, "KC_PAUS"), (0x49, "KC_INS"), (0x4A, "KC_HOME"),
    (0x4B, "KC_PGUP"), (0x4C, "KC_DEL"), (0x4D, "KC_END"), (0x4E, "KC_PGDN"), (0x4F, "KC_RGHT"),
    (0x50, "KC_LEFT"), (0x51, "KC_DOWN"), (0x52, "KC_UP"), (0x53, "KC_NUM"), (0x54, "KC_PSLS"),
    (0x55, "KC_PAST"), (0x56, "KC_PMNS"), (0x57, "KC_PPLS"), (0x58, "KC_PENT"), (0x59, "KC_P1"),
    (0x5A, "KC_P2"), (0x5B, "KC_P3"), (0x5C, "KC_P4"), (0x5D, "KC_P5"), (0x5E, "KC_P6"), (0x5F, "KC_P7"),
    (0x60, "KC_P8"), (0x61, "KC_P9"), (0x62, "KC_P0"), (0x63, "KC_PDOT"), (0x64, "KC_NUBS"),
    (0x65, "KC_APP"), (0x66, "KC_KB_POWER"), (0x67, "KC_PEQL"), (0x68, "KC_F13"), (0x69, "KC_F14"),
    (0x6A, "KC_F15"), (0x6B, "KC_F16"), (0x6C, "KC_F17"), (0x6D, "KC_F18"), (0x6E, "KC_F19"),
    (0x6F, "KC_F20"), (0x70, "KC_F21"), (0x71, "KC_F22"), (0x72, "KC_F23"), (0x73, "KC_F24"),
    (0x74, "KC_EXEC"), (0x75, "KC_HELP"), (0x76, "KC_MENU"), (0x77, "KC_SLCT"), (0x78, "KC_STOP"),
    (0x79, "KC_AGIN"), (0x7A, "KC_UNDO"), (0x7B, "KC_CUT"), (0x7C, "KC_COPY"), (0x7D, "KC_PSTE"),
    (0x7E, "KC_FIND"), (0x7F, "KC_KB_MUTE"), (0x80, "KC_KB_VOLUME_UP"), (0x81, "KC_KB_VOLUME_DOWN"),
    (0x85, "KC_PCMM"), (0x87, "KC_INT1"), (0x88, "KC_INT2"), (0x89, "KC_INT3"), (0x8A, "KC_INT4"),
    (0x8B, "KC_INT5"), (0x8C, "KC_INT6"), (0x8D, "KC_INT7"), (0x8E, "KC_INT8"), (0x8F, "KC_INT9"),
    (0x90, "KC_LNG1"), (0x91, "KC_LNG2"), (0x92, "KC_LNG3"), (0x93, "KC_LNG4"), (0x94, "KC_LNG5"),
    (0x95, "KC_LNG6"), (0x96, "KC_LNG7"), (0x97, "KC_LNG8"), (0x98, "KC_LNG9"), (0x99, "KC_ERAS"),
    (0x9A, "KC_SYRQ"), (0x9B, "KC_CNCL"), (0x9C, "KC_CLR"), (0x9D, "KC_PRIR"), (0x9E, "KC_RETN"),
    (0x9F, "KC_SEPR"), (0xA0, "KC_OUT"), (0xA1, "KC_OPER"), (0xA2, "KC_CLAG"), (0xA3, "KC_CRSL"),
    (0xA4, "KC_EXSL"), (0xA5, "KC_PWR"), (0xA6, "KC_SLEP"), (0xA7, "KC_WAKE"), (0xA8, "KC_MUTE"),
    (0xA9, "KC_VOLU"), (0xAA, "KC_VOLD"), (0xAB, "KC_MNXT"), (0xAC, "KC_MPRV"), (0xAD, "KC_MSTP"),
    (0xAE, "KC_MPLY"), (0xAF, "KC_MSEL"), (0xB0, "KC_EJCT"), (0xB1, "KC_MAIL"), (0xB2, "KC_CALC"),
    (0xB3, "KC_MYCM"), (0xB4, "KC_WSCH"), (0xB5, "KC_WHOM"), (0xB6, "KC_WBAK"), (0xB7, "KC_WFWD"),
    (0xB8, "KC_WSTP"), (0xB9, "KC_WREF"), (0xBA, "KC_WFAV"), (0xBB, "KC_MFFD"), (0xBC, "KC_MRWD"),
    (0xBD, "KC_BRIU"), (0xBE, "KC_BRID"), (0xBF, "KC_CPNL"), (0xC0, "KC_ASST"), (0xC1, "KC_MCTL"),
    (0xC2, "KC_LPAD"), (0xCD, "MS_UP"), (0xCE, "MS_DOWN"), (0xCF, "MS_LEFT"), (0xD0, "MS_RGHT"),
    (0xD1, "MS_BTN1"), (0xD2, "MS_BTN2"), (0xD3, "MS_BTN3"), (0xD4, "MS_BTN4"), (0xD5, "MS_BTN5"),
    (0xD6, "MS_BTN6"), (0xD7, "MS_BTN7"), (0xD8, "MS_BTN8"), (0xD9, "MS_WHLU"), (0xDA, "MS_WHLD"),
    (0xDB, "MS_WHLL"), (0xDC, "MS_WHLR"), (0xDD, "MS_ACL0"), (0xDE, "MS_ACL1"), (0xDF, "MS_ACL2"),
    (0xE0, "KC_LCTL"), (0xE1, "KC_LSFT"), (0xE2, "KC_LALT"), (0xE3, "KC_LGUI"), (0xE4, "KC_RCTL"),
    (0xE5, "KC_RSFT"), (0xE6, "KC_RALT"), (0xE7, "KC_RGUI"),
];
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib keycodes`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/keycodes.rs src/lib.rs
git commit -m "feat: decode and label QMK keycodes"
```

---

### Task 5: Layout parser (Vial KLE definition → key geometry)

**Files:**
- Create: `src/layout.rs`
- Modify: `src/lib.rs` (add `pub mod layout;`)

**Interfaces:**
- Produces:
  - `layout::KeyGeom { row: u8, col: u8, x: f32, y: f32, w: f32, h: f32, angle: f32, rx: f32, ry: f32 }` (`Debug, Clone, PartialEq`); positions are in key units, `angle` in degrees clockwise about `(rx, ry)`. Methods: `corners() -> [(f32, f32); 4]` (rotated; TL, TR, BR, BL) and `center() -> (f32, f32)`.
  - `layout::Layout { name: String, rows: u8, cols: u8, keys: Vec<KeyGeom> }` (`Debug, Clone, PartialEq`). Methods: `from_definition(&serde_json::Value) -> Result<Layout, LayoutError>`, `bounds() -> (f32, f32, f32, f32)` (min_x, min_y, max_x, max_y), `key(row, col) -> Option<&KeyGeom>`.
  - `layout::LayoutError` (thiserror).

Parsing follows vial-gui's `kle_serial.py`:
- Rows are arrays; a leading object row is metadata and is skipped.
- Object items set properties: `r`, `rx`, `ry`, `a`, `x`, `y`, `w`, `h`, `d`, processed in that order.
- Setting `rx` or `ry` resets the cursor to `(rx, ry)`.
- After each key, `x += w` and `w`, `h` and the decal flag reset.
- After each row, `y += 1` and `x = rx`.
- Labels are remapped by alignment. The label that lands in slot 0 is `"row,col"`, slot 4 = `"e"` marks an encoder (skipped), and slot 8 = `"idx,opt"` is a layout option (only option 0 is kept). Decals are skipped.

- [ ] **Step 1: Write failing tests at the bottom of `src/layout.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(keymap: Value, rows: u8, cols: u8) -> Value {
        json!({ "name": "Test", "matrix": { "rows": rows, "cols": cols }, "layouts": { "keymap": keymap } })
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn places_keys_left_to_right_and_row_by_row() {
        let layout = Layout::from_definition(&def(json!([["0,0", "0,1"], [{ "w": 2 }, "1,0"]]), 2, 2)).unwrap();
        let k: Vec<_> = layout.keys.iter().map(|k| (k.row, k.col, k.x, k.y, k.w)).collect();
        assert_eq!(k, vec![(0, 0, 0.0, 0.0, 1.0), (0, 1, 1.0, 0.0, 1.0), (1, 0, 0.0, 1.0, 2.0)]);
        assert_eq!(layout.name, "Test");
    }

    #[test]
    fn handles_lily58_style_offsets() {
        let km = json!([[{ "x": 3.5 }, "0,3", { "x": 8.5 }, "5,3"], [{ "y": -0.875, "x": 2.5 }, "0,2"]]);
        let layout = Layout::from_definition(&def(km, 10, 6)).unwrap();
        let (a, b, c) = (layout.key(0, 3).unwrap(), layout.key(5, 3).unwrap(), layout.key(0, 2).unwrap());
        assert!(close(a.x, 3.5) && close(a.y, 0.0));
        assert!(close(b.x, 13.0) && close(b.y, 0.0));
        assert!(close(c.x, 2.5) && close(c.y, 0.125));
    }

    #[test]
    fn rotation_about_rx_ry() {
        let km = json!([[{ "r": 15, "rx": 4, "ry": 3, "y": -1, "x": 1 }, "0,0"]]);
        let layout = Layout::from_definition(&def(km, 1, 1)).unwrap();
        let k = &layout.keys[0];
        assert!(close(k.x, 5.0) && close(k.y, 2.0) && close(k.angle, 15.0));
        let (x, y) = k.corners()[0];
        assert!(close(x, 5.2247) && close(y, 2.2929), "got {x},{y}");
    }

    #[test]
    fn skips_metadata_decals_encoders_and_non_default_options() {
        let km = json!([
            { "name": "meta" },
            ["0,0", { "d": true }, "", "0,1\n\n\n\n\n\n\n\n\ne", "0,2\n\n\n1,1"],
            ["1,0"]
        ]);
        let layout = Layout::from_definition(&def(km, 2, 3)).unwrap();
        let rc: Vec<_> = layout.keys.iter().map(|k| (k.row, k.col)).collect();
        assert_eq!(rc, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn rejects_keys_outside_the_matrix_and_missing_fields() {
        assert!(Layout::from_definition(&def(json!([["2,0"]]), 2, 2)).is_err());
        assert!(Layout::from_definition(&json!({ "layouts": { "keymap": [] } })).is_err());
    }

    #[test]
    fn bounds_cover_all_corners() {
        let layout = Layout::from_definition(&def(json!([["0,0", { "w": 1.5 }, "0,1"]]), 1, 2)).unwrap();
        assert_eq!(layout.bounds(), (0.0, 0.0, 2.5, 1.0));
    }
}
```

With the default alignment (4), label line 9 lands in slot 4 and line 3 in slot 8. So `"0,1\n…\ne"` is an encoder, and `"0,2\n\n\n1,1"` is option 1 of layout group 1. Both are skipped.

- [ ] **Step 2: Add `pub mod layout;` to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib layout`
Expected: compile errors (items not found).

- [ ] **Step 3: Implement `src/layout.rs` (above the tests)**

```rust
//! Physical key layout from the Vial definition (KLE format, as parsed by vial-gui's kle_serial.py).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyGeom {
    pub row: u8,
    pub col: u8,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Degrees, clockwise, about (rx, ry).
    pub angle: f32,
    pub rx: f32,
    pub ry: f32,
}

impl KeyGeom {
    /// Corners after rotation, clockwise from top-left, in key units.
    pub fn corners(&self) -> [(f32, f32); 4] {
        let (s, c) = self.angle.to_radians().sin_cos();
        let rot = |px: f32, py: f32| {
            let (dx, dy) = (px - self.rx, py - self.ry);
            (self.rx + dx * c - dy * s, self.ry + dx * s + dy * c)
        };
        [
            rot(self.x, self.y),
            rot(self.x + self.w, self.y),
            rot(self.x + self.w, self.y + self.h),
            rot(self.x, self.y + self.h),
        ]
    }

    pub fn center(&self) -> (f32, f32) {
        let c = self.corners();
        ((c[0].0 + c[2].0) / 2.0, (c[0].1 + c[2].1) / 2.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub name: String,
    pub rows: u8,
    pub cols: u8,
    pub keys: Vec<KeyGeom>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("keyboard definition is missing {0}")]
    Missing(&'static str),
    #[error("bad layout entry: {0}")]
    Bad(String),
}

/// KLE label-slot remapping by alignment (`a`), from kle-serial.
const LABEL_MAP: [[i8; 12]; 8] = [
    [0, 6, 2, 8, 9, 11, 3, 5, 1, 4, 7, 10],
    [1, 7, -1, -1, 9, 11, 4, -1, -1, -1, -1, 10],
    [3, -1, 5, -1, 9, 11, -1, -1, 4, -1, -1, 10],
    [4, -1, -1, -1, 9, 11, -1, -1, -1, -1, -1, 10],
    [0, 6, 2, 8, 10, -1, 3, 5, 1, 4, 7, -1],
    [1, 7, -1, -1, 10, -1, 4, -1, -1, -1, -1, -1],
    [3, -1, 5, -1, 10, -1, -1, -1, 4, -1, -1, -1],
    [4, -1, -1, -1, 10, -1, -1, -1, -1, -1, -1, -1],
];

fn reorder_labels(text: &str, align: usize) -> [Option<String>; 12] {
    let mut out: [Option<String>; 12] = Default::default();
    for (i, label) in text.split('\n').enumerate().take(12) {
        let slot = LABEL_MAP[align.min(7)][i];
        if slot >= 0 && !label.is_empty() {
            out[slot as usize] = Some(label.to_owned());
        }
    }
    out
}

fn parse_row_col(s: &str) -> Option<(u8, u8)> {
    let (r, c) = s.split_once(',')?;
    Some((r.trim().parse().ok()?, c.trim().parse().ok()?))
}

impl Layout {
    pub fn from_definition(def: &Value) -> Result<Layout, LayoutError> {
        let dim = |ptr: &'static str| -> Result<u8, LayoutError> {
            def.pointer(ptr)
                .and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok())
                .ok_or(LayoutError::Missing(ptr))
        };
        let rows = dim("/matrix/rows")?;
        let cols = dim("/matrix/cols")?;
        let kle = def
            .pointer("/layouts/keymap")
            .and_then(Value::as_array)
            .ok_or(LayoutError::Missing("/layouts/keymap"))?;
        let name = def.get("name").and_then(Value::as_str).unwrap_or("keyboard").to_owned();

        let mut keys = Vec::new();
        let (mut x, mut y, mut w, mut h) = (0.0f32, 0.0f32, 1.0f32, 1.0f32);
        let (mut angle, mut rx, mut ry) = (0.0f32, 0.0f32, 0.0f32);
        let (mut align, mut decal) = (4usize, false);

        for row in kle {
            let Some(items) = row.as_array() else { continue }; // metadata object
            for item in items {
                match item {
                    Value::Object(props) => {
                        let num = |k: &str| props.get(k).and_then(Value::as_f64).map(|v| v as f32);
                        if let Some(v) = num("r") {
                            angle = v;
                        }
                        if let Some(v) = num("rx") {
                            rx = v;
                            (x, y) = (rx, ry);
                        }
                        if let Some(v) = num("ry") {
                            ry = v;
                            (x, y) = (rx, ry);
                        }
                        if let Some(v) = props.get("a").and_then(Value::as_u64) {
                            align = v as usize;
                        }
                        if let Some(v) = num("x") {
                            x += v;
                        }
                        if let Some(v) = num("y") {
                            y += v;
                        }
                        if let Some(v) = num("w") {
                            w = v;
                        }
                        if let Some(v) = num("h") {
                            h = v;
                        }
                        if let Some(v) = props.get("d").and_then(Value::as_bool) {
                            decal = v;
                        }
                    }
                    Value::String(text) => {
                        let labels = reorder_labels(text, align);
                        let encoder = labels[4].as_deref() == Some("e");
                        let default_option = labels[8].as_deref().is_none_or(|opt| opt.ends_with(",0"));
                        let rc = labels[0].as_deref().and_then(parse_row_col);
                        if let (false, false, true, Some((r, c))) = (encoder, decal, default_option, rc) {
                            if r >= rows || c >= cols {
                                return Err(LayoutError::Bad(format!("key {r},{c} is outside the {rows}x{cols} matrix")));
                            }
                            keys.push(KeyGeom { row: r, col: c, x, y, w, h, angle, rx, ry });
                        }
                        x += w;
                        (w, h, decal) = (1.0, 1.0, false);
                    }
                    other => return Err(LayoutError::Bad(other.to_string())),
                }
            }
            y += 1.0;
            x = rx;
        }
        if keys.is_empty() {
            return Err(LayoutError::Missing("keys with row,col labels"));
        }
        Ok(Layout { name, rows, cols, keys })
    }

    /// (min_x, min_y, max_x, max_y) over all rotated key corners, in key units.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for key in &self.keys {
            for (px, py) in key.corners() {
                b = (b.0.min(px), b.1.min(py), b.2.max(px), b.3.max(py));
            }
        }
        b
    }

    pub fn key(&self, row: u8, col: u8) -> Option<&KeyGeom> {
        self.keys.iter().find(|k| k.row == row && k.col == col)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib layout`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add src/layout.rs src/lib.rs
git commit -m "feat: parse Vial KLE layout into key geometry"
```

---

### Task 6: Keymap grid and layer tracker

**Files:**
- Create: `src/keymap.rs`, `src/layers.rs`
- Modify: `src/lib.rs` (add `pub mod keymap;` and `pub mod layers;`)

**Interfaces:**
- Consumes: `keycodes::{decode, Action, tap_basic, KC_NO, KC_TRNS}` (Task 4).
- Produces:
  - `keymap::Keymap` (`Debug, Clone, PartialEq, Eq`):
    - `buffer_len(layers, rows, cols) -> usize`
    - `from_buffer(layers, rows, cols, &[u8]) -> Result<Keymap, KeymapError>` (big-endian u16 per key, ordered layer → row → col)
    - `layers()`, `rows()`, `cols()`
    - `get(layer, row, col) -> u16` (`KC_NO` out of range)
    - `resolve(mask: u32, row, col) -> (u8 layer, u16 code)`: QMK transparency
    - `find_position(mask: u32, usages: &[u8]) -> Option<KeyHit>`: search `mask`, then layer 0, then each layer
  - `keymap::KeyHit { row: u8, col: u8, layer: u8, code: u16 }` (`Debug, Clone, Copy, PartialEq, Eq`).
  - `layers::TriLayer { lower, upper, adjust }` (`Default` = 1, 2, 3) and `layers::TAPPING_TERM: Duration` (200 ms).
  - `layers::LayerTracker`:
    - `new(tri: TriLayer, always_tri: bool)`, `reset()`
    - `press(row, col, code: u16, now: Instant)`, where `code` is resolved at press time
    - `release(row, col, now)`
    - `mask(now) -> u32`, `active_layer(now) -> u8`

- [ ] **Step 1: Write failing tests at the bottom of `src/keymap.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // 2 layers, 2 rows, 2 cols.
    // layer 0: A    B   / LSFT MO(1)
    // layer 1: F1  TRNS / TRNS TRNS
    const CODES: [u16; 8] = [0x0004, 0x0005, 0x00E1, 0x5221, 0x003A, 0x0001, 0x0001, 0x0001];

    fn keymap() -> Keymap {
        let buf: Vec<u8> = CODES.iter().flat_map(|c| c.to_be_bytes()).collect();
        Keymap::from_buffer(2, 2, 2, &buf).unwrap()
    }

    #[test]
    fn decodes_big_endian_layer_row_col() {
        let km = keymap();
        assert_eq!(km.get(0, 0, 1), 0x0005);
        assert_eq!(km.get(0, 1, 1), 0x5221);
        assert_eq!(km.get(1, 0, 0), 0x003A);
        assert_eq!(km.get(5, 0, 0), KC_NO);
        assert!(Keymap::from_buffer(2, 2, 2, &[0; 7]).is_err());
    }

    #[test]
    fn resolves_through_transparent_keys() {
        let km = keymap();
        assert_eq!(km.resolve(0b01, 0, 0), (0, 0x0004));
        assert_eq!(km.resolve(0b11, 0, 0), (1, 0x003A));
        assert_eq!(km.resolve(0b11, 0, 1), (0, 0x0005)); // TRNS on layer 1 falls to layer 0
    }

    #[test]
    fn finds_positions_on_active_layer_then_layer_zero_then_others() {
        let km = keymap();
        assert_eq!(km.find_position(0b01, &[0x05]), Some(KeyHit { row: 0, col: 1, layer: 0, code: 0x0005 }));
        assert_eq!(km.find_position(0b01, &[0x3A]), Some(KeyHit { row: 0, col: 0, layer: 1, code: 0x003A }));
        assert_eq!(km.find_position(0b11, &[0x04]), Some(KeyHit { row: 0, col: 0, layer: 0, code: 0x0004 }));
        assert_eq!(km.find_position(0b01, &[0x28]), None);
    }
}
```

- [ ] **Step 2: Write failing tests at the bottom of `src/layers.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const KC_A: u16 = 0x0004;
    const MO1: u16 = 0x5221;
    const MO2: u16 = 0x5222;
    const LT2_SPC: u16 = 0x422C;
    const TG1: u16 = 0x5261;
    const TO3: u16 = 0x5203;
    const DF1: u16 = 0x5241;
    const OSL1: u16 = 0x5281;
    const TL_LOWR: u16 = 0x7C77;
    const TL_UPPR: u16 = 0x7C78;

    fn tracker(always_tri: bool) -> (LayerTracker, Instant) {
        (LayerTracker::new(TriLayer::default(), always_tri), Instant::now())
    }

    #[test]
    fn momentary_layer_while_held() {
        let (mut t, now) = tracker(false);
        t.press(4, 1, MO1, now);
        assert_eq!(t.active_layer(now), 1);
        t.release(4, 1, now);
        assert_eq!(t.active_layer(now), 0);
    }

    #[test]
    fn layer_tap_activates_after_tapping_term_or_interrupt() {
        let (mut t, now) = tracker(false);
        t.press(4, 3, LT2_SPC, now);
        assert_eq!(t.active_layer(now + Duration::from_millis(50)), 0);
        assert_eq!(t.active_layer(now + Duration::from_millis(250)), 2);
        t.release(4, 3, now);

        t.press(4, 3, LT2_SPC, now);
        t.press(0, 0, KC_A, now + Duration::from_millis(20));
        assert_eq!(t.active_layer(now + Duration::from_millis(30)), 2);
    }

    #[test]
    fn toggle_to_and_default() {
        let (mut t, now) = tracker(false);
        t.press(0, 0, TG1, now);
        t.release(0, 0, now);
        assert_eq!(t.active_layer(now), 1);
        t.press(0, 0, TG1, now);
        t.release(0, 0, now);
        assert_eq!(t.active_layer(now), 0);
        t.press(0, 1, TO3, now);
        assert_eq!(t.active_layer(now), 3);
        t.reset();
        t.press(0, 2, DF1, now);
        assert_eq!(t.active_layer(now), 1);
    }

    #[test]
    fn one_shot_layer_applies_to_next_key_only() {
        let (mut t, now) = tracker(false);
        t.press(0, 0, OSL1, now);
        t.release(0, 0, now);
        assert_eq!(t.active_layer(now), 1);
        t.press(1, 1, KC_A, now);
        assert_eq!(t.active_layer(now), 1);
        t.release(1, 1, now);
        assert_eq!(t.active_layer(now), 0);
    }

    #[test]
    fn tri_layer_emulation() {
        let (mut t, now) = tracker(true);
        t.press(4, 1, MO1, now);
        t.press(9, 1, MO2, now);
        assert_eq!(t.active_layer(now), 3);

        let (mut t, now) = tracker(false);
        t.press(4, 1, MO1, now);
        t.press(9, 1, MO2, now);
        assert_eq!(t.active_layer(now), 2);

        let (mut t, now) = tracker(false);
        t.press(4, 1, TL_LOWR, now);
        assert_eq!(t.active_layer(now), 1);
        t.press(9, 1, TL_UPPR, now);
        assert_eq!(t.active_layer(now), 3);
    }
}
```

- [ ] **Step 3: Add the modules to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib keymap layers`
Expected: compile errors (items not found).

- [ ] **Step 4: Implement `src/keymap.rs` (above the tests)**

```rust
//! The keyboard's keymap as read with `VIA_GET_BUFFER`.

use crate::keycodes::{self, KC_NO, KC_TRNS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    layers: u8,
    rows: u8,
    cols: u8,
    codes: Vec<u16>,
}

#[derive(Debug, thiserror::Error)]
#[error("keymap buffer is {got} bytes but {layers}x{rows}x{cols} needs {want}")]
pub struct KeymapError {
    got: usize,
    want: usize,
    layers: u8,
    rows: u8,
    cols: u8,
}

/// Where an OS-reported key most likely is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHit {
    pub row: u8,
    pub col: u8,
    pub layer: u8,
    pub code: u16,
}

impl Keymap {
    pub fn buffer_len(layers: u8, rows: u8, cols: u8) -> usize {
        layers as usize * rows as usize * cols as usize * 2
    }

    pub fn from_buffer(layers: u8, rows: u8, cols: u8, buf: &[u8]) -> Result<Keymap, KeymapError> {
        let want = Self::buffer_len(layers, rows, cols);
        if buf.len() != want {
            return Err(KeymapError { got: buf.len(), want, layers, rows, cols });
        }
        let codes = buf.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
        Ok(Keymap { layers, rows, cols, codes })
    }

    pub fn layers(&self) -> u8 {
        self.layers
    }

    pub fn rows(&self) -> u8 {
        self.rows
    }

    pub fn cols(&self) -> u8 {
        self.cols
    }

    pub fn get(&self, layer: u8, row: u8, col: u8) -> u16 {
        if layer >= self.layers || row >= self.rows || col >= self.cols {
            return KC_NO;
        }
        self.codes[(layer as usize * self.rows as usize + row as usize) * self.cols as usize + col as usize]
    }

    /// QMK lookup: the highest layer set in `mask` whose code is not `KC_TRNS`.
    pub fn resolve(&self, mask: u32, row: u8, col: u8) -> (u8, u16) {
        for layer in (0..self.layers.min(32)).rev() {
            if mask & (1u32 << layer) != 0 {
                let code = self.get(layer, row, col);
                if code != KC_TRNS {
                    return (layer, code);
                }
            }
        }
        (0, self.get(0, row, col))
    }

    /// First key (matrix order) whose resolved code sends one of `usages`, searching the
    /// layers in `mask`, then layer 0, then each other layer on its own.
    pub fn find_position(&self, mask: u32, usages: &[u8]) -> Option<KeyHit> {
        let masks = std::iter::once(mask).chain((0..self.layers.min(32)).map(|l| 1u32 << l));
        for m in masks {
            for row in 0..self.rows {
                for col in 0..self.cols {
                    let (layer, code) = self.resolve(m, row, col);
                    if keycodes::tap_basic(code).is_some_and(|b| usages.contains(&b)) {
                        return Some(KeyHit { row, col, layer, code });
                    }
                }
            }
        }
        None
    }
}
```

- [ ] **Step 5: Implement `src/layers.rs` (above the tests)**

```rust
//! Approximates QMK's layer state from physical key presses (matrix tier only).

use std::time::{Duration, Instant};

use crate::keycodes::{Action, decode};

/// Layers used by QMK's tri-layer feature: lower + upper held → adjust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriLayer {
    pub lower: u8,
    pub upper: u8,
    pub adjust: u8,
}

impl Default for TriLayer {
    fn default() -> Self {
        Self { lower: 1, upper: 2, adjust: 3 }
    }
}

/// QMK's default TAPPING_TERM; the firmware's real value is not readable over Vial.
pub const TAPPING_TERM: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
struct Held {
    row: u8,
    col: u8,
    action: Action,
    since: Instant,
    interrupted: bool,
}

#[derive(Debug, Clone)]
pub struct LayerTracker {
    default_layer: u8,
    toggled: u32,
    held: Vec<Held>,
    oneshot: Option<u8>,
    oneshot_consumer: Option<(u8, u8)>,
    tri: TriLayer,
    /// Emulate firmware-side `update_tri_layer_state` (the stock Lily58 Vial keymap does this).
    always_tri: bool,
}

impl LayerTracker {
    pub fn new(tri: TriLayer, always_tri: bool) -> Self {
        Self { default_layer: 0, toggled: 0, held: Vec::new(), oneshot: None, oneshot_consumer: None, tri, always_tri }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.tri, self.always_tri);
    }

    /// `code` is the keycode at (row, col) resolved with the layer state *before* this press.
    pub fn press(&mut self, row: u8, col: u8, code: u16, now: Instant) {
        let action = decode(code);
        for h in &mut self.held {
            h.interrupted = true;
        }
        let holds_layer = matches!(
            action,
            Action::Momentary(_)
                | Action::LayerTap { .. }
                | Action::LayerMod { .. }
                | Action::TapToggle(_)
                | Action::OneShotLayer(_)
                | Action::TriLayerLower
                | Action::TriLayerUpper
        );
        match action {
            Action::Toggle(l) => self.toggled ^= bit(l),
            Action::To(l) => {
                self.toggled = bit(l);
                self.held.clear();
            }
            Action::DefaultLayer(l) | Action::PersistentDefault(l) => self.default_layer = l,
            _ if holds_layer => self.held.push(Held { row, col, action, since: now, interrupted: false }),
            _ => {
                if self.oneshot.is_some() && self.oneshot_consumer.is_none() {
                    self.oneshot_consumer = Some((row, col));
                }
            }
        }
    }

    pub fn release(&mut self, row: u8, col: u8, _now: Instant) {
        if let Some(i) = self.held.iter().position(|h| h.row == row && h.col == col) {
            let h = self.held.remove(i);
            if let (Action::OneShotLayer(l), false) = (h.action, h.interrupted) {
                self.oneshot = Some(l);
                self.oneshot_consumer = None;
            }
        }
        if self.oneshot_consumer == Some((row, col)) {
            self.oneshot = None;
            self.oneshot_consumer = None;
        }
    }

    pub fn mask(&self, now: Instant) -> u32 {
        let mut mask = bit(self.default_layer) | self.toggled;
        let mut tri_key_held = false;
        for h in &self.held {
            let long = h.interrupted || now.duration_since(h.since) >= TAPPING_TERM;
            match h.action {
                Action::Momentary(l) | Action::LayerMod { layer: l, .. } | Action::OneShotLayer(l) => mask |= bit(l),
                Action::LayerTap { layer: l, .. } | Action::TapToggle(l) if long => mask |= bit(l),
                Action::TriLayerLower => {
                    mask |= bit(self.tri.lower);
                    tri_key_held = true;
                }
                Action::TriLayerUpper => {
                    mask |= bit(self.tri.upper);
                    tri_key_held = true;
                }
                _ => {}
            }
        }
        if let Some(l) = self.oneshot {
            mask |= bit(l);
        }
        let both = mask & bit(self.tri.lower) != 0 && mask & bit(self.tri.upper) != 0;
        if (self.always_tri || tri_key_held) && both {
            mask |= bit(self.tri.adjust);
        }
        mask
    }

    pub fn active_layer(&self, now: Instant) -> u8 {
        31 - self.mask(now).leading_zeros() as u8
    }
}

fn bit(layer: u8) -> u32 {
    1u32.checked_shl(layer as u32).unwrap_or(0)
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib keymap layers`
Expected: 3 + 5 passed.

- [ ] **Step 7: Commit**

```bash
git add src/keymap.rs src/layers.rs src/lib.rs
git commit -m "feat: keymap transparency resolution and QMK layer tracking"
```

---

### Task 7: Vial client and firmware simulator

**Files:**
- Create: `src/vial.rs`
- Modify: `src/hid/fake.rs` (add `SimState`, `KeyboardSim`, `SMALL_DEFINITION`, `SMALL_KEYMAP`), `src/lib.rs` (add `pub mod vial;`)

**Interfaces:**
- Consumes: `ReadOnlyGuard`, `GuardError`, `Transport`, `protocol::*`, `FakeTransport` (Task 2).
- Produces:
  - `vial::VialClient<T: Transport>`, constructed with `new(ReadOnlyGuard<T>)` (500 ms timeout and retry delay) or `with_timing(guard, timeout, retry_delay)`. Queries:
    - `via_protocol_version() -> Result<u16>`
    - `keyboard_id() -> Result<KeyboardId>`
    - `definition() -> Result<serde_json::Value>`
    - `layer_count() -> Result<u8>`
    - `keymap_buffer(len: usize) -> Result<Vec<u8>>`
    - `unlock_status() -> Result<UnlockStatus>`
    - `unlock_start() -> Result<()>`
    - `unlock_poll() -> Result<UnlockPoll>`
    - `matrix_state(rows, cols) -> Result<MatrixState>`
  - `vial::check_protocol(&KeyboardId) -> Result<(), VialError>`.
  - `vial::KeyboardId { vial_protocol: u32, uid: u64 }`.
  - `vial::UnlockStatus { unlocked: bool, in_progress: bool, keys: Vec<(u8, u8)> }`.
  - `vial::UnlockPoll { unlocked, in_progress, counter: u8 }`.
  - `vial::MatrixState`: `empty(rows, cols)`, `is_pressed(row, col)`, `pressed() -> Vec<(u8, u8)>`; derives `Debug, Clone, PartialEq, Eq`.
  - `vial::VialError` (thiserror): `Guard`, `Io`, `Timeout(u8)`, `BadReply(String)`, `ProtocolTooOld { found }`, plus `is_disconnect() -> bool`.
  - Test-only `hid::fake::KeyboardSim`: `new(definition_json, layers, rows, cols, codes: &[u16], unlock_keys: &[(u8, u8)])`, `small()`, `with(|&mut SimState| ..)`, `transport() -> FakeTransport`, `unplug()`, `is_unplugged()`. `SimState` has public fields `definition_xz, layers, rows, cols, keymap, vial_protocol, unlocked, unlock_in_progress, unlock_counter, unlock_keys, matrix: Vec<u32>, requests: usize`.

- [ ] **Step 1: Add the firmware simulator to `src/hid/fake.rs`**

Add `use crate::protocol::*;` next to the existing imports (and remove the now-redundant `use crate::protocol::Report;`), then append:

```rust
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
    /// One bitmask per row; bit n = column n pressed.
    pub matrix: Vec<u32>,
    /// Number of reports received.
    pub requests: usize,
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
            matrix: vec![0; rows as usize],
            requests: 0,
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
        }
        (VIAL_PREFIX, VIAL_UNLOCK_POLL) => {
            if s.unlock_in_progress {
                let holding = s.unlock_keys.iter().all(|&(row, col)| (s.matrix[row as usize] >> col) & 1 == 1);
                if holding {
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
```

- [ ] **Step 2: Write failing tests at the bottom of `src/vial.rs`**

```rust
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
        let mut last = c.unlock_poll().unwrap();
        assert_eq!(last.counter, UNLOCK_COUNTER_MAX - 1);
        for _ in 0..60 {
            if last.unlocked {
                break;
            }
            last = c.unlock_poll().unwrap();
        }
        assert!(last.unlocked && !last.in_progress);
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
```

- [ ] **Step 3: Add `pub mod vial;` to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib vial`
Expected: compile errors (items not found).

- [ ] **Step 4: Implement `src/vial.rs` (above the tests)**

```rust
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
            .chunks_exact(2)
            .map(|p| (p[0], p[1]))
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib vial`
Expected: 12 passed.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/vial.rs src/hid/fake.rs src/lib.rs
git commit -m "feat: Vial client with firmware simulator tests"
```

---

### Task 8: Host layout characters and HID/evdev key maps

**Files:**
- Create: `src/hostlayout.rs`, `src/hidmap.rs`
- Modify: `src/lib.rs` (add `pub mod hidmap;` and `pub mod hostlayout;`)

**Interfaces:**
- Produces:
  - `hostlayout::HostLayout { Gb (default), Us }`: derives `Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize` with lowercase names; `char_for(self, usage: u8, shift: bool) -> Option<char>`.
  - `hidmap::hid_to_evdev(u8) -> Option<u16>`.
  - `hidmap::evdev_to_hid(u16) -> Vec<u8>`: only keyboard-page usages `0x04..=0xA4` and `0xE0..=0xE7`.
  - `hidmap::egui_key_to_hid(egui::Key) -> Vec<u8>`.

- [ ] **Step 1: Write failing tests**

Bottom of `src/hostlayout.rs`:

```rust
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
```

Bottom of `src/hidmap.rs`:

```rust
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
```

- [ ] **Step 2: Add the modules to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib hostlayout hidmap`
Expected: compile errors.

- [ ] **Step 3: Implement `src/hostlayout.rs` (above the tests)**

```rust
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
```

- [ ] **Step 4: Implement `src/hidmap.rs` (above the tests)**

```rust
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
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib hostlayout hidmap`
Expected: 3 + 3 passed.

- [ ] **Step 6: Commit**

```bash
git add src/hostlayout.rs src/hidmap.rs src/lib.rs
git commit -m "feat: host-layout characters and HID/evdev/egui key maps"
```

---

### Task 9: Input sources (evdev and focused window)

**Files:**
- Create: `src/input/mod.rs`, `src/input/evdev.rs`, `src/input/focused.rs`
- Modify: `src/lib.rs` (add `pub mod input;`)

**Interfaces:**
- Consumes: `hid::discover::usb_device_dir` (Task 3), `hidmap::{evdev_to_hid, egui_key_to_hid}` (Task 8).
- Produces:
  - `input::OsSource { Focused, Evdev }`.
  - `input::OsKey { source: OsSource, usages: Vec<u8>, pressed: bool, name: String }` (`Debug, Clone, PartialEq, Eq`); `name` uniquely identifies the physical OS key.
  - `input::InputMsg { Key(OsKey), EvdevGone(PathBuf) }`.
  - `input::evdev::EvdevStatus { Active, NoAccess(Vec<PathBuf>), NotFound }` (`Debug, Clone, PartialEq, Eq`).
  - `input::evdev::find_event_nodes(sys_root, dev_root, usb_dir) -> Vec<PathBuf>`.
  - `input::evdev::start(&[PathBuf], Sender<InputMsg>, notify: impl Fn() + Send + Clone + 'static) -> EvdevStatus`.
  - `input::evdev::key_msg(code: u16, pressed: bool) -> OsKey`.
  - `input::focused::{FocusedInput { Key(OsKey), Text(String) }, translate(&[egui::Event]) -> Vec<FocusedInput>}`.

- [ ] **Step 1: Write `src/input/mod.rs` and register the module**

```rust
//! Key events as the operating system reports them (no layer keys; the position is inferred).

pub mod evdev;
pub mod focused;

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsSource {
    Focused,
    Evdev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsKey {
    pub source: OsSource,
    /// HID usages that could have produced this key (several when ambiguous).
    pub usages: Vec<u8>,
    pub pressed: bool,
    /// Identifies the physical OS key, e.g. "evdev 30" or "A".
    pub name: String,
}

pub enum InputMsg {
    Key(OsKey),
    /// An evdev reader stopped (device unplugged).
    EvdevGone(PathBuf),
}
```

Add `pub mod input;` to `src/lib.rs`.

- [ ] **Step 2: Write failing tests**

Bottom of `src/input/evdev.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn finds_only_event_nodes_of_the_same_usb_device() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let ours = root.join("devices/usb1/1-3");
        let other = root.join("devices/usb1/1-4");
        for usb in [&ours, &other] {
            fs::create_dir_all(usb).unwrap();
            fs::write(usb.join("idVendor"), "7171\n").unwrap();
        }
        let links = [
            ("event5", ours.join("1-3:1.0/0003:7171:0012.0001/input/input7")),
            ("event6", other.join("1-4:1.0/0003:1234:5678.0003/input/input9")),
            ("mouse0", ours.join("1-3:1.2/0003:7171:0012.0003/input/input8")),
        ];
        for (name, input_dir) in &links {
            fs::create_dir_all(input_dir).unwrap();
            let class_dir = root.join("class/input").join(name);
            fs::create_dir_all(&class_dir).unwrap();
            symlink(input_dir, class_dir.join("device")).unwrap();
        }
        let usb_dir = fs::canonicalize(&ours).unwrap();
        assert_eq!(find_event_nodes(root, Path::new("/dev"), &usb_dir), vec![PathBuf::from("/dev/input/event5")]);
    }

    #[test]
    fn no_nodes_means_not_found() {
        let (tx, _rx) = std::sync::mpsc::channel();
        assert_eq!(start(&[], tx, || {}), EvdevStatus::NotFound);
    }

    #[test]
    fn key_messages_carry_hid_candidates() {
        let k = key_msg(43, true);
        assert_eq!(k.usages, vec![0x31, 0x32]);
        assert!(k.pressed);
        assert_eq!(k.source, OsSource::Evdev);
        assert_eq!(k.name, "evdev 43");
    }
}
```

Bottom of `src/input/focused.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Modifiers;

    fn key(k: Key, pressed: bool, repeat: bool) -> Event {
        Event::Key { key: k, physical_key: Some(k), pressed, repeat, modifiers: Modifiers::default() }
    }

    #[test]
    fn translates_keys_and_text_and_drops_repeats() {
        let out = translate(&[key(Key::A, true, false), key(Key::A, true, true), Event::Text("a".into())]);
        assert_eq!(out.len(), 2);
        match &out[0] {
            FocusedInput::Key(k) => {
                assert_eq!(k.usages, vec![0x04]);
                assert!(k.pressed);
                assert_eq!(k.name, "A");
            }
            FocusedInput::Text(_) => panic!("expected a key"),
        }
        assert!(matches!(&out[1], FocusedInput::Text(t) if t == "a"));
    }
}
```

Run: `cargo test --lib input`
Expected: compile errors.

- [ ] **Step 3: Implement `src/input/evdev.rs` (above the tests)**

```rust
//! Unfocused tier: reads the Lily58's own `/dev/input/eventN` nodes (needs the optional udev rule).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use ::evdev::{Device, EventSummary};

use super::{InputMsg, OsKey, OsSource};
use crate::hid::discover::usb_device_dir;
use crate::hidmap::evdev_to_hid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvdevStatus {
    Active,
    /// Nodes exist but none could be opened (no udev rule yet).
    NoAccess(Vec<PathBuf>),
    NotFound,
}

/// `/dev/input/eventN` nodes that belong to the USB device at `usb_dir` (canonical sysfs path).
pub fn find_event_nodes(sys_root: &Path, dev_root: &Path, usb_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sys_root.join("class/input")) else { return Vec::new() };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
        .filter(|e| {
            fs::canonicalize(e.path().join("device")).ok().and_then(|p| usb_device_dir(&p)).as_deref() == Some(usb_dir)
        })
        .map(|e| dev_root.join("input").join(e.file_name()))
        .collect();
    nodes.sort();
    nodes
}

/// Opens every node and spawns one reader thread per opened node.
pub fn start(nodes: &[PathBuf], tx: Sender<InputMsg>, notify: impl Fn() + Send + Clone + 'static) -> EvdevStatus {
    if nodes.is_empty() {
        return EvdevStatus::NotFound;
    }
    let (mut opened, mut denied) = (0, Vec::new());
    for node in nodes {
        match Device::open(node) {
            Ok(dev) => {
                opened += 1;
                spawn_reader(node.clone(), dev, tx.clone(), notify.clone());
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied.push(node.clone()),
            Err(e) => log::warn!("cannot open {}: {e}", node.display()),
        }
    }
    if opened > 0 {
        EvdevStatus::Active
    } else if !denied.is_empty() {
        EvdevStatus::NoAccess(denied)
    } else {
        EvdevStatus::NotFound
    }
}

pub fn key_msg(code: u16, pressed: bool) -> OsKey {
    OsKey { source: OsSource::Evdev, usages: evdev_to_hid(code), pressed, name: format!("evdev {code}") }
}

fn spawn_reader(node: PathBuf, mut dev: Device, tx: Sender<InputMsg>, notify: impl Fn() + Send + 'static) {
    let name = format!("evdev {}", node.display());
    let spawned = thread::Builder::new().name(name).spawn(move || {
        loop {
            let events = match dev.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    log::info!("{} closed: {e}", node.display());
                    let _ = tx.send(InputMsg::EvdevGone(node));
                    notify();
                    return;
                }
            };
            for ev in events {
                if let EventSummary::Key(_, code, value) = ev.destructure() {
                    if value == 2 {
                        continue; // auto-repeat
                    }
                    if tx.send(InputMsg::Key(key_msg(code.code(), value == 1))).is_err() {
                        return; // UI gone
                    }
                    notify();
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::error!("cannot start evdev reader: {e}");
    }
}
```

- [ ] **Step 4: Implement `src/input/focused.rs` (above the tests)**

```rust
//! Focused tier: key and text events egui delivers while our window has focus.

use eframe::egui::{Event, Key};

use super::{OsKey, OsSource};
use crate::hidmap::egui_key_to_hid;

pub enum FocusedInput {
    Key(OsKey),
    Text(String),
}

pub fn translate(events: &[Event]) -> Vec<FocusedInput> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Key { key, physical_key, pressed, repeat: false, .. } => {
                let k: Key = physical_key.unwrap_or(*key);
                Some(FocusedInput::Key(OsKey {
                    source: OsSource::Focused,
                    usages: egui_key_to_hid(k),
                    pressed: *pressed,
                    name: format!("{k:?}"),
                }))
            }
            Event::Text(t) => Some(FocusedInput::Text(t.clone())),
            _ => None,
        })
        .collect()
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib input`
Expected: 4 passed.

- [ ] **Step 6: Commit**

```bash
git add src/input src/lib.rs
git commit -m "feat: evdev and focused-window input sources"
```

---

### Task 10: `--probe` diagnostics and the real-keyboard fixture

**Files:**
- Create: `src/probe.rs`, `tests/fixtures/lily58-definition.json` (captured, not hand-written)
- Modify: `src/main.rs`, `src/lib.rs` (add `pub mod probe;`), `src/layout.rs` (fixture test)

**Interfaces:**
- Consumes:
  - `discover::{find_vial_device, other_holders, VIAL_SERIAL_MARKER}`
  - `hidraw::open_guarded`
  - `vial::{VialClient, check_protocol}`
  - `Layout::from_definition`
  - `Keymap::{buffer_len, from_buffer, get}`
  - `keycodes::label`
  - `input::evdev::find_event_nodes`
- Produces: `probe::run(out: &mut impl Write, dump_definition: Option<&Path>) -> anyhow::Result<()>`; CLI `lily58-assistant --probe [--dump-definition FILE]`.

This is the first task that talks to the real keyboard. It sends only guarded read commands.

- [ ] **Step 1: Write `src/probe.rs` and add `pub mod probe;` to `src/lib.rs`**

```rust
//! `--probe`: prints what the app can see, using only guarded read commands.

use std::io::{self, Write};
use std::path::Path;

use crate::hid::{discover, hidraw};
use crate::input::evdev::find_event_nodes;
use crate::keycodes;
use crate::keymap::Keymap;
use crate::layout::Layout;
use crate::vial::{self, VialClient};

pub fn run(out: &mut impl Write, dump_definition: Option<&Path>) -> anyhow::Result<()> {
    let (sys, dev_root) = (Path::new("/sys"), Path::new("/dev"));
    let Some(dev) = discover::find_vial_device(sys, dev_root)? else {
        writeln!(out, "No Vial keyboard found (looked for a USB serial containing {}).", discover::VIAL_SERIAL_MARKER)?;
        return Ok(());
    };
    writeln!(out, "Device:      {} ({:04x}:{:04x}) at {}", dev.product, dev.vendor_id, dev.product_id, dev.dev_node.display())?;
    let holders = discover::other_holders(Path::new("/proc"), &dev.dev_node, std::process::id());
    if !holders.is_empty() {
        writeln!(out, "WARNING:     also open in {}; close it for reliable results", holders.join(", "))?;
    }
    let guard = match hidraw::open_guarded(&dev.dev_node) {
        Ok(guard) => guard,
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            writeln!(out, "Raw HID:     NO ACCESS ({e}); see README, \"Permissions\"")?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    writeln!(out, "Raw HID:     accessible")?;

    let mut client = VialClient::new(guard);
    let id = client.keyboard_id()?;
    writeln!(out, "Vial:        protocol {} (keyboard uid {:016x})", id.vial_protocol, id.uid)?;
    vial::check_protocol(&id)?;
    let status = client.unlock_status()?;
    let lock = match (status.unlocked, status.in_progress) {
        (true, _) => "unlocked (live layer tracking available)",
        (false, true) => "UNLOCK IN PROGRESS: VIA reads are blocked until it completes or the keyboard is replugged",
        (false, false) => "locked",
    };
    writeln!(out, "Unlock:      {lock}; unlock keys {:?}", status.keys)?;
    if status.in_progress && !status.unlocked {
        return Ok(());
    }
    writeln!(out, "VIA:         protocol {:#06x}", client.via_protocol_version()?)?;

    let definition = client.definition()?;
    if let Some(path) = dump_definition {
        std::fs::write(path, serde_json::to_string_pretty(&definition)? + "\n")?;
        writeln!(out, "Definition:  written to {}", path.display())?;
    }
    let layout = Layout::from_definition(&definition)?;
    writeln!(out, "Layout:      {}: {} keys, matrix {}x{}", layout.name, layout.keys.len(), layout.rows, layout.cols)?;

    let layers = client.layer_count()?;
    let buf = client.keymap_buffer(Keymap::buffer_len(layers, layout.rows, layout.cols))?;
    let keymap = Keymap::from_buffer(layers, layout.rows, layout.cols, &buf)?;
    for layer in 0..layers {
        writeln!(out, "Layer {layer}:")?;
        for row in 0..layout.rows {
            let cells: Vec<String> =
                (0..layout.cols).map(|col| format!("{:>14}", keycodes::label(keymap.get(layer, row, col)))).collect();
            writeln!(out, "  row {row}:{}", cells.concat())?;
        }
    }

    for node in find_event_nodes(sys, dev_root, &dev.usb_dir) {
        let access = match std::fs::File::open(&node) {
            Ok(_) => "readable (all-windows tracking available)".to_string(),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => "no access (optional udev rule not installed)".to_string(),
            Err(e) => format!("error: {e}"),
        };
        writeln!(out, "Input node:  {} {access}", node.display())?;
    }
    Ok(())
}
```

- [ ] **Step 2: Replace `src/main.rs` with argument parsing (the GUI is still the smoke window)**

```rust
use std::path::Path;

use eframe::egui;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => run_gui(),
        ["--probe"] => lily58_assistant::probe::run(&mut std::io::stdout(), None),
        ["--probe", "--dump-definition", file] => {
            lily58_assistant::probe::run(&mut std::io::stdout(), Some(Path::new(file)))
        }
        _ => {
            eprintln!("usage: lily58-assistant [--probe [--dump-definition FILE]]");
            std::process::exit(2);
        }
    }
}

fn run_gui() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lily58 Assistant")
            .with_app_id("lily58-assistant")
            .with_inner_size([960.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native("Lily58 Assistant", options, Box::new(|_cc| Ok(Box::new(Smoke))))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

struct Smoke;

impl eframe::App for Smoke {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Lily58 Assistant: smoke build");
        });
    }
}
```

(Keep the `renderer: eframe::Renderer::Glow` line if Task 1 needed it.)

- [ ] **Step 3: Build**

Run: `cargo build`
Expected: no errors or warnings.

- [ ] **Step 4: Probe the real keyboard**

Close Vial if it's running. The Lily58 must be plugged in.

Run: `cargo run -- --probe`
Expected output shape (values will differ):

```
Device:      Lily58 Pro R2G (7171:0012) at /dev/hidraw15
Raw HID:     accessible
Vial:        protocol 6 (keyboard uid ...)
Unlock:      locked; unlock keys [(4, 4), (9, 4)]
VIA:         protocol 0x000c
Layout:      Lily58: 58 keys, matrix 10x6
Layer 0:
  row 0:        KC_ESC          KC_1 ...
...
Input node:  /dev/input/event259 no access (optional udev rule not installed)
```

If it says `NO ACCESS`, the hidraw udev rule is missing: see README "Permissions" (Task 13 writes it; on the Fedora PC `/etc/udev/rules.d/59-vial.rules` already exists). If it errors with `definition does not decompress`, the container is not xz: capture the raw bytes and switch `xz_decompress` to `lzma_decompress` in `VialClient::definition`, updating the simulator in `fake.rs` to match. **Stop and report** any error before continuing.

- [ ] **Step 5: Capture the definition fixture**

Run: `mkdir -p tests/fixtures && cargo run -- --probe --dump-definition tests/fixtures/lily58-definition.json`
Expected: `Definition:  written to tests/fixtures/lily58-definition.json`. The file is pretty-printed JSON with `"matrix"` and `"layouts"`.

- [ ] **Step 6: Add the fixture test to the `tests` module in `src/layout.rs`**

Use the matrix size and key count the probe printed in Step 4 (expected 10x6 and 58):

```rust
    #[test]
    fn parses_the_captured_lily58_definition() {
        let def: Value = serde_json::from_str(include_str!("../tests/fixtures/lily58-definition.json")).unwrap();
        let layout = Layout::from_definition(&def).unwrap();
        assert_eq!((layout.rows, layout.cols), (10, 6));
        assert_eq!(layout.keys.len(), 58);
        let (min_x, min_y, max_x, max_y) = layout.bounds();
        assert!(max_x - min_x > 10.0 && max_y - min_y > 3.0, "split board should be wide: {:?}", layout.bounds());
    }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test --lib layout`
Expected: 7 passed.

- [ ] **Step 8: Commit**

```bash
git add src/probe.rs src/main.rs src/lib.rs src/layout.rs tests/fixtures/lily58-definition.json
git commit -m "feat: --probe diagnostics; add captured Lily58 definition fixture"
```

---

### Task 11: Device worker

**Files:**
- Create: `src/device.rs`
- Modify: `src/lib.rs` (add `pub mod device;`)

**Interfaces:**
- Consumes: `discover::{VialDevice, find_vial_device, other_holders}`, `hidraw::open_guarded`, `ReadOnlyGuard::boxed`, `VialClient` + `check_protocol` + `MatrixState` + `VialError::is_disconnect`, `Layout`, `Keymap`, test-only `KeyboardSim`.
- Produces:
  - `device::DeviceEvent` (`Debug, Clone, PartialEq`): `Waiting`, `NoAccess(PathBuf)`, `Paused { holders }`, `Resumed`, `Connected { info: DeviceInfo, layout: Layout, keymap: Keymap }`, `Disconnected`, `Locked { unlock_keys }`, `Unlocking { counter, unlock_keys }`, `Unlocked`, `Matrix { pressed, released }`, `Error(String)`.
  - `device::DeviceInfo { dev_node, usb_dir, product, via_protocol: u16, vial_protocol: u32 }`.
  - `device::DeviceCommand { Reload, StartUnlock }`.
  - `device::Connector` trait (`find`, `open`, `other_holders`) and `device::SystemConnector`.
  - `device::Worker<C>`: `new(connector, Sender<DeviceEvent>, Box<dyn Fn() + Send>)`, `step(now) -> Duration`, `handle(cmd)`.
  - `device::spawn(connector, Sender<DeviceEvent>, notify) -> Sender<DeviceCommand>`.

Event-order guarantees the UI relies on:
- `Connected` is always followed immediately by `Locked` or `Unlocked`.
- `Matrix` events appear only after `Unlocked`.
- After an unlock left in progress, `Unlocking` events come *before* the first `Connected`: the keymap can only be read once the unlock completes.

- [ ] **Step 1: Write failing tests at the bottom of `src/device.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::fake::KeyboardSim;
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};

    struct FakeConnector {
        sim: KeyboardSim,
        deny: bool,
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
            let connector = FakeConnector { sim: sim.clone(), deny, holders: Arc::clone(&holders) };
            Self { worker: Worker::new(connector, tx, Box::new(|| {})), rx, sim, holders, t0: Instant::now() }
        }

        /// Runs `n` steps at t0 + `at_ms` and returns the events they produced.
        fn steps(&mut self, n: usize, at_ms: u64) -> Vec<DeviceEvent> {
            for _ in 0..n {
                self.worker.step(self.t0 + Duration::from_millis(at_ms));
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
        let events = h.steps(60, 0);
        let n = names(&events);
        assert_eq!(n.iter().filter(|&&e| e == "Unlocking").count(), 49);
        let unlocked = n.iter().position(|&e| e == "Unlocked").expect("unlocked");
        assert_eq!(events[unlocked + 1], DeviceEvent::Matrix { pressed: vec![(1, 0), (1, 2)], released: vec![] });
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
        let events = h.steps(60, 0);
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
}
```

- [ ] **Step 2: Add `pub mod device;` to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib device`
Expected: compile errors (items not found).

- [ ] **Step 3: Implement `src/device.rs` (above the tests)**

```rust
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
/// The firmware counts down at most once per 100 ms of polling, so 50 ms keeps an unlock at ~5 s.
pub const UNLOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const IDLE_POLL: Duration = Duration::from_millis(100);

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
}

impl<C: Connector> Worker<C> {
    pub fn new(connector: C, events: Sender<DeviceEvent>, notify: Box<dyn Fn() + Send>) -> Self {
        Self { connector, events, notify, conn: Conn::Idle { reported: None } }
    }

    /// Does one unit of work and returns how long to wait before the next.
    pub fn step(&mut self, now: Instant) -> Duration {
        let mut out = Vec::new();
        let wait = if let Conn::Connected(s) = &mut self.conn {
            match poll(s, &mut self.connector, now, &mut out) {
                Ok(wait) => wait,
                Err(e) => {
                    self.drop_session(e, &mut out);
                    SCAN_INTERVAL
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
            Ok(None) => DeviceEvent::Waiting,
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
        out.push(DeviceEvent::Disconnected);
        let reported = if e.is_disconnect() {
            log::info!("keyboard went away: {e}");
            None
        } else {
            log::warn!("keyboard error: {e}");
            let event = DeviceEvent::Error(e.to_string());
            out.push(event.clone());
            Some(event)
        };
        self.conn = Conn::Idle { reported };
    }
}

fn start_session(dev: VialDevice, guard: ReadOnlyGuard<Box<dyn Transport>>, now: Instant) -> Result<Session, VialError> {
    let mut client = VialClient::new(guard);
    // Only Vial commands here: they are answered even while an unlock is in progress.
    let id = client.keyboard_id()?;
    vial::check_protocol(&id)?;
    let status = client.unlock_status()?;
    let lock = match (status.unlocked, status.in_progress) {
        (true, _) => Lock::Unlocked,
        (false, true) => Lock::Unlocking,
        (false, false) => Lock::Locked,
    };
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
            s.loaded = false; // the other program may have changed the keymap
            out.push(DeviceEvent::Resumed);
        }
    }
    if s.paused {
        return Ok(HOLDER_CHECK_INTERVAL);
    }

    if s.lock == Lock::Unlocking {
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
        log::error!("cannot start the device thread: {e}");
    }
    cmd_tx
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib device`
Expected: 11 passed.

- [ ] **Step 5: Commit**

```bash
git add src/device.rs src/lib.rs
git commit -m "feat: device worker with hotplug, pause, unlock and matrix streaming"
```

---

### Task 12: App state

**Files:**
- Create: `src/state.rs`
- Modify: `src/lib.rs` (add `pub mod state;`)

**Interfaces:**
- Consumes: `Layout`, `Keymap::{resolve, find_position}`, `KeyHit`, `LayerTracker`, `TriLayer`, `HostLayout::char_for`, `keycodes::{decode, Action, label, basic_name, tap_basic, adds_shift}`, `input::OsKey`.
- Produces:
  - `state::Tier { Focused, Unfocused, Matrix }` with `describe() -> &'static str`.
  - `state::LastKey { label: String, text: Option<String>, layer: u8, position: Option<(u8, u8)>, inferred: bool }`.
  - `state::AppState`:
    - public fields: `layout`, `keymap`, `last`, `matrix_active`, `evdev_active`
    - setup: `new(host, tri, always_tri)`, `host()`, `set_keyboard(layout, keymap)`, `clear_keyboard()`, `reset_tracking()`, `set_matrix_active(bool)`
    - queries: `tier()`, `layer_mask(now)`, `active_layer(now)`, `held_positions() -> BTreeSet<(u8, u8)>`
    - input: `matrix_changed(&pressed, &released, now)`, `os_key(&OsKey, now)`, `on_text(&str)`

- [ ] **Step 1: Write failing tests at the bottom of `src/state.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::fake::{SMALL_DEFINITION, SMALL_KEYMAP};
    use crate::input::OsSource;

    // SMALL_KEYMAP: layer 0: A B C / MO(1) SPC LSFT; layer 1: 1 TRNS 3 / TRNS TRNS TRNS
    fn state() -> AppState {
        let mut s = AppState::new(HostLayout::Gb, TriLayer::default(), false);
        let layout = Layout::from_definition(&serde_json::from_str(SMALL_DEFINITION).unwrap()).unwrap();
        let buf: Vec<u8> = SMALL_KEYMAP.iter().flat_map(|c| c.to_be_bytes()).collect();
        s.set_keyboard(layout, Keymap::from_buffer(2, 2, 3, &buf).unwrap());
        s
    }

    fn os(name: &str, usages: &[u8], pressed: bool) -> OsKey {
        OsKey { source: OsSource::Evdev, usages: usages.to_vec(), pressed, name: name.into() }
    }

    #[test]
    fn matrix_press_reports_code_character_and_shift() {
        let mut s = state();
        s.set_matrix_active(true);
        let now = Instant::now();
        s.matrix_changed(&[(0, 0)], &[], now);
        assert_eq!(s.last.as_ref().unwrap().label, "KC_A");
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("a"));
        s.matrix_changed(&[(1, 2)], &[(0, 0)], now); // hold LSFT
        s.matrix_changed(&[(0, 0)], &[], now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("A"));
        assert_eq!(s.held_positions(), BTreeSet::from([(0, 0), (1, 2)]));
    }

    #[test]
    fn matrix_layer_key_changes_what_keys_do() {
        let mut s = state();
        s.set_matrix_active(true);
        let now = Instant::now();
        s.matrix_changed(&[(1, 0)], &[], now); // MO(1)
        assert_eq!(s.active_layer(now), 1);
        s.matrix_changed(&[(0, 0)], &[], now);
        let last = s.last.clone().unwrap();
        assert_eq!((last.label.as_str(), last.layer, last.text.as_deref()), ("KC_1", 1, Some("1")));
        s.matrix_changed(&[(0, 1)], &[], now); // TRNS on layer 1 → B from layer 0
        assert_eq!(s.last.as_ref().unwrap().label, "KC_B");
        assert_eq!(s.last.as_ref().unwrap().layer, 0);
    }

    #[test]
    fn os_keys_are_placed_by_keymap_search_without_matrix() {
        let mut s = state();
        let now = Instant::now();
        s.os_key(&os("evdev 48", &[0x05], true), now);
        let last = s.last.clone().unwrap();
        assert_eq!((last.position, last.inferred, last.label.as_str()), (Some((0, 1)), true, "KC_B"));
        assert_eq!(s.held_positions(), BTreeSet::from([(0, 1)]));
        s.os_key(&os("evdev 48", &[0x05], false), now);
        assert!(s.held_positions().is_empty());
        s.os_key(&os("evdev 4", &[0x20], true), now); // KC_3 exists only on layer 1
        assert_eq!(s.last.as_ref().unwrap().layer, 1);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("3"));
    }

    #[test]
    fn os_shift_applies_to_characters() {
        let mut s = state();
        let now = Instant::now();
        s.os_key(&os("evdev 42", &[0xE1], true), now);
        s.os_key(&os("evdev 4", &[0x20], true), now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("£"));
    }

    #[test]
    fn os_positions_are_ignored_while_matrix_is_active() {
        let mut s = state();
        s.set_matrix_active(true);
        s.os_key(&os("evdev 48", &[0x05], true), Instant::now());
        assert!(s.last.is_none());
        assert!(s.held_positions().is_empty());
    }

    #[test]
    fn works_without_a_keymap_and_text_overrides() {
        let mut s = AppState::new(HostLayout::Gb, TriLayer::default(), false);
        s.os_key(&os("A", &[0x04], true), Instant::now());
        assert_eq!(s.last.as_ref().unwrap().label, "KC_A");
        s.on_text("á");
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("á"));
    }

    #[test]
    fn tiers() {
        let mut s = state();
        assert_eq!(s.tier(), Tier::Focused);
        s.evdev_active = true;
        assert_eq!(s.tier(), Tier::Unfocused);
        s.set_matrix_active(true);
        assert_eq!(s.tier(), Tier::Matrix);
        s.clear_keyboard();
        assert_eq!(s.tier(), Tier::Focused);
    }
}
```

- [ ] **Step 2: Add `pub mod state;` to `src/lib.rs` and run the tests to verify they fail**

Run: `cargo test --lib state`
Expected: compile errors (items not found).

- [ ] **Step 3: Implement `src/state.rs` (above the tests)**

```rust
//! Everything the UI shows, merged from the three input tiers.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use crate::hostlayout::HostLayout;
use crate::input::OsKey;
use crate::keycodes::{self, Action};
use crate::keymap::Keymap;
use crate::layers::{LayerTracker, TriLayer};
use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Focused,
    Unfocused,
    Matrix,
}

impl Tier {
    pub fn describe(self) -> &'static str {
        match self {
            Tier::Focused => "focused window only",
            Tier::Unfocused => "all windows, no layer tracking",
            Tier::Matrix => "all windows, live layers",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastKey {
    /// QMK name of what the key did, e.g. `KC_QUOT` or `LT(2,KC_SPC)`.
    pub label: String,
    /// The character typed, if printable.
    pub text: Option<String>,
    pub layer: u8,
    pub position: Option<(u8, u8)>,
    /// Position and layer were guessed from an OS event rather than seen in the matrix.
    pub inferred: bool,
}

pub struct AppState {
    pub layout: Option<Layout>,
    pub keymap: Option<Keymap>,
    pub last: Option<LastKey>,
    pub matrix_active: bool,
    pub evdev_active: bool,
    host: HostLayout,
    tracker: LayerTracker,
    matrix_held: BTreeSet<(u8, u8)>,
    /// OS keys currently down (by `OsKey::name`) and where we placed them.
    os_held: HashMap<String, Option<(u8, u8)>>,
    os_shift: bool,
}

impl AppState {
    pub fn new(host: HostLayout, tri: TriLayer, always_tri: bool) -> Self {
        Self {
            layout: None,
            keymap: None,
            last: None,
            matrix_active: false,
            evdev_active: false,
            host,
            tracker: LayerTracker::new(tri, always_tri),
            matrix_held: BTreeSet::new(),
            os_held: HashMap::new(),
            os_shift: false,
        }
    }

    pub fn host(&self) -> HostLayout {
        self.host
    }

    pub fn set_keyboard(&mut self, layout: Layout, keymap: Keymap) {
        self.layout = Some(layout);
        self.keymap = Some(keymap);
        self.reset_tracking();
    }

    pub fn clear_keyboard(&mut self) {
        self.layout = None;
        self.keymap = None;
        self.matrix_active = false;
        self.evdev_active = false;
        self.reset_tracking();
    }

    /// Forgets held keys and layer state.
    pub fn reset_tracking(&mut self) {
        self.tracker.reset();
        self.matrix_held.clear();
        self.os_held.clear();
        self.os_shift = false;
    }

    pub fn set_matrix_active(&mut self, active: bool) {
        if !active {
            self.matrix_held.clear();
            self.tracker.reset();
        }
        self.matrix_active = active;
    }

    pub fn tier(&self) -> Tier {
        if self.matrix_active {
            Tier::Matrix
        } else if self.evdev_active {
            Tier::Unfocused
        } else {
            Tier::Focused
        }
    }

    /// Active layers as a QMK bitmask. Without the matrix tier only layer 0 is known.
    pub fn layer_mask(&self, now: Instant) -> u32 {
        if self.matrix_active { self.tracker.mask(now) } else { 1 }
    }

    pub fn active_layer(&self, now: Instant) -> u8 {
        31 - self.layer_mask(now).leading_zeros() as u8
    }

    pub fn held_positions(&self) -> BTreeSet<(u8, u8)> {
        if self.matrix_active {
            self.matrix_held.clone()
        } else {
            self.os_held.values().flatten().copied().collect()
        }
    }

    pub fn matrix_changed(&mut self, pressed: &[(u8, u8)], released: &[(u8, u8)], now: Instant) {
        for &(row, col) in released {
            self.matrix_held.remove(&(row, col));
            self.tracker.release(row, col, now);
        }
        for &(row, col) in pressed {
            self.matrix_held.insert((row, col));
            let Some(keymap) = &self.keymap else { continue };
            let (layer, code) = keymap.resolve(self.tracker.mask(now), row, col);
            self.tracker.press(row, col, code, now);
            let shift = self.matrix_shift(now);
            self.last = Some(self.describe(code, layer, Some((row, col)), shift, false));
        }
    }

    pub fn os_key(&mut self, key: &OsKey, now: Instant) {
        if key.usages.iter().any(|&u| u == 0xE1 || u == 0xE5) {
            self.os_shift = key.pressed;
        }
        if self.matrix_active {
            return; // the matrix already knows exactly which key it was
        }
        if !key.pressed {
            self.os_held.remove(&key.name);
            return;
        }
        let mask = self.layer_mask(now);
        let hit = self.keymap.as_ref().and_then(|km| km.find_position(mask, &key.usages));
        self.os_held.insert(key.name.clone(), hit.map(|h| (h.row, h.col)));
        self.last = Some(match hit {
            Some(h) => self.describe(h.code, h.layer, Some((h.row, h.col)), self.os_shift, true),
            None => {
                let usage = key.usages.first().copied();
                LastKey {
                    label: usage.and_then(keycodes::basic_name).map_or_else(|| key.name.clone(), str::to_owned),
                    text: usage.and_then(|u| self.host.char_for(u, self.os_shift)).map(String::from),
                    layer: 0,
                    position: None,
                    inferred: true,
                }
            }
        });
    }

    /// Text from the focused window is the ground truth for what was typed.
    pub fn on_text(&mut self, text: &str) {
        if let Some(last) = &mut self.last {
            last.text = Some(text.to_owned());
        }
    }

    fn matrix_shift(&self, now: Instant) -> bool {
        let Some(keymap) = &self.keymap else { return self.os_shift };
        let mask = self.tracker.mask(now);
        self.os_shift
            || self
                .matrix_held
                .iter()
                .any(|&(r, c)| matches!(keycodes::decode(keymap.resolve(mask, r, c).1), Action::Basic(0xE1 | 0xE5)))
    }

    fn describe(&self, code: u16, layer: u8, position: Option<(u8, u8)>, shift: bool, inferred: bool) -> LastKey {
        let shift = shift || keycodes::adds_shift(code);
        let text = keycodes::tap_basic(code).and_then(|b| self.host.char_for(b, shift)).map(String::from);
        LastKey { label: keycodes::label(code), text, layer, position, inferred }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib state`
Expected: 7 passed.

- [ ] **Step 5: Commit**

```bash
git add src/state.rs src/lib.rs
git commit -m "feat: app state merging matrix and OS key events"
```

---

### Task 13: Config, setup hints and README

**Files:**
- Create: `src/config.rs`, `src/hints.rs`, `README.md`
- Modify: `src/lib.rs` (add `pub mod config;` and `pub mod hints;`)

**Interfaces:**
- Consumes: `HostLayout` (Task 8), `TriLayer` (Task 6).
- Produces:
  - `config::Config { host_layout: HostLayout, tri_layer: Vec<u8> }`: `Default` (gb, `[1, 2, 3]`), `parse(&str) -> Result<Config, String>`, `load_from(&Path) -> Result<Config, String>`, `tri() -> (TriLayer, bool)`.
  - `config::config_path() -> PathBuf`.
  - `hints::{Hint { title, explanation, commands: Vec<String> }, hidraw_hint(), evdev_hint(), unlock_hint(), install_command(file, rule) -> String}`.
  - `hints` constants `HIDRAW_RULE_FILE`, `HIDRAW_RULE`, `EVDEV_RULE_FILE`, `EVDEV_RULE`, `RELOAD_COMMAND`.

- [ ] **Step 1: Write failing tests**

Bottom of `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.host_layout, HostLayout::Gb);
        assert_eq!(c.tri(), (TriLayer::default(), true));
        assert_eq!(Config::parse("").unwrap(), c);
    }

    #[test]
    fn parses_values() {
        let c = Config::parse("host_layout = \"us\"\ntri_layer = [2, 3, 4]\n").unwrap();
        assert_eq!(c.host_layout, HostLayout::Us);
        assert_eq!(c.tri(), (TriLayer { lower: 2, upper: 3, adjust: 4 }, true));
        assert!(!Config::parse("tri_layer = []").unwrap().tri().1);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(Config::parse("tri_layer = [1, 2]").is_err());
        assert!(Config::parse("host_layout = \"fr\"").is_err());
        assert!(Config::parse("colour = \"red\"").is_err());
    }

    #[test]
    fn missing_file_means_defaults() {
        assert_eq!(Config::load_from(Path::new("/nonexistent/lily58-assistant.toml")).unwrap(), Config::default());
    }
}
```

Bottom of `src/hints.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_contains_every_setup_command_verbatim() {
        let readme = include_str!("../README.md");
        for hint in [hidraw_hint(), evdev_hint()] {
            assert!(!hint.commands.is_empty());
            for cmd in &hint.commands {
                assert!(readme.contains(cmd.as_str()), "README.md is missing:\n{cmd}");
            }
        }
    }

    #[test]
    fn install_command_quotes_the_rule() {
        assert_eq!(install_command("/etc/x.rules", "A==\"b\""), "echo 'A==\"b\"' | sudo tee /etc/x.rules");
    }
}
```

- [ ] **Step 2: Add the modules to `src/lib.rs` and run the tests to verify they fail**

Create an empty `README.md` so `include_str!` compiles.

Run: `cargo test --lib config hints`
Expected: compile errors (items not found).

- [ ] **Step 3: Implement `src/config.rs` (above the tests)**

```rust
//! `~/.config/lily58-assistant/config.toml`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::hostlayout::HostLayout;
use crate::layers::TriLayer;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// OS keyboard layout used to show characters: "gb" or "us".
    pub host_layout: HostLayout,
    /// [lower, upper, adjust] for tri-layer emulation; [] turns it off.
    pub tri_layer: Vec<u8>,
}

impl Default for Config {
    fn default() -> Self {
        Self { host_layout: HostLayout::Gb, tri_layer: vec![1, 2, 3] }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, String> {
        let config: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        if !(config.tri_layer.is_empty() || config.tri_layer.len() == 3) {
            return Err("tri_layer must be [] or [lower, upper, adjust]".into());
        }
        Ok(config)
    }

    /// A missing file means defaults; an unreadable or invalid one is an error.
    pub fn load_from(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Tri-layer layers and whether to emulate it for plain MO keys.
    pub fn tri(&self) -> (TriLayer, bool) {
        match self.tri_layer[..] {
            [lower, upper, adjust] => (TriLayer { lower, upper, adjust }, true),
            _ => (TriLayer::default(), false),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("lily58-assistant/config.toml")
}
```

- [ ] **Step 4: Implement `src/hints.rs` (above the tests)**

```rust
//! Setup instructions shown in the app. README.md repeats them verbatim (checked by a test).

pub const HIDRAW_RULE_FILE: &str = "/etc/udev/rules.d/59-vial.rules";
/// Vial's standard rule: raw-HID access to Vial keyboards for the logged-in user.
pub const HIDRAW_RULE: &str = r#"KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{serial}=="*vial:f64c2b3c*", MODE="0660", GROUP="users", TAG+="uaccess", TAG+="udev-acl""#;
/// Must sort before systemd's 73-seat-late.rules for `uaccess` to apply.
pub const EVDEV_RULE_FILE: &str = "/etc/udev/rules.d/70-lily58-assistant.rules";
pub const EVDEV_RULE: &str = r#"SUBSYSTEM=="input", KERNEL=="event*", ATTRS{idVendor}=="7171", ATTRS{idProduct}=="0012", TAG+="uaccess""#;
pub const RELOAD_COMMAND: &str = "sudo udevadm control --reload-rules && sudo udevadm trigger";

pub struct Hint {
    pub title: &'static str,
    pub explanation: &'static str,
    pub commands: Vec<String>,
}

pub fn install_command(file: &str, rule: &str) -> String {
    format!("echo '{rule}' | sudo tee {file}")
}

pub fn hidraw_hint() -> Hint {
    Hint {
        title: "Allow access to the keyboard",
        explanation: "The assistant reads the layout and keymap over the keyboard's raw-HID interface \
                      (read-only). Vial needs the same rule, so if Vial works on this PC you already have it.",
        commands: vec![install_command(HIDRAW_RULE_FILE, HIDRAW_RULE), RELOAD_COMMAND.to_owned()],
    }
}

pub fn evdev_hint() -> Hint {
    Hint {
        title: "Track keys typed into other windows",
        explanation: "Lets the assistant read the Lily58's own input device, so highlights keep working while \
                      another window has focus. Trade-off: any program running as you can then read keystrokes \
                      from this keyboard.",
        commands: vec![install_command(EVDEV_RULE_FILE, EVDEV_RULE), RELOAD_COMMAND.to_owned()],
    }
}

pub fn unlock_hint() -> Hint {
    Hint {
        title: "Unlock the keyboard for live layer tracking",
        explanation: "Vial only reports the switch matrix after its physical unlock. Click the button below, then \
                      hold the highlighted keys for about 5 seconds. The keyboard forgets the unlock when unplugged. \
                      An unlock can't be cancelled once started; unplug the keyboard to abort.",
        commands: Vec::new(),
    }
}
```

- [ ] **Step 5: Write `README.md`**

If Task 1 needed any `-dev` packages or the glow renderer, add them to the Building section.

````markdown
# Lily58 Assistant

An on-screen companion for learning a Lily58 split keyboard running Vial firmware. It draws your keyboard, lights up the keys you press, shows what the last key typed, and follows layer changes as they happen. Everything is read from the keyboard itself.

**Strictly read-only.** The assistant never changes your keymap: every message it sends to the keyboard passes an allowlist of read commands (`src/hid/guard.rs`). The one exception is Vial's two-command unlock handshake, which enables live layer tracking and changes no keymap data. Use [Vial](https://get.vial.today) to remap.

## What it can see

| Tier | Needs | Shows |
|---|---|---|
| Focused window | nothing | keys typed into the assistant's own window |
| All windows | the optional `/dev/input` udev rule | keys typed anywhere; layer keys are invisible, so the layer is guessed |
| Live layers | unlocking the keyboard (each time it's plugged in) | every physical key, including LOWER/RAISE, with instant layer switching |

The status bar shows the active tier; **how to enable more…** opens these instructions in the app.

## Building

Needs Rust 1.95 or newer (`rustc --version`).

**Fedora**

```bash
sudo dnf install rust cargo gcc
cargo build --release
```

**Ubuntu**: if the packaged Rust is older than 1.95, use rustup:

```bash
sudo apt install build-essential curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release
```

Run it with `./target/release/lily58-assistant`.

## Permissions

### Keyboard access (needed for the keyboard picture)

The assistant reads the layout and keymap over the keyboard's raw-HID interface, the same access Vial needs. If Vial works on this PC, you already have this. Otherwise:

```bash
echo 'KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{serial}=="*vial:f64c2b3c*", MODE="0660", GROUP="users", TAG+="uaccess", TAG+="udev-acl"' | sudo tee /etc/udev/rules.d/59-vial.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### All-windows tracking (optional)

```bash
echo 'SUBSYSTEM=="input", KERNEL=="event*", ATTRS{idVendor}=="7171", ATTRS{idProduct}=="0012", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/70-lily58-assistant.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then press **Reload** (Ctrl+R) in the assistant. Trade-off: with this rule in place, any program running as you can read keystrokes from the Lily58 (only that keyboard).

## Live layer tracking

Vial only reports the switch matrix after its physical unlock. In the assistant, open **how to enable more…**, click **Unlock for layer tracking**, and hold the highlighted keys until the bar fills (about 5 seconds). The keyboard forgets the unlock when it's unplugged.

An unlock can't be cancelled once started: until it completes, the keyboard answers nothing else over raw HID. To abort, unplug the keyboard. Vial behaves the same way.

## Configuration

Optional: `~/.config/lily58-assistant/config.toml` (or `$XDG_CONFIG_HOME/lily58-assistant/config.toml`):

```toml
# OS keyboard layout used to show characters: "gb" (default) or "us"
host_layout = "gb"

# Holding layers 1 and 2 shows layer 3, like the stock Lily58 firmware
# (update_tri_layer_state in keymap.c). Set to [] if your firmware doesn't do this.
tri_layer = [1, 2, 3]
```

## Keeping the window on top

The assistant doesn't do this itself, because Wayland offers no portable way. Use your desktop instead:

- **KDE Plasma:** Alt+F3 → More Actions → Keep Above Others
- **GNOME:** Alt+Space → Always on Top

## Known limits

- Layer logic compiled into the firmware isn't visible over Vial; `tri_layer` covers the common case.
- Tap/hold keys (e.g. `LT`) are approximated with QMK's default 200 ms tapping term.
- Without live layers, the position of a key typed elsewhere is a best guess: layer 0 first, then the other layers.
- While Vial or another program has the keyboard open, the assistant pauses and resumes when it's closed.

## Troubleshooting

```bash
./target/release/lily58-assistant --probe
```

Prints the device, permissions, protocol versions, layout, full keymap and unlock state, using only read commands. For logs, run with `RUST_LOG=debug`.
````

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib config hints`
Expected: 4 + 2 passed.

- [ ] **Step 7: Commit**

```bash
git add src/config.rs src/hints.rs src/lib.rs README.md
git commit -m "feat: config file, setup hints, README"
```

---

### Task 14: GUI

**Files:**
- Create: `src/ui/mod.rs`, `src/ui/keyboard.rs`, `src/ui/status.rs`, `src/ui/dialogs.rs`
- Modify: `src/main.rs` (use `ui::run`, delete `Smoke`), `src/lib.rs` (add `pub mod ui;`)

**Interfaces:**
- Consumes:
  - `device::{spawn, SystemConnector, DeviceEvent, DeviceCommand}`
  - `input::{InputMsg, evdev::{find_event_nodes, start, EvdevStatus}, focused::{translate, FocusedInput}}`
  - `state::{AppState, Tier}`
  - `config::{Config, config_path}`
  - `hints::*`
  - `keycodes::{decode, Action, label, tap_basic, adds_shift, KC_NO}`
  - `HostLayout`, `KeyGeom::{corners, center}`, `Layout::bounds`, `protocol::UNLOCK_COUNTER_MAX`
- Produces: `ui::run() -> eframe::Result`; `ui::keyboard::{Keycap { main, shifted }, keycap(code, HostLayout) -> Keycap}`.

- [ ] **Step 1: Write the failing keycap test at the bottom of `src/ui/keyboard.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn cap(main: &str, shifted: Option<&str>) -> Keycap {
        Keycap { main: main.into(), shifted: shifted.map(Into::into) }
    }

    #[test]
    fn keycaps_show_what_the_key_types() {
        let gb = HostLayout::Gb;
        assert_eq!(keycap(0x0004, gb), cap("A", None));
        assert_eq!(keycap(0x001F, gb), cap("2", Some("\"")));
        assert_eq!(keycap(0x0032, gb), cap("#", Some("~")));
        assert_eq!(keycap(0x002C, gb), cap("Space", None));
        assert_eq!(keycap(0x021E, gb), cap("!", None)); // KC_EXLM
        assert_eq!(keycap(0x5221, gb), cap("MO(1)", None));
        assert_eq!(keycap(0x0028, gb), cap("ENT", None));
        assert_eq!(keycap(0x422C, gb), cap("LT(2,SPC)", None));
        assert_eq!(keycap(KC_NO, gb), cap("", None));
    }
}
```

Create `src/ui/mod.rs` containing only `pub mod keyboard;` for now, and add `pub mod ui;` to `src/lib.rs`.

Run: `cargo test --lib ui`
Expected: compile errors (`keycap` not found).

- [ ] **Step 2: Implement `src/ui/keyboard.rs` (above the tests)**

```rust
//! Draws the keyboard from the layout the keyboard reported.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Vec2};

use crate::hostlayout::HostLayout;
use crate::keycodes::{self, Action, KC_NO};
use crate::layout::KeyGeom;
use crate::state::AppState;

const HELD: Color32 = Color32::from_rgb(90, 170, 255);
const UNLOCK: Color32 = Color32::from_rgb(255, 170, 60);
/// Gap around each key, in key units.
const GAP: f32 = 0.05;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keycap {
    pub main: String,
    pub shifted: Option<String>,
}

/// What to print on a key: the characters it types on the host layout, else a short keycode name.
pub fn keycap(code: u16, host: HostLayout) -> Keycap {
    let plain = |main: String| Keycap { main, shifted: None };
    if code == KC_NO {
        return plain(String::new());
    }
    if let Action::Basic(b) = keycodes::decode(code) {
        match (host.char_for(b, false), host.char_for(b, true)) {
            (Some(' '), _) => return plain("Space".into()),
            (Some(c), _) if c.is_ascii_alphabetic() => return plain(c.to_ascii_uppercase().to_string()),
            (Some(c), Some(s)) if c != s => return Keycap { main: c.to_string(), shifted: Some(s.to_string()) },
            (Some(c), _) => return plain(c.to_string()),
            _ => {}
        }
    }
    if keycodes::adds_shift(code)
        && let Some(c) = keycodes::tap_basic(code).and_then(|b| host.char_for(b, true))
    {
        return plain(c.to_string()); // e.g. LSFT(KC_1) → "!"
    }
    plain(keycodes::label(code).replace("KC_", ""))
}

pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, unlock_keys: &[(u8, u8)]) {
    let (Some(layout), Some(keymap)) = (&state.layout, &state.keymap) else { return };
    let mask = state.layer_mask(now);
    let active = state.active_layer(now);
    let held = state.held_positions();

    let (min_x, min_y, max_x, max_y) = layout.bounds();
    let (span_x, span_y) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let avail = ui.available_size();
    let unit = ((avail.x - 24.0) / span_x).min((avail.y - 24.0) / span_y).max(8.0);
    let (response, painter) = ui.allocate_painter(avail, Sense::hover());
    let origin = response.rect.center() - Vec2::new(span_x, span_y) * unit / 2.0;
    let to_screen = |(x, y): (f32, f32)| Pos2::new(origin.x + (x - min_x) * unit, origin.y + (y - min_y) * unit);
    let visuals = ui.visuals().clone();

    for key in &layout.keys {
        let (layer, code) = keymap.resolve(mask, key.row, key.col);
        let pos = (key.row, key.col);
        let (is_held, is_unlock) = (held.contains(&pos), unlock_keys.contains(&pos));
        let fill = if is_held {
            HELD
        } else if is_unlock {
            UNLOCK
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let text_color = if is_held || is_unlock {
            Color32::BLACK
        } else if layer < active {
            visuals.weak_text_color() // transparent key: showing a lower layer
        } else {
            visuals.text_color()
        };
        let inner = KeyGeom { x: key.x + GAP, y: key.y + GAP, w: key.w - 2.0 * GAP, h: key.h - 2.0 * GAP, ..key.clone() };
        let points: Vec<Pos2> = inner.corners().into_iter().map(to_screen).collect();
        painter.add(Shape::convex_polygon(points, fill, Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color)));

        let cap = keycap(code, state.host());
        let center = to_screen(inner.center());
        let size = (unit * 0.3).clamp(9.0, 20.0);
        match &cap.shifted {
            Some(shifted) => {
                let font = FontId::proportional(size * 0.85);
                painter.text(center - Vec2::new(0.0, unit * 0.18), Align2::CENTER_CENTER, shifted, font.clone(), text_color);
                painter.text(center + Vec2::new(0.0, unit * 0.18), Align2::CENTER_CENTER, &cap.main, font, text_color);
            }
            None => {
                let chars = cap.main.chars().count().max(1) as f32;
                let fit = (inner.w * unit * 0.9 / (chars * size * 0.6)).clamp(0.45, 1.0);
                painter.text(center, Align2::CENTER_CENTER, &cap.main, FontId::proportional(size * fit), text_color);
            }
        }
    }
}
```

- [ ] **Step 3: Run the keycap test to verify it passes**

Run: `cargo test --lib ui`
Expected: 1 passed.

- [ ] **Step 4: Write `src/ui/status.rs`**

```rust
//! The status bar: last key, layer, tracking tier.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText};

use super::{App, Connection};
use crate::state::Tier;

pub(super) fn show(ui: &mut egui::Ui, app: &mut App, now: Instant) {
    ui.horizontal_wrapped(|ui| {
        match &app.state.last {
            Some(last) => {
                let typed = last.text.as_deref().map(|t| format!("  →  {}", visible(t))).unwrap_or_default();
                ui.label(RichText::new(format!("{}{typed}", last.label)).monospace().strong());
                if last.inferred && last.layer != 0 {
                    ui.label(format!("(probably layer {})", last.layer));
                }
            }
            None => {
                ui.label("Type something…");
            }
        }
        ui.separator();
        if app.state.matrix_active {
            ui.label(format!("Layer {}", app.state.active_layer(now)));
        } else {
            ui.label("Layer: unknown");
        }
        ui.separator();
        let tier = app.state.tier();
        ui.label(format!("Tracking: {}", tier.describe()));
        if tier != Tier::Matrix && ui.link("how to enable more…").clicked() {
            app.show_hints = true;
        }
        if let Connection::Paused(holders) = &app.connection {
            ui.separator();
            ui.colored_label(Color32::from_rgb(230, 180, 40), format!("paused: {} has the keyboard open", holders.join(", ")));
        }
        ui.separator();
        if ui.button("Reload (Ctrl+R)").clicked() {
            app.reload();
        }
    });
    if let Some(err) = &app.error {
        ui.colored_label(Color32::from_rgb(230, 90, 90), err);
    }
}

fn visible(text: &str) -> String {
    if text == " " { "Space".into() } else { text.to_owned() }
}
```

- [ ] **Step 5: Write `src/ui/dialogs.rs`**

```rust
//! The unlock progress window and the tracking-tiers/setup window.

use eframe::egui::{self, Align2, RichText};

use super::{App, Connection, Unlock};
use crate::hints::{self, Hint};
use crate::input::evdev::EvdevStatus;
use crate::protocol::UNLOCK_COUNTER_MAX;

pub(super) fn show(ctx: &egui::Context, app: &mut App) {
    if let Unlock::InProgress { counter } = app.unlock {
        unlock_window(ctx, counter, &app.unlock_keys, app.state.layout.is_some());
    }
    if app.show_hints {
        hints_window(ctx, app);
    }
}

/// Not closable: once started, only completing the unlock or unplugging ends it.
fn unlock_window(ctx: &egui::Context, counter: u8, keys: &[(u8, u8)], picture: bool) {
    egui::Window::new("Unlocking keyboard")
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_TOP, [0.0, 12.0])
        .show(ctx, |ui| {
            if picture {
                ui.label("Hold the highlighted keys until the bar fills (about 5 seconds).");
            } else {
                ui.label(format!("Hold the keys at matrix positions (row, col) {keys:?} until the bar fills."));
            }
            let progress = 1.0 - f32::from(counter) / f32::from(UNLOCK_COUNTER_MAX);
            ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0)));
            ui.small("An unlock can't be cancelled: to abort, unplug the keyboard. (Vial behaves the same way.)");
        });
}

fn hints_window(ctx: &egui::Context, app: &mut App) {
    let mut open = true;
    egui::Window::new("Tracking tiers").open(&mut open).collapsible(false).default_width(560.0).show(ctx, |ui| {
        ui.label(format!("{} Focused window: always available", mark(true)));
        if matches!(app.connection, Connection::NoAccess(_)) {
            ui.label(format!("{} Keyboard picture and keymap", mark(false)));
            hint_block(ui, &hints::hidraw_hint());
        }
        let evdev_ok = app.evdev == EvdevStatus::Active;
        ui.label(format!("{} All windows (/dev/input)", mark(evdev_ok)));
        if !evdev_ok {
            hint_block(ui, &hints::evdev_hint());
            ui.label("Then press Reload (Ctrl+R).");
        }
        let matrix_ok = app.unlock == Unlock::Unlocked;
        ui.label(format!("{} Live layers (Vial matrix)", mark(matrix_ok)));
        if !matrix_ok {
            hint_block(ui, &hints::unlock_hint());
            let can_unlock = app.unlock == Unlock::Locked && app.connection == Connection::Connected;
            if ui.add_enabled(can_unlock, egui::Button::new("Unlock for layer tracking")).clicked() {
                app.start_unlock();
            }
        }
    });
    if !open {
        app.show_hints = false;
    }
}

fn hint_block(ui: &mut egui::Ui, hint: &Hint) {
    ui.indent(hint.title, |ui| {
        ui.label(RichText::new(hint.title).strong());
        ui.label(hint.explanation);
        for cmd in &hint.commands {
            ui.horizontal(|ui| {
                ui.code(cmd.as_str());
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
            });
        }
    });
}

fn mark(ok: bool) -> &'static str {
    if ok { "✔" } else { "✖" }
}
```

- [ ] **Step 6: Write `src/ui/mod.rs`**

```rust
//! The eframe application: connects the device worker and input sources to the screen.

mod dialogs;
pub mod keyboard;
mod status;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::{self, Config};
use crate::device::{self, DeviceCommand, DeviceEvent, SystemConnector};
use crate::input::InputMsg;
use crate::input::evdev::{self as evdev_input, EvdevStatus};
use crate::input::focused::{self, FocusedInput};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Connection {
    Waiting,
    NoAccess(PathBuf),
    Paused(Vec<String>),
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unlock {
    Unknown,
    Locked,
    InProgress { counter: u8 },
    Unlocked,
}

pub struct App {
    state: AppState,
    connection: Connection,
    unlock: Unlock,
    unlock_keys: Vec<(u8, u8)>,
    evdev: EvdevStatus,
    evdev_usb_dir: Option<PathBuf>,
    error: Option<String>,
    show_hints: bool,
    device_rx: Receiver<DeviceEvent>,
    device_tx: Sender<DeviceCommand>,
    input_tx: Sender<InputMsg>,
    input_rx: Receiver<InputMsg>,
    ctx: egui::Context,
}

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Lily58 Assistant")
            .with_app_id("lily58-assistant")
            .with_inner_size([960.0, 460.0])
            .with_min_inner_size([480.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native("Lily58 Assistant", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (config, error) = match Config::load_from(&config::config_path()) {
            Ok(config) => (config, None),
            Err(e) => (Config::default(), Some(format!("Config ignored: {e}"))),
        };
        let (tri, always_tri) = config.tri();
        let ctx = cc.egui_ctx.clone();
        let (events_tx, device_rx) = mpsc::channel();
        let repaint = ctx.clone();
        let device_tx = device::spawn(SystemConnector, events_tx, move || repaint.request_repaint());
        let (input_tx, input_rx) = mpsc::channel();
        Self {
            state: AppState::new(config.host_layout, tri, always_tri),
            connection: Connection::Waiting,
            unlock: Unlock::Unknown,
            unlock_keys: Vec::new(),
            evdev: EvdevStatus::NotFound,
            evdev_usb_dir: None,
            error,
            show_hints: false,
            device_rx,
            device_tx,
            input_tx,
            input_rx,
            ctx,
        }
    }

    fn on_device_event(&mut self, event: DeviceEvent, now: Instant) {
        match event {
            DeviceEvent::Waiting => self.connection = Connection::Waiting,
            DeviceEvent::NoAccess(path) => self.connection = Connection::NoAccess(path),
            DeviceEvent::Paused { holders } => {
                self.connection = Connection::Paused(holders);
                self.state.set_matrix_active(false);
            }
            DeviceEvent::Resumed => {} // a fresh Connected follows
            DeviceEvent::Connected { info, layout, keymap } => {
                self.connection = Connection::Connected;
                self.error = None;
                self.state.set_keyboard(layout, keymap);
                if self.evdev_usb_dir.as_deref() != Some(info.usb_dir.as_path()) {
                    self.start_evdev(&info.usb_dir);
                }
            }
            DeviceEvent::Disconnected => {
                self.connection = Connection::Waiting;
                self.unlock = Unlock::Unknown;
                self.state.clear_keyboard();
                self.evdev = EvdevStatus::NotFound;
                self.evdev_usb_dir = None;
            }
            DeviceEvent::Locked { unlock_keys } => {
                self.unlock = Unlock::Locked;
                self.unlock_keys = unlock_keys;
                self.state.set_matrix_active(false);
            }
            DeviceEvent::Unlocking { counter, unlock_keys } => {
                self.unlock = Unlock::InProgress { counter };
                self.unlock_keys = unlock_keys;
            }
            DeviceEvent::Unlocked => {
                self.unlock = Unlock::Unlocked;
                self.state.set_matrix_active(true);
            }
            DeviceEvent::Matrix { pressed, released } => self.state.matrix_changed(&pressed, &released, now),
            DeviceEvent::Error(e) => self.error = Some(e),
        }
    }

    fn start_evdev(&mut self, usb_dir: &Path) {
        let nodes = evdev_input::find_event_nodes(Path::new("/sys"), Path::new("/dev"), usb_dir);
        let repaint = self.ctx.clone();
        self.evdev = evdev_input::start(&nodes, self.input_tx.clone(), move || repaint.request_repaint());
        self.evdev_usb_dir = Some(usb_dir.to_path_buf());
        self.state.evdev_active = self.evdev == EvdevStatus::Active;
    }

    /// Re-reads the keymap and, if all-windows tracking isn't running yet, retries it
    /// (e.g. just after installing the udev rule).
    fn reload(&mut self) {
        let _ = self.device_tx.send(DeviceCommand::Reload);
        self.state.reset_tracking();
        if self.evdev != EvdevStatus::Active
            && let Some(dir) = self.evdev_usb_dir.clone()
        {
            self.start_evdev(&dir);
        }
    }

    fn start_unlock(&mut self) {
        let _ = self.device_tx.send(DeviceCommand::StartUnlock);
    }

    fn unlock_highlight(&self) -> &[(u8, u8)] {
        if matches!(self.unlock, Unlock::InProgress { .. }) { &self.unlock_keys } else { &[] }
    }

    fn central(&mut self, ui: &mut egui::Ui, now: Instant) {
        if self.state.layout.is_some() {
            keyboard::show(ui, &self.state, now, self.unlock_highlight());
            return;
        }
        match &self.connection {
            Connection::NoAccess(path) => {
                ui.heading("Can't read the keyboard");
                ui.label(format!(
                    "{} is not accessible, so the keyboard picture is unavailable. Keys typed into this window still show below.",
                    path.display()
                ));
                if ui.button("How to fix…").clicked() {
                    self.show_hints = true;
                }
            }
            Connection::Paused(holders) => {
                ui.heading("Paused");
                ui.label(format!("{} has the keyboard open. Close it and the assistant reconnects.", holders.join(", ")));
            }
            Connection::Waiting | Connection::Connected => {
                ui.heading("Waiting for Lily58…");
                ui.label("Plug in the keyboard; it's picked up automatically.");
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        while let Ok(event) = self.device_rx.try_recv() {
            self.on_device_event(event, now);
        }
        while let Ok(msg) = self.input_rx.try_recv() {
            match msg {
                InputMsg::Key(key) => self.state.os_key(&key, now),
                InputMsg::EvdevGone(_) => {
                    self.evdev = EvdevStatus::NotFound;
                    self.state.evdev_active = false;
                }
            }
        }
        let events = ui.ctx().input(|i| i.events.clone());
        for input in focused::translate(&events) {
            match input {
                FocusedInput::Key(key) if !self.state.evdev_active => self.state.os_key(&key, now),
                FocusedInput::Key(_) => {} // evdev already reported it
                FocusedInput::Text(text) => self.state.on_text(&text),
            }
        }
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::R)) {
            self.reload();
        }

        egui::Panel::bottom("status").show(ui, |ui| status::show(ui, self, now));
        egui::CentralPanel::default().show(ui, |ui| self.central(ui, now));
        dialogs::show(ui.ctx(), self);

        // Tap-hold keys become holds after the tapping term with no new input, so keep repainting.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}
```

- [ ] **Step 7: Point `main.rs` at the real GUI**

In `src/main.rs`, replace `run_gui`'s body with `lily58_assistant::ui::run().map_err(|e| anyhow::anyhow!("{e}"))`, and delete the `Smoke` struct, its `impl`, and `use eframe::egui;`. If Task 1 needed the glow renderer, add `renderer: eframe::Renderer::Glow,` to `NativeOptions` in `ui::run` instead.

- [ ] **Step 8: Build, lint and test**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: builds with no warnings; clippy is clean (fix anything it reports); all tests pass.

- [ ] **Step 9: Run it against the keyboard**

Close Vial. Run: `cargo run`
Expected:
- the window shows the Lily58 drawn as two halves;
- typing into the window highlights keys and shows e.g. `KC_A  →  a` in the status bar;
- the status bar says `Tracking: focused window only` (or `all windows…` if the evdev rule is installed).

If the egui API differs from this plan (e.g. a renamed method), fix it locally: the plan was written against egui 0.36.2's source.

- [ ] **Step 10: Commit**

```bash
git add src/ui src/main.rs src/lib.rs
git commit -m "feat: GUI with keyboard view, status bar, unlock and setup windows"
```

---

### Task 15: Hardware and desktop verification

**Files:**
- Create: `docs/manual-test-checklist.md`
- Modify: `README.md` only if a check exposes a documentation gap

This task needs the user at the keyboard. Holding unlock keys can't be automated.

- [ ] **Step 1: Write `docs/manual-test-checklist.md`**

```markdown
# Manual test checklist

Run on each PC after changing input, device or UI code. Close Vial unless a step says otherwise.

## Startup
- [ ] Keyboard unplugged: the window shows "Waiting for Lily58…".
- [ ] Plug in: the keyboard picture appears within about a second, as two halves with the thumb clusters in place.
- [ ] `--probe` output matches what the window shows (layer 0 labels).

## Focused window (no evdev rule, keyboard locked)
- [ ] Typing into the window highlights the right keys; the status bar shows e.g. `KC_QUOT  →  '`.
- [ ] Shift+2 shows `"` and Shift+3 shows `£` (UK layout).
- [ ] Typing in another window changes nothing.

## All windows (evdev rule installed)
- [ ] After installing the rule and pressing Reload, the status bar says "all windows, no layer tracking".
- [ ] Typing in another window highlights keys in the assistant.
- [ ] A symbol that exists only on a layer shows "(probably layer N)".

## Live layers (unlocked)
- [ ] "how to enable more…" → "Unlock for layer tracking": the unlock keys turn orange; holding them fills the bar in about 5 s; the window closes.
- [ ] Holding LOWER switches the labels to layer 1 before any other key is pressed; releasing it returns to layer 0.
- [ ] Holding LOWER + RAISE shows layer 3 (with `tri_layer = [1, 2, 3]`).
- [ ] Transparent keys show the lower layer's label, dimmed.
- [ ] The status bar shows "Layer N" and "all windows, live layers".

## Robustness
- [ ] Opening Vial while the assistant runs shows "paused: vial (…) has the keyboard open", and Vial works normally. Closing Vial makes the assistant reconnect.
- [ ] Remapping a key in Vial, then closing Vial, shows the new label after the reconnect.
- [ ] Unplugging mid-session goes back to "Waiting…"; replugging reconnects, locked again.

## Desktops
- [ ] Fedora/KDE: runs; Alt+F3 → More Actions → Keep Above Others keeps it on top.
- [ ] Ubuntu/GNOME: builds from the README; runs; Alt+Space → Always on Top keeps it on top.
```

- [ ] **Step 2: Walk the checklist on the Fedora PC with the user**

Run `cargo run --release` and go through every section except "Ubuntu/GNOME". Record failures. **Stop and report** any failure; don't paper over it.

- [ ] **Step 3: Fix any documentation gaps found, run the full suite, commit**

Run: `cargo clippy --all-targets -- -D warnings && cargo test`
Expected: clean, all tests pass.

```bash
git add docs/manual-test-checklist.md README.md
git commit -m "docs: manual test checklist"
```

The Ubuntu/GNOME section is checked when the user is next at the work PC.
