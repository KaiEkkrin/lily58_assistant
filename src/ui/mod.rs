//! The eframe application: connects the device worker and input sources to the screen.

mod dialogs;
pub mod keyboard;
mod status;
mod tutor_panel;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::config::{self, Config};
use crate::device::{self, DeviceCommand, DeviceEvent, SystemConnector};
use crate::input::InputMsg;
use crate::input::evdev::{self as evdev_input, EvdevStatus};
use crate::input::focused::{self, FocusedInput};
use crate::state::AppState;
use crate::tutor::{self, Availability, Session};
use crate::tutor::drills;

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

/// Shown when the device worker has stopped (it panicked, or its thread never started).
const DEVICE_WORKER_STOPPED: &str =
    "The keyboard worker has stopped, so the keyboard can't be read. Restart the assistant; the log says why.";

pub struct App {
    state: AppState,
    connection: Connection,
    unlock: Unlock,
    unlock_keys: Vec<(u8, u8)>,
    evdev: EvdevStatus,
    evdev_usb_dir: Option<PathBuf>,
    error: Option<String>,
    /// Kept apart from `error`, which device events clear; this stays until dismissed.
    config_error: Option<String>,
    /// The device worker's channel has closed; reported once.
    worker_stopped: bool,
    show_hints: bool,
    tutor: Session,
    /// Everything typed into the window since the last `logic` call, copied in
    /// `raw_input_hook` before egui sees it and drained (applied to `state`) in `logic`, which
    /// eframe calls exactly once per hook call on both the visible and hidden paths.
    typed: Vec<egui::Event>,
    /// The tail of `raw_input.events` (after our own filtering) already folded into `typed`,
    /// so a hidden pass that re-hooks the same not-yet-delivered input isn't recorded twice.
    /// Reset to empty once a real egui pass finally consumes that input (see `raw_input_hook`).
    typed_seen: Vec<egui::Event>,
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
        let (config, config_error) = config::load_or_default(&config::config_path());
        let ctx = cc.egui_ctx.clone();
        let (events_tx, device_rx) = mpsc::channel();
        let repaint = ctx.clone();
        let device_tx = device::spawn(SystemConnector, events_tx, move || repaint.request_repaint());
        Self::with_device(&config, config_error, ctx, device_tx, device_rx)
    }

    /// Everything but starting the device worker, so tests can stand in for it with channels.
    fn with_device(
        config: &Config,
        config_error: Option<String>,
        ctx: egui::Context,
        device_tx: Sender<DeviceCommand>,
        device_rx: Receiver<DeviceEvent>,
    ) -> Self {
        let (tri, always_tri) = config.tri();
        let (input_tx, input_rx) = mpsc::channel();
        Self {
            state: AppState::new(config.host_layout, tri, always_tri),
            connection: Connection::Waiting,
            unlock: Unlock::Unknown,
            unlock_keys: Vec::new(),
            evdev: EvdevStatus::NotFound,
            evdev_usb_dir: None,
            error: None,
            config_error,
            worker_stopped: false,
            show_hints: false,
            tutor: Session::new(),
            typed: Vec::new(),
            typed_seen: Vec::new(),
            device_rx,
            device_tx,
            input_tx,
            input_rx,
            ctx,
        }
    }

    fn on_device_event(&mut self, event: DeviceEvent, now: Instant) {
        match event {
            DeviceEvent::Waiting => {
                self.connection = Connection::Waiting;
                self.unlock = Unlock::Unknown;
                self.error = None; // no keyboard, so any error about it is stale
            }
            DeviceEvent::NoAccess(path) => {
                self.connection = Connection::NoAccess(path);
                self.unlock = Unlock::Unknown;
            }
            DeviceEvent::Paused { holders } => {
                self.connection = Connection::Paused(holders);
                // The other program may lock, unlock or finish an unlock; a resume re-reports it.
                self.unlock = Unlock::Unknown;
                self.state.set_matrix_active(false);
            }
            // The session is live again; a fresh Connected or Unlocking follows.
            DeviceEvent::Resumed => self.connection = Connection::Connected,
            DeviceEvent::Connected { info, layout, keymap } => {
                self.connection = Connection::Connected;
                self.error = None;
                // Check the layout against the finger map before it moves into the state.
                let availability = match tutor::fingers::validate(&layout) {
                    Ok(()) => Availability::Ready,
                    Err(why) => Availability::LayoutMismatch(why),
                };
                self.state.set_keyboard(layout, keymap);
                self.tutor.set_availability(availability);
                // A freshly read keymap invalidates a batch's paths. This arrives on Reload and
                // when another program releases the keyboard, which is how a remap in Vial
                // reaches the drills without restarting the app.
                self.tutor.keyboard_changed();
                if self.evdev_usb_dir.as_deref() != Some(info.usb_dir.as_path()) {
                    self.start_evdev(&info.usb_dir);
                }
            }
            DeviceEvent::Disconnected => {
                self.connection = Connection::Waiting;
                self.unlock = Unlock::Unknown;
                self.state.clear_keyboard();
                self.tutor.set_availability(Availability::NoKeyboard);
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
            DeviceEvent::Error(e) => {
                self.error = Some(e);
                self.unlock = Unlock::Unknown; // errors come only when there is no session
            }
        }
    }

    fn drain_device_events(&mut self, now: Instant) {
        loop {
            match self.device_rx.try_recv() {
                Ok(event) => self.on_device_event(event, now),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.on_worker_stopped();
                    return;
                }
            }
        }
    }

    /// The device worker's end of the channels is gone. Nothing about the keyboard will change
    /// again, so clear it and say so.
    fn on_worker_stopped(&mut self) {
        if self.worker_stopped {
            return;
        }
        self.worker_stopped = true;
        log::error!("the device worker has stopped");
        self.on_device_event(DeviceEvent::Disconnected, Instant::now());
        self.error = Some(DEVICE_WORKER_STOPPED.into());
    }

    fn send(&mut self, cmd: DeviceCommand) {
        if self.device_tx.send(cmd).is_err() {
            self.on_worker_stopped();
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
        self.send(DeviceCommand::Reload);
        self.state.reset_tracking();
        if self.evdev != EvdevStatus::Active
            && let Some(dir) = self.evdev_usb_dir.clone()
        {
            self.start_evdev(&dir);
        }
    }

    fn start_unlock(&mut self) {
        self.tutor.close();
        self.send(DeviceCommand::StartUnlock);
    }

    /// The failure reason is recorded inside the session, so the panel reads it from there
    /// rather than this keeping a second copy that could drift.
    fn start_drill(&mut self, id: drills::DrillId) {
        let Some(keymap) = &self.state.keymap else { return };
        let _ = self.tutor.start(id, keymap, self.state.host());
    }

    fn tutor_input(&mut self, input: tutor::Input, now: Instant) {
        let Some(keymap) = &self.state.keymap else { return };
        self.tutor.input(input, keymap, self.state.host(), now);
    }

    fn unlock_highlight(&self) -> &[(u8, u8)] {
        if matches!(self.unlock, Unlock::InProgress { .. }) { &self.unlock_keys } else { &[] }
    }

    fn central(&mut self, ui: &mut egui::Ui, now: Instant) {
        if self.state.layout.is_some() {
            keyboard::show(ui, &self.state, now, keyboard::View {
                unlock_keys: self.unlock_highlight(),
                fingers: self.tutor.colours_on && self.tutor.is_active(),
                hint: self.tutor.hint(),
            });
            return;
        }
        if self.worker_stopped {
            // The red error in the status bar carries the detail; don't also suggest plugging
            // the keyboard back in when nothing about it will change again.
            ui.heading("Keyboard worker stopped");
            ui.label("Restart the assistant to read the keyboard again.");
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
            Connection::Waiting => {
                ui.heading("Waiting for Lily58…");
                ui.label("Plug in the keyboard; it's picked up automatically.");
            }
            Connection::Connected => {
                ui.heading("Reading the keyboard…");
                let label = if matches!(self.unlock, Unlock::InProgress { .. }) {
                    "Finish the unlock to see the keyboard picture."
                } else {
                    "This takes a moment."
                };
                ui.label(label);
            }
        }
    }
}

