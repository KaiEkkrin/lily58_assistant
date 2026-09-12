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
