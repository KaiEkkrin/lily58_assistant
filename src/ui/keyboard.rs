//! Draws the keyboard from the layout the keyboard reported.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Shape, Stroke, Vec2};

use crate::hostlayout::HostLayout;
use crate::keycodes::{self, Action, KC_NO};
use crate::layout::KeyGeom;
use crate::state::AppState;
use crate::tutor::fingers::{self, Finger, Hand};
use crate::tutor::hint::KeyPath;

const HELD: Color32 = Color32::from_rgb(90, 170, 255);
const UNLOCK: Color32 = Color32::from_rgb(255, 170, 60);
/// The key to press next, and the keys to hold to reach it.
const HINT: Color32 = Color32::from_rgb(120, 200, 120);
const HINT_HOLD: Color32 = Color32::from_rgb(170, 215, 170);
/// Gap around each key, in key units.
const GAP: f32 = 0.05;
/// Thicker than the plain hairline, so a finger group reads as a group.
const FINGER_STROKE: f32 = 2.0;
/// Key fill alpha in compact mode, so what's underneath shows faintly through.
const TRANSLUCENT_ALPHA: f32 = 0.9;
/// Space `Fit::Fill` leaves around the keys, in total per axis. Points.
const FILL_PADDING: f32 = 24.0;
/// Segments per rounded key corner.
const ARC_STEPS: usize = 4;

/// How big to draw the keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// As large as fits, centred.
    Fill,
    /// This key size in points, anchored `margin` points from the top-left.
    Fixed { unit: f32, margin: f32 },
}

/// What to highlight on the picture besides the keys being pressed.
#[derive(Default)]
pub struct View<'a> {
    pub unlock_keys: &'a [(u8, u8)],
    /// Outline each key in its finger's colour.
    pub fingers: bool,
    pub hint: Option<&'a KeyPath>,
    /// Draw key fills slightly see-through (compact mode).
    pub translucent: bool,
}

/// Ten colours: four fingers and a thumb on each hand. A mirrored five would be easier on the
/// eye but wouldn't say which hand a key belongs to, and on a split board that's half the
/// information. The mapping is a fixed table, not derived from an index, so adjusting one colour
/// after a theme check doesn't shuffle the others.
pub fn finger_colour(hand: Hand, finger: Finger) -> Color32 {
    match (hand, finger) {
        (Hand::Left, Finger::Pinky) => Color32::from_rgb(224, 108, 117),
        (Hand::Left, Finger::Ring) => Color32::from_rgb(224, 154, 76),
        (Hand::Left, Finger::Middle) => Color32::from_rgb(206, 184, 70),
        (Hand::Left, Finger::Index) => Color32::from_rgb(126, 176, 105),
        (Hand::Left, Finger::Thumb) => Color32::from_rgb(176, 124, 198),
        (Hand::Right, Finger::Pinky) => Color32::from_rgb(86, 182, 194),
        (Hand::Right, Finger::Ring) => Color32::from_rgb(97, 175, 239),
        (Hand::Right, Finger::Middle) => Color32::from_rgb(140, 140, 224),
        (Hand::Right, Finger::Index) => Color32::from_rgb(224, 135, 192),
        (Hand::Right, Finger::Thumb) => Color32::from_rgb(190, 145, 110),
    }
}

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

