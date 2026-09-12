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
                ui.label("Hold the highlighted keys until the bar fills (about 10 seconds).");
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
