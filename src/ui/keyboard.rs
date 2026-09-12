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
