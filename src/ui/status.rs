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
        let blocked = app.tutor.available().reason().or_else(|| {
            matches!(app.unlock, super::Unlock::InProgress { .. }).then(|| "Finish the unlock first.".to_string())
        });
        let label = if app.tutor.is_active() { "Close tutor (Ctrl+T)" } else { "Typing tutor (Ctrl+T)" };
        let button = ui.add_enabled(blocked.is_none(), egui::Button::new(label));
        if button.clicked() {
            app.tutor.toggle();
        }
        if let Some(why) = blocked {
            // The button is only ever disabled when there is a reason, and `on_hover_text`
            // deliberately shows nothing on a disabled widget — so this has to be the
            // disabled-specific variant or the reason never reaches the user.
            button.on_disabled_hover_text(why);
        }
        ui.separator();
        if ui.button("Reload (Ctrl+R)").clicked() {
            app.reload();
        }
    });
    if let Some(err) = &app.error {
        ui.colored_label(Color32::from_rgb(230, 90, 90), err);
    }
    let mut dismissed = false;
    if let Some(err) = &app.config_error {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(Color32::from_rgb(230, 180, 40), err);
            dismissed = ui.small_button("Dismiss").clicked();
        });
    }
    if dismissed {
        app.config_error = None;
    }
}

fn visible(text: &str) -> String {
    if text == " " { "Space".into() } else { text.to_owned() }
}