/// Draws the keyboard and returns the key size used, in points.
pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, view: View<'_>, fit: Fit) -> Option<f32> {
    let (Some(layout), Some(keymap)) = (&state.layout, &state.keymap) else { return None };
    let mask = state.layer_mask(now);
    let active = state.active_layer(now);
    let held = state.held_positions();

    let (min_x, min_y, max_x, max_y) = layout.bounds();
    let (span_x, span_y) = ((max_x - min_x).max(1.0), (max_y - min_y).max(1.0));
    let avail = ui.available_size();
    let (response, painter) = ui.allocate_painter(avail, Sense::hover());
    let (unit, origin) = match fit {
        Fit::Fill => {
            let unit = ((avail.x - FILL_PADDING) / span_x).min((avail.y - FILL_PADDING) / span_y).max(8.0);
            (unit, response.rect.center() - Vec2::new(span_x, span_y) * unit / 2.0)
        }
        Fit::Fixed { unit, margin } => (unit, response.rect.min + Vec2::splat(margin)),
    };
    let to_screen = |(x, y): (f32, f32)| Pos2::new(origin.x + (x - min_x) * unit, origin.y + (y - min_y) * unit);
    let visuals = ui.visuals().clone();
    let hint_key = view.hint.map(|h| h.key);
    let hint_hold: &[(u8, u8)] = view.hint.map_or(&[], |h| &h.hold);

    for key in &layout.keys {
        let (layer, code) = keymap.resolve(mask, key.row, key.col);
        let pos = (key.row, key.col);
        let (is_held, is_unlock) = (held.contains(&pos), view.unlock_keys.contains(&pos));
        let is_hint = hint_key == Some(pos);
        let is_hold = hint_hold.contains(&pos);
        // A hinted key turns blue the moment it's actually pressed, so the hint and the press
        // feedback compose instead of fighting over the same pixels.
        let fill = if is_held {
            HELD
        } else if is_unlock {
            UNLOCK
        } else if is_hint {
            HINT
        } else if is_hold {
            HINT_HOLD
        } else {
            visuals.widgets.inactive.bg_fill
        };
        let fill = if view.translucent { fill.gamma_multiply(TRANSLUCENT_ALPHA) } else { fill };
        let text_color = if is_held || is_unlock || is_hint || is_hold {
            Color32::BLACK
        } else if layer < active {
            visuals.weak_text_color() // transparent key: showing a lower layer
        } else {
            visuals.text_color()
        };
        let stroke = match view.fingers.then(|| fingers::spot(key.row, key.col)).flatten() {
            Some(spot) => Stroke::new(FINGER_STROKE, finger_colour(spot.hand, spot.finger)),
            None => Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        };
        let inner = KeyGeom { x: key.x + GAP, y: key.y + GAP, w: key.w - 2.0 * GAP, h: key.h - 2.0 * GAP, ..key.clone() };
        let points = rounded(inner.corners().map(to_screen), corner_radius(unit));
        painter.add(Shape::convex_polygon(points, fill, stroke));

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
    Some(unit)
}

/// How round a key's corners are, in points, for keys `unit` points wide. Slightly round, not
/// a pill. Also used for the compact labels, so they match the keys.
pub fn corner_radius(unit: f32) -> f32 {
    (unit * 0.08).clamp(2.0, 8.0)
}