impl eframe::App for App {
    /// The focused tier reads everything typed into the window. Tab, Space and Enter would also
    /// move keyboard focus onto the app's buttons and press them, including the unlock, which
    /// can't be cancelled. egui acts on Tab before `ui` runs, so the keys are taken out here.
    ///
    /// While the window is minimised/occluded, eframe runs no egui pass at all: it hands this
    /// hook the same not-yet-delivered `RawInput` again on every pass (growing it with any new
    /// events in the meantime) instead of a fresh one, so `raw_input.events` isn't reliably just
    /// "what's new" (see `update_logic_only`/`prepare_raw_input` in eframe's
    /// `native/epi_integration.rs`). Only the events beyond what `typed_seen` already accounts
    /// for are genuinely new; a shorter list than `typed_seen` means a real pass finally
    /// consumed the backlog, so everything here is new again.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let new_from = if raw_input.events.starts_with(&self.typed_seen) { self.typed_seen.len() } else { 0 };
        self.typed.extend(raw_input.events[new_from..].iter().cloned());
        raw_input.events.retain(|e| !focused::operates_widgets(e));
        self.typed_seen.clone_from(&raw_input.events);
    }

    /// Called once right after `raw_input_hook`, on both the visible path (before `ui`) and the
    /// hidden one (instead of it), so this is where `typed` must be drained: `ui` only runs
    /// while the window is visible, which is exactly when the duplication in `raw_input_hook`'s
    /// doc comment would otherwise pile up unseen.
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        for input in focused::translate(&std::mem::take(&mut self.typed)) {
            match input {
                FocusedInput::Key(key) if !self.state.evdev_active => self.state.os_key(&key, now),
                FocusedInput::Key(_) => {} // evdev already reported it
                FocusedInput::Text(text) => {
                    self.state.on_text(&text);
                    if self.tutor.is_active() {
                        // Text can carry several characters at once (IME, dead keys). Space
                        // arrives here too, which is why `status::visible` has to special-case it.
                        for c in text.chars() {
                            self.tutor_input(tutor::Input::Char(c), now);
                        }
                    }
                }
                FocusedInput::Command(key) if self.tutor.is_active() => {
                    let input = match key {
                        egui::Key::Backspace => tutor::Input::Backspace,
                        egui::Key::Enter => tutor::Input::Enter,
                        _ => tutor::Input::Escape,
                    };
                    self.tutor_input(input, now);
                }
                FocusedInput::Command(_) => {}
                FocusedInput::FocusLost => {
                    self.state.release_focused_keys();
                    self.tutor_input(tutor::Input::FocusLost, now);
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        self.drain_device_events(now);
        while let Ok(msg) = self.input_rx.try_recv() {
            match msg {
                InputMsg::Key(key) => self.state.os_key(&key, now),
                InputMsg::EvdevGone(_) => {
                    self.evdev = EvdevStatus::NotFound;
                    self.state.evdev_active = false;
                }
            }
        }
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::R)) {
            self.reload();
        }
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::T)) {
            self.tutor.toggle();
        }

        egui::Panel::bottom("status").show(ui, |ui| status::show(ui, self, now));
        if self.tutor.is_active() {
            egui::Panel::top("tutor").show(ui, |ui| tutor_panel::show(ui, self));
        }
        egui::CentralPanel::default().show(ui, |ui| self.central(ui, now));
        dialogs::show(ui.ctx(), self);

        // Tap-hold keys become holds after the tapping term with no new input, so keep repainting.
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceInfo;
    use crate::hid::fake::{SMALL_DEFINITION, SMALL_KEYMAP};
    use crate::keymap::Keymap;
    use crate::layout::Layout;

    /// An `App` wired to channels instead of a device worker.
    fn app(config_error: Option<&str>) -> (App, Sender<DeviceEvent>, Receiver<DeviceCommand>) {
        let (events_tx, device_rx) = mpsc::channel();
        let (device_tx, commands_rx) = mpsc::channel();
        let app =
            App::with_device(&Config::default(), config_error.map(String::from), egui::Context::default(), device_tx, device_rx);
        (app, events_tx, commands_rx)
    }

    /// `Connected` for the 2x3 test keyboard. Its `usb_dir` matches no real device, so no evdev reader starts.
    fn connected() -> DeviceEvent {
        let layout = Layout::from_definition(&serde_json::from_str(SMALL_DEFINITION).unwrap()).unwrap();
        let buf: Vec<u8> = SMALL_KEYMAP.iter().flat_map(|c| c.to_be_bytes()).collect();
        let info = DeviceInfo {
            dev_node: "/dev/hidraw99".into(),
            usb_dir: "/nonexistent/usb".into(),
            product: "Sim58".into(),
            via_protocol: 12,
            vial_protocol: 6,
        };
        DeviceEvent::Connected { info, layout, keymap: Keymap::from_buffer(2, 2, 3, &buf).unwrap() }
    }

    #[test]
    fn config_error_outlives_device_events() {
        let (mut app, _events, _commands) = app(Some("Config ignored, using defaults: bad"));
        app.on_device_event(DeviceEvent::Error("boom".into()), Instant::now());
        app.on_device_event(connected(), Instant::now());
        assert_eq!(app.error, None, "Connected clears device errors");
        assert_eq!(app.config_error.as_deref(), Some("Config ignored, using defaults: bad"));
    }

    #[test]
    fn tab_space_and_enter_reach_the_tracker_but_not_the_buttons() {
        let (mut app, _events, _commands) = app(None);
        let ctx = egui::Context::default();
        let key = |k: egui::Key| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (mut clicks, mut typed) = (0, Vec::new());
        // Unfiltered, Tab focuses the button and Space and Enter each press it.
        let frames = [
            vec![],
            vec![key(egui::Key::Tab)],
            vec![],
            vec![key(egui::Key::Space)],
            vec![],
            vec![key(egui::Key::Enter)],
            vec![],
        ];
        for events in frames {
            let mut raw = egui::RawInput { events, ..Default::default() };
            eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
            typed.append(&mut app.typed);
            ctx.run_ui(raw, |ui| {
                if ui.button("Unlock for layer tracking").clicked() {
                    clicks += 1;
                }
            })
            .textures_delta
            .clear();
        }
        assert_eq!(clicks, 0);
        assert_eq!(typed, vec![key(egui::Key::Tab), key(egui::Key::Space), key(egui::Key::Enter)]);
    }

    /// While the window is minimised/occluded, eframe runs no egui pass at all: it re-hooks the
    /// same not-yet-delivered `RawInput` on every pass (growing it with any new events), instead
    /// of handing the hook a fresh one each time. `typed` must still see each event once.
    #[test]
    fn hidden_passes_do_not_duplicate_typed_events() {
        let (mut app, _events, _commands) = app(None);
        let ctx = egui::Context::default();
        let a = egui::Event::Text("a".into());
        let mut raw = egui::RawInput { events: vec![a.clone()], ..Default::default() };

        // First hidden pass.
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
        assert_eq!(app.typed, vec![a.clone()]);

        // Second hidden pass: nothing consumed `raw`, so eframe hooks the same events again.
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
        assert_eq!(app.typed, vec![a.clone()], "the event must not be recorded twice");

        // A genuinely new event arriving on a later hidden pass is still captured, once.
        let b = egui::Event::Text("b".into());
        raw.events.push(b.clone());
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
        assert_eq!(app.typed, vec![a, b]);
    }

    #[test]
    fn paused_hides_an_unlock_in_progress() {
        let (mut app, _events, _commands) = app(None);
        let now = Instant::now();
        app.on_device_event(DeviceEvent::Unlocking { counter: 50, unlock_keys: vec![(1, 0), (1, 2)] }, now);
        assert!(!app.unlock_highlight().is_empty());
        app.on_device_event(DeviceEvent::Paused { holders: vec!["vial (7)".into()] }, now);
        assert_eq!(app.unlock, Unlock::Unknown, "no unlock window over \"Paused\"");
        assert!(app.unlock_highlight().is_empty());
    }

    #[test]
    fn resume_clears_paused_while_an_unlock_is_in_progress() {
        let (mut app, _events, _commands) = app(None);
        let now = Instant::now();
        app.on_device_event(DeviceEvent::Unlocking { counter: 50, unlock_keys: vec![(1, 0), (1, 2)] }, now);
        app.on_device_event(DeviceEvent::Paused { holders: vec!["vial (7)".into()] }, now);
        app.on_device_event(DeviceEvent::Resumed, now);
        app.on_device_event(DeviceEvent::Unlocking { counter: 50, unlock_keys: vec![(1, 0), (1, 2)] }, now);
        assert_eq!(app.connection, Connection::Connected, "no \"Paused\" once the other program has let go (#9)");
        assert!(matches!(app.unlock, Unlock::InProgress { .. }));
    }

    #[test]
    fn losing_the_session_forgets_the_unlock_and_waiting_clears_the_error() {
        let (mut app, _events, _commands) = app(None);
        let now = Instant::now();
        app.on_device_event(DeviceEvent::Unlocking { counter: 40, unlock_keys: vec![] }, now);
        app.on_device_event(DeviceEvent::Error("keyboard sent an unexpected reply: x".into()), now);
        assert_eq!(app.unlock, Unlock::Unknown);
        assert!(app.error.is_some());
        app.on_device_event(DeviceEvent::Waiting, now);
        assert_eq!(app.error, None, "an error about a keyboard that's gone is stale");
    }

    #[test]
    fn a_stopped_device_worker_is_reported() {
        let (mut app, events, _commands) = app(None);
        app.on_device_event(connected(), Instant::now());
        drop(events); // the worker thread ended, or never started
        app.drain_device_events(Instant::now());
        assert_eq!(app.error.as_deref(), Some(DEVICE_WORKER_STOPPED));
        assert!(app.state.layout.is_none(), "the picture can no longer update, so it goes");
    }

    #[test]
    fn commands_to_a_stopped_device_worker_are_reported() {
        let (mut app, _events, commands) = app(None);
        drop(commands);
        app.start_unlock();
        assert_eq!(app.error.as_deref(), Some(DEVICE_WORKER_STOPPED));
    }

    /// The tutor is unavailable until the keyboard's layout has been checked against the finger
    /// map, and goes away again when the keyboard does.
    #[test]
    fn the_tutor_follows_the_keyboard() {
        let (mut app, _events, _commands) = app(None);
        assert_eq!(app.tutor.available(), &crate::tutor::Availability::NoKeyboard);
        app.on_device_event(connected(), Instant::now());
        // The 2x3 test keyboard is not a Lily58, so the finger map rejects it by design.
        assert!(matches!(app.tutor.available(), crate::tutor::Availability::LayoutMismatch(_)));
        app.tutor.toggle();
        assert!(!app.tutor.is_active(), "it can't be opened against a layout it doesn't know");
        app.on_device_event(DeviceEvent::Disconnected, Instant::now());
        assert_eq!(app.tutor.available(), &crate::tutor::Availability::NoKeyboard);
    }
}
