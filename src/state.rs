//! Everything the UI shows, merged from the three input tiers.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use crate::hostlayout::HostLayout;
use crate::input::{OsKey, OsSource};
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
    /// OS keys currently down (by source and `OsKey::name`) and where we placed them.
    os_held: HashMap<(OsSource, String), Option<(u8, u8)>>,
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
        if self.matrix_active != active {
            self.os_held.clear();
        }
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
        let held = (key.source, key.name.clone());
        if !key.pressed {
            self.os_held.remove(&held);
            return;
        }
        if self.matrix_active {
            return; // the matrix already knows exactly which key it was
        }
        let mask = self.layer_mask(now);
        let hit = self.keymap.as_ref().and_then(|km| km.find_position(mask, &key.usages));
        self.os_held.insert(held, hit.map(|h| (h.row, h.col)));
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

    /// The window lost focus (e.g. Alt+Tab). On Wayland it never sees the release of a key held
    /// at that moment, so forget the keys it saw go down, and the shift state if it came from them.
    pub fn release_focused_keys(&mut self) {
        self.os_held.retain(|(source, _), _| *source != OsSource::Focused);
        if !self.evdev_active {
            self.os_shift = false; // the UI feeds focused keys to `os_key` only while evdev is off
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

    #[test]
    fn os_release_during_matrix_tier_is_not_lost() {
        let mut s = state();
        s.os_key(&os("evdev 48", &[0x05], true), Instant::now());
        assert!(!s.held_positions().is_empty());
        s.set_matrix_active(true);
        s.os_key(&os("evdev 48", &[0x05], false), Instant::now());
        s.set_matrix_active(false);
        assert!(s.held_positions().is_empty());
    }

    #[test]
    fn os_shift_still_applies_while_matrix_is_active() {
        let mut s = state();
        s.set_matrix_active(true);
        let now = Instant::now();
        s.os_key(&os("evdev 42", &[0xE1], true), now);
        s.matrix_changed(&[(0, 0)], &[], now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("A"));
    }

    fn focused(name: &str, usages: &[u8], pressed: bool) -> OsKey {
        OsKey { source: OsSource::Focused, usages: usages.to_vec(), pressed, name: name.into() }
    }

    #[test]
    fn focus_loss_forgets_keys_held_in_the_window() {
        let mut s = state();
        let now = Instant::now();
        s.os_key(&focused("ShiftLeft", &[0xE1], true), now); // LSFT is at (1,2)
        s.os_key(&focused("B", &[0x05], true), now);
        assert_eq!(s.held_positions(), BTreeSet::from([(0, 1), (1, 2)]));
        s.release_focused_keys(); // Alt+Tab: the releases never arrive
        assert!(s.held_positions().is_empty());
        s.os_key(&focused("Num3", &[0x20], true), now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("3"), "shift is no longer held");
    }

    #[test]
    fn focus_loss_keeps_keys_seen_by_evdev() {
        let mut s = state();
        s.evdev_active = true;
        let now = Instant::now();
        s.os_key(&os("evdev 42", &[0xE1], true), now);
        s.release_focused_keys();
        assert_eq!(s.held_positions(), BTreeSet::from([(1, 2)]));
        s.os_key(&os("evdev 4", &[0x20], true), now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("£"), "evdev still has shift down");
    }
}
