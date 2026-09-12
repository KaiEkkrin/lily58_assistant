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