/// A convex quad's outline with each corner replaced by a curve `radius` points in from it along
/// both edges (capped at half the shorter edge). A curve rather than a circular arc so it also
/// suits rotated and non-rectangular keys; it stays inside the quad, so the result is convex.
fn rounded(corners: [Pos2; 4], radius: f32) -> Vec<Pos2> {
    let mut points = Vec::with_capacity(4 * (ARC_STEPS + 1));
    for i in 0..4 {
        let (prev, c, next) = (corners[(i + 3) % 4], corners[i], corners[(i + 1) % 4]);
        let r = radius.min(c.distance(prev) / 2.0).min(c.distance(next) / 2.0);
        let towards = |p: Pos2| c + (p - c).normalized() * r;
        let (a, b) = (towards(prev), towards(next));
        // Quadratic Bézier from `a` to `b` with the corner as its control point.
        points.extend((0..=ARC_STEPS).map(|s| {
            let t = s as f32 / ARC_STEPS as f32;
            let u = 1.0 - t;
            Pos2::new(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y)
        }));
    }
    points
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

    /// Ten colours, one per finger per hand: a mirrored five would be easier on the eye but
    /// wouldn't say which hand a key belongs to, and on a split board that's half the point.
    #[test]
    fn every_finger_of_every_hand_has_its_own_colour() {
        use crate::tutor::fingers::{Finger, Hand};
        let all: Vec<Color32> = [Hand::Left, Hand::Right]
            .into_iter()
            .flat_map(|hand| {
                [Finger::Pinky, Finger::Ring, Finger::Middle, Finger::Index, Finger::Thumb]
                    .into_iter()
                    .map(move |finger| finger_colour(hand, finger))
            })
            .collect();
        assert_eq!(all.len(), 10);
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two fingers share a colour");
            }
        }
    }

    fn quad(w: f32, h: f32) -> [Pos2; 4] {
        [Pos2::new(0.0, 0.0), Pos2::new(w, 0.0), Pos2::new(w, h), Pos2::new(0.0, h)]
    }

    #[test]
    fn rounded_keys_lose_their_sharp_corners_but_stay_inside_the_key() {
        let corners = quad(40.0, 30.0);
        let points = rounded(corners, 4.0);
        assert_eq!(points.len(), 4 * (ARC_STEPS + 1));
        for p in &points {
            assert!((0.0..=40.0).contains(&p.x) && (0.0..=30.0).contains(&p.y), "{p:?} outside the key");
        }
        for c in corners {
            assert!(points.iter().all(|p| p.distance(c) > 1.0), "corner {c:?} is still sharp");
        }
        // Each curve starts and ends 4 points in from its corner, along the edges.
        assert_eq!(points[0], Pos2::new(0.0, 4.0));
        assert_eq!(points[ARC_STEPS], Pos2::new(4.0, 0.0));
    }

    #[test]
    fn a_radius_too_big_for_the_key_is_capped_at_half_the_shorter_edge() {
        let points = rounded(quad(40.0, 10.0), 100.0);
        assert_eq!(points[0], Pos2::new(0.0, 5.0));
        assert_eq!(points[ARC_STEPS], Pos2::new(5.0, 0.0));
    }

    #[test]
    fn corners_are_slightly_round_at_any_key_size() {
        assert_eq!(corner_radius(10.0), 2.0);
        assert!((corner_radius(55.0) - 4.4).abs() < 1e-4);
        assert_eq!(corner_radius(200.0), 8.0);
    }

    #[test]
    fn a_default_view_highlights_nothing() {
        let view = View::default();
        assert!(view.unlock_keys.is_empty());
        assert!(!view.fingers);
        assert!(view.hint.is_none());
        assert!(!view.translucent);
    }

    fn lily58_state() -> AppState {
        use crate::layers::TriLayer;
        use crate::layout::Layout;
        let layout =
            Layout::from_definition(&serde_json::from_str(include_str!("../../tests/fixtures/lily58-definition.json")).unwrap())
                .unwrap();
        let mut state = AppState::new(HostLayout::Gb, TriLayer::default(), false);
        state.set_keyboard(layout, crate::tutor::fixture::reference_keymap());
        state
    }

    /// Runs one frame on a 960×460 screen and returns what `show` reported and the space it had.
    fn run(state: &AppState, fit: Fit) -> (Option<f32>, Vec2) {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 460.0))),
            ..Default::default()
        };
        let (mut unit, mut avail) = (None, Vec2::ZERO);
        ctx.run_ui(raw, |ui| {
            avail = ui.available_size();
            unit = show(ui, state, Instant::now(), View::default(), fit);
        })
        .textures_delta
        .clear();
        (unit, avail)
    }

    #[test]
    fn fill_reports_the_key_size_it_chose() {
        let (unit, avail) = run(&lily58_state(), Fit::Fill);
        // Lily58 spans 16.5 × 5.75 key units; Fill leaves FILL_PADDING around the keys.
        let expected = ((avail.x - FILL_PADDING) / 16.5).min((avail.y - FILL_PADDING) / 5.75).max(8.0);
        let unit = unit.expect("a keyboard is loaded");
        assert!((unit - expected).abs() < 1e-3, "{unit} vs {expected}");
    }

    #[test]
    fn fixed_uses_the_key_size_it_is_given() {
        let (unit, _) = run(&lily58_state(), Fit::Fixed { unit: 37.0, margin: 8.0 });
        assert_eq!(unit, Some(37.0));
    }

    /// Compact labels are placed assuming the keys start at the ui's top-left plus the margin
    /// (`Fit::Fixed`'s contract), not centred the way `Fit::Fill` draws them. Pin the anchor by
    /// checking where the key shapes actually land.
    #[test]
    fn fixed_anchors_keys_at_the_top_left_plus_margin() {
        let state = lily58_state();
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 460.0))),
            ..Default::default()
        };
        let (unit, margin) = (37.0, 8.0);
        let mut ui_min = Pos2::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            ui_min = ui.cursor().min;
            show(ui, &state, Instant::now(), View::default(), Fit::Fixed { unit, margin });
        });

        let mut min = Pos2::new(f32::INFINITY, f32::INFINITY);
        for clipped in &output.shapes {
            if let Shape::Path(path) = &clipped.shape {
                for p in &path.points {
                    min.x = min.x.min(p.x);
                    min.y = min.y.min(p.y);
                }
            }
        }
        output.textures_delta.clear();

        // Every key is inset by GAP before being placed, so the closest corner lands GAP*unit
        // beyond the margin, not at a centred origin.
        let expected = ui_min + Vec2::splat(margin + GAP * unit);
        assert!((min - expected).length() < 1.0, "{min:?} vs {expected:?} (top-left anchored, not centred)");
    }

    #[test]
    fn no_keyboard_no_key_size() {
        let state = AppState::new(HostLayout::Gb, crate::layers::TriLayer::default(), false);
        assert_eq!(run(&state, Fit::Fill).0, None);
    }
}
