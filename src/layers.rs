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
                Action::Momentary(l) | Action::LayerMod { layer: l, .. } | Action::OneShotLayer(l) | Action::TapToggle(l) => mask |= bit(l),
                Action::LayerTap { layer: l, .. } if long => mask |= bit(l),
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
    const TT1: u16 = 0x52C1;
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

    #[test]
    fn tap_toggle_acts_like_momentary_while_held() {
        let (mut t, now) = tracker(false);
        t.press(0, 0, TT1, now);
        assert_eq!(t.active_layer(now), 1);
        t.release(0, 0, now);
        assert_eq!(t.active_layer(now), 0);

        t.press(0, 0, TT1, now);
        t.release(0, 0, now);
        assert_eq!(t.active_layer(now), 0);
    }
}
