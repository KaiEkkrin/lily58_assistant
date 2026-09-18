//! Focused tier: key and text events egui delivers while our window has focus.

use eframe::egui::{Event, Key, Modifiers};

use super::{OsKey, OsSource};
use crate::hidmap::egui_key_to_hid;

pub enum FocusedInput {
    Key(OsKey),
    Text(String),
    /// A bare Backspace, Enter or Escape: the typing tutor's controls. Modifier combinations are
    /// deliberately excluded, so Ctrl+R and Ctrl+T pass through untouched.
    Command(Key),
    /// The window lost keyboard focus, so releases of keys held now won't arrive.
    FocusLost,
}

pub fn translate(events: &[Event]) -> Vec<FocusedInput> {
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::Key { key, physical_key, pressed, repeat: false, modifiers } => {
                let k: Key = physical_key.unwrap_or(*key);
                out.push(FocusedInput::Key(OsKey {
                    source: OsSource::Focused,
                    usages: egui_key_to_hid(k),
                    pressed: *pressed,
                    name: format!("{k:?}"),
                }));
                if *pressed && *modifiers == Modifiers::NONE && matches!(k, Key::Backspace | Key::Enter | Key::Escape) {
                    out.push(FocusedInput::Command(k));
                }
            }
            Event::Text(t) => out.push(FocusedInput::Text(t.clone())),
            Event::WindowFocused(false) => out.push(FocusedInput::FocusLost),
            _ => {}
        }
    }
    out
}

/// Keys egui uses to move keyboard focus between widgets (Tab) and to press the focused one
/// (Space, Enter).
pub fn operates_widgets(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Tab | Key::Space | Key::Enter, .. })
}

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
            _ => panic!("expected a key"),
        }
        assert!(matches!(&out[1], FocusedInput::Text(t) if t == "a"));
    }

    #[test]
    fn focus_loss_is_reported() {
        let out = translate(&[Event::WindowFocused(true), Event::WindowFocused(false)]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], FocusedInput::FocusLost));
    }

    /// The tutor's controls. Modifier combinations are left alone, so Ctrl+R and Ctrl+T still
    /// reach the app while a batch is being typed.
    #[test]
    fn bare_backspace_enter_and_escape_are_reported_as_commands() {
        let out = translate(&[key(Key::Backspace, true, false), key(Key::Enter, true, false), key(Key::Escape, true, false)]);
        let commands: Vec<Key> = out
            .iter()
            .filter_map(|i| match i {
                FocusedInput::Command(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert_eq!(commands, vec![Key::Backspace, Key::Enter, Key::Escape]);
        assert_eq!(out.len(), 6, "each one is still tracked as a key press too");
    }

    #[test]
    fn modified_and_released_keys_are_not_commands() {
        let ctrl = Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        };
        let released = key(Key::Escape, false, false);
        let out = translate(&[ctrl, released]);
        assert!(!out.iter().any(|i| matches!(i, FocusedInput::Command(_))));
    }
}
