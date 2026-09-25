//! Compact mode: while unfocused, the window drops to just the keys and two labels on a
//! transparent, click-through background, and comes back in full on focus.
//!
//! One window switches modes rather than a second overlay window: a Wayland app can't restore a
//! minimized window, and a new window would inherit neither the user's keep-above setting nor its
//! position. The app can't move its window on Wayland either, so the keys shift a little on each
//! change (the title bar goes, and the keys go from centred to top-left anchored).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2, ViewportCommand, pos2};

use super::{App, keyboard, status};
use crate::layout::Layout;
use crate::state::AppState;

/// Space around the keys in compact mode, so their outlines aren't clipped. Points.
pub const MARGIN: f32 = 8.0;
/// The viewport's minimum inner size in full mode. Below this, some X11 WMs clamp the window
/// instead of shrinking it (leaving a transparent strip) and some Wayland compositors push back;
/// compact mode lifts the floor to `Vec2::ZERO` while it's in effect.
pub const FULL_MIN_SIZE: Vec2 = Vec2::new(480.0, 240.0);
/// Label box, in key units.
const LABEL_W: f32 = 2.4;
const LABEL_H: f32 = 0.6;
/// Height of the extra row the labels move to when a layout has no gap under its corners.
const LABEL_ROW: f32 = 0.8;
/// How long the window must stay unfocused, at one size, before it goes compact. On GNOME
/// Wayland, dragging a client-drawn border to resize takes keyboard focus for the drag; going
/// compact then would drop the border and the drag with it. The size changing restarts the wait,
/// and focus comes back when the drag ends.
pub const GRACE: Duration = Duration::from_secs(1);

/// What the window looks like this frame, gathered from egui by the caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowFacts {
    pub focused: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    pub content_size: Vec2,
}

/// Where the two labels go, in key units (the layout's own coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slots {
    pub left: Rect,
    pub right: Rect,
    /// The layout has no gap under its bottom corners, so the labels sit in a row below the keys.
    pub extra_row: bool,
}

pub fn should_be_compact(enabled: bool, focused: bool, has_layout: bool, maximized: bool, fullscreen: bool) -> bool {
    enabled && !focused && has_layout && !maximized && !fullscreen
}

pub fn label_slots(layout: &Layout) -> Slots {
    let (min_x, _, max_x, max_y) = layout.bounds();
    let at = |bottom: f32| Slots {
        left: Rect::from_min_max(pos2(min_x, bottom - LABEL_H), pos2(min_x + LABEL_W, bottom)),
        right: Rect::from_min_max(pos2(max_x - LABEL_W, bottom - LABEL_H), pos2(max_x, bottom)),
        extra_row: false,
    };
    let corners = at(max_y);
    if !covers_a_key(layout, corners.left) && !covers_a_key(layout, corners.right) {
        return corners;
    }
    Slots { extra_row: true, ..at(max_y + LABEL_ROW) }
}

/// Whether `r` overlaps any key's bounding box by more than an edge.
fn covers_a_key(layout: &Layout, r: Rect) -> bool {
    layout.keys.iter().any(|k| {
        let c = k.corners();
        let key = Rect::from_points(&c.map(|(x, y)| pos2(x, y)));
        key.intersects(r) && key.intersect(r).area() > 0.0
    })
}

pub fn window_size(layout: &Layout, unit: f32) -> Vec2 {
    let (min_x, min_y, max_x, max_y) = layout.bounds();
    let row = if label_slots(layout).extra_row { LABEL_ROW } else { 0.0 };
    Vec2::new((max_x - min_x) * unit, (max_y - min_y + row) * unit) + Vec2::splat(2.0 * MARGIN)
}

pub fn enter_commands(size: Vec2) -> Vec<ViewportCommand> {
    vec![
        ViewportCommand::MinInnerSize(Vec2::ZERO),
        ViewportCommand::Decorations(false),
        ViewportCommand::MousePassthrough(true),
        ViewportCommand::InnerSize(size),
    ]
}

pub fn leave_commands(size: Vec2) -> Vec<ViewportCommand> {
    vec![
        ViewportCommand::MinInnerSize(FULL_MIN_SIZE),
        ViewportCommand::Decorations(true),
        ViewportCommand::MousePassthrough(false),
        ViewportCommand::InnerSize(size),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Frozen {
    unit: f32,
    /// Content size to go back to.
    restore: Vec2,
}

#[derive(Debug, Default)]
pub struct Compact {
    /// The status-bar checkbox.
    pub enabled: bool,
    /// Key size and content size on the last full frame that wasn't still catching up to a
    /// resize back from compact. The restore size on entering compact comes from here, not from
    /// this frame's `WindowFacts::content_size`, which can still read as the compact size if
    /// focus is lost again before the resize back has landed.
    full: Option<(f32, Vec2)>,
    /// The size last asked for on entering compact; full frames still at this size are the
    /// resize back not having landed yet, and mustn't be recorded as the full key size.
    compact_size: Option<Vec2>,
    /// Set exactly while compact.
    frozen: Option<Frozen>,
    /// While compact is wanted but waiting out `GRACE`: when the wait started, and the content
    /// size then.
    waiting: Option<(Instant, Vec2)>,
}

impl Compact {
    /// Decides this frame's mode and returns the commands for a change, if any. The size to
    /// restore comes from the last `record_full_unit` call, read here, never from a resize that
    /// may still be in flight. Entering waits out `GRACE`; leaving is immediate.
    pub fn frame(&mut self, facts: WindowFacts, layout: Option<&Layout>, now: Instant) -> Vec<ViewportCommand> {
        let want = should_be_compact(self.enabled, facts.focused, layout.is_some(), facts.maximized, facts.fullscreen);
        if layout.is_none() {
            // A replug must not enter compact with a stale size from before the keyboard left.
            self.full = None;
        }
        if !want || self.frozen.is_some() {
            self.waiting = None;
        } else {
            let start = match self.waiting {
                Some((start, size)) if (size - facts.content_size).length() < 1.0 => start,
                _ => now, // just lost focus, or still being resized
            };
            self.waiting = Some((start, facts.content_size));
            if now.duration_since(start) < GRACE {
                return Vec::new();
            }
        }
        match (self.frozen, want, layout, self.full) {
            (None, true, Some(layout), Some((unit, full_size))) => {
                let size = window_size(layout, unit);
                self.frozen = Some(Frozen { unit, restore: full_size });
                self.compact_size = Some(size);
                enter_commands(size)
            }
            (Some(frozen), false, ..) => {
                self.frozen = None;
                leave_commands(frozen.restore)
            }
            _ => Vec::new(),
        }
    }

    /// Called on each full frame with the key size `keyboard::show` used and the window's
    /// content size.
    pub fn record_full_unit(&mut self, unit: f32, content_size: Vec2) {
        let still_small = self.compact_size.is_some_and(|s| (s - content_size).length() < 1.0);
        if !still_small {
            self.full = Some((unit, content_size));
        }
    }

    /// While waiting to go compact, how long until `frame` should be called again.
    pub fn wake_in(&self, now: Instant) -> Option<Duration> {
        self.waiting.map(|(start, _)| GRACE.saturating_sub(now.duration_since(start)))
    }

    pub fn frozen_unit(&self) -> Option<f32> {
        self.frozen.map(|f| f.unit)
    }
}

/// Background behind each label, so it reads over whatever is underneath.
const LABEL_BG: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 215);

pub fn layer_text(state: &AppState, now: Instant) -> String {
    if state.matrix_active { format!("Layer {}", state.active_layer(now)) } else { "Layer ?".into() }
}

/// Draws the compact view: keys at the frozen size, and the last-key and layer labels.
pub fn show(ui: &mut egui::Ui, app: &App, now: Instant, unit: f32) {
    let Some(layout) = &app.state.layout else { return };
    let top_left = ui.max_rect().min + Vec2::splat(MARGIN);
    let _ = keyboard::show(ui, &app.state, now, keyboard::View {
        unlock_keys: app.unlock_highlight(),
        fingers: app.finger_colours_shown(),
        hint: app.tutor.hint(),
        translucent: true,
    }, keyboard::Fit::Fixed { unit, margin: MARGIN });

    let (min_x, min_y, ..) = layout.bounds();
    let to_screen = |p: Pos2| top_left + Vec2::new((p.x - min_x) * unit, (p.y - min_y) * unit);
    let slots = label_slots(layout);
    let last = app.state.last.as_ref().map(status::last_key_text).unwrap_or_default();
    let painter = ui.painter();
    for (slot, text, align) in [(slots.left, last, Align2::LEFT_CENTER), (slots.right, layer_text(&app.state, now), Align2::RIGHT_CENTER)] {
        if text.is_empty() {
            continue;
        }
        let rect = Rect::from_min_max(to_screen(slot.min), to_screen(slot.max));
        // The key labels' size, shrunk if the text is too wide for the box.
        let size = (unit * 0.3).clamp(9.0, 20.0);
        let galley = painter.layout_no_wrap(text.clone(), FontId::monospace(size), Color32::WHITE);
        let pad = 6.0;
        let fit = ((rect.width() - 2.0 * pad) / galley.size().x).min(1.0);
        let galley = painter.layout_no_wrap(text, FontId::monospace(size * fit), Color32::WHITE);
        let bg_w = galley.size().x + 2.0 * pad;
        let bg = if align == Align2::LEFT_CENTER {
            Rect::from_min_max(rect.min, pos2(rect.min.x + bg_w, rect.max.y))
        } else {
            Rect::from_min_max(pos2(rect.max.x - bg_w, rect.min.y), rect.max)
        };
        painter.rect_filled(bg, CornerRadius::same(keyboard::corner_radius(unit).round() as u8), LABEL_BG);
        painter.galley(pos2(bg.min.x + pad, bg.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lily58() -> Layout {
        Layout::from_definition(&serde_json::from_str(include_str!("../../tests/fixtures/lily58-definition.json")).unwrap())
            .unwrap()
    }

    /// Two rows of three keys: a label in either bottom corner lands on a key.
    fn no_gap() -> Layout {
        let def = json!({
            "name": "Block", "matrix": { "rows": 2, "cols": 3 },
            "layouts": { "keymap": [["0,0", "0,1", "0,2"], ["1,0", "1,1", "1,2"]] }
        });
        Layout::from_definition(&def).unwrap()
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-3
    }

    fn facts(focused: bool) -> WindowFacts {
        WindowFacts { focused, maximized: false, fullscreen: false, content_size: Vec2::new(960.0, 460.0) }
    }

    fn overlaps_a_key(layout: &Layout, r: Rect) -> bool {
        layout.keys.iter().any(|k| {
            let c = k.corners();
            let (xs, ys) = (c.map(|p| p.0), c.map(|p| p.1));
            let key = Rect::from_min_max(
                pos2(xs.iter().copied().fold(f32::MAX, f32::min), ys.iter().copied().fold(f32::MAX, f32::min)),
                pos2(xs.iter().copied().fold(f32::MIN, f32::max), ys.iter().copied().fold(f32::MIN, f32::max)),
            );
            key.intersects(r) && key.intersect(r).area() > 0.0
        })
    }

    #[test]
    fn compact_only_when_every_condition_holds() {
        assert!(should_be_compact(true, false, true, false, false));
        assert!(!should_be_compact(false, false, true, false, false), "checkbox off");
        assert!(!should_be_compact(true, true, true, false, false), "focused");
        assert!(!should_be_compact(true, false, false, false, false), "no keyboard picture");
        assert!(!should_be_compact(true, false, true, true, false), "maximized: the shrink would be ignored");
        assert!(!should_be_compact(true, false, true, false, true), "fullscreen");
    }

    #[test]
    fn lily58_labels_fit_under_the_outer_columns() {
        let layout = lily58();
        let slots = label_slots(&layout);
        assert!(!slots.extra_row);
        assert!(!overlaps_a_key(&layout, slots.left), "left label {:?} covers a key", slots.left);
        assert!(!overlaps_a_key(&layout, slots.right), "right label {:?} covers a key", slots.right);
        assert!(close(slots.left.left_bottom().to_vec2(), Vec2::new(0.5, 5.75)), "{:?}", slots.left);
        assert!(close(slots.right.right_bottom().to_vec2(), Vec2::new(17.0, 5.75)), "{:?}", slots.right);
    }

    #[test]
    fn labels_move_below_when_the_corners_are_taken() {
        let layout = no_gap();
        let slots = label_slots(&layout);
        assert!(slots.extra_row);
        assert!(!overlaps_a_key(&layout, slots.left));
        assert!(!overlaps_a_key(&layout, slots.right));
        assert!(slots.left.top() >= 2.0, "below the keys: {:?}", slots.left);
    }

    #[test]
    fn window_size_is_the_keys_at_their_size_plus_the_margin() {
        // Lily58 spans 16.5 × 5.75 key units.
        assert!(close(window_size(&lily58(), 40.0), Vec2::new(16.5 * 40.0 + 16.0, 5.75 * 40.0 + 16.0)));
        // 3 × 2 plus a label row.
        assert!(close(window_size(&no_gap(), 40.0), Vec2::new(3.0 * 40.0 + 16.0, 2.8 * 40.0 + 16.0)));
    }

    #[test]
    fn mode_change_commands() {
        let size = Vec2::new(676.0, 246.0);
        assert_eq!(enter_commands(size), vec![
            ViewportCommand::MinInnerSize(Vec2::ZERO),
            ViewportCommand::Decorations(false),
            ViewportCommand::MousePassthrough(true),
            ViewportCommand::InnerSize(size),
        ]);
        assert_eq!(leave_commands(size), vec![
            ViewportCommand::MinInnerSize(FULL_MIN_SIZE),
            ViewportCommand::Decorations(true),
            ViewportCommand::MousePassthrough(false),
            ViewportCommand::InnerSize(size),
        ]);
    }

    /// Calls `frame` at `t` and again once `GRACE` has passed, returning the second's commands:
    /// staying unfocused long enough to go compact.
    fn settle(c: &mut Compact, facts: WindowFacts, layout: Option<&Layout>, t: Instant) -> Vec<ViewportCommand> {
        c.frame(facts, layout, t);
        c.frame(facts, layout, t + GRACE)
    }

    #[test]
    fn losing_focus_freezes_the_key_size_and_regaining_it_restores_the_window() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        assert_eq!(c.frame(facts(true), Some(&layout), t), vec![]);
        assert_eq!(c.frozen_unit(), None);

        assert_eq!(settle(&mut c, facts(false), Some(&layout), t), enter_commands(window_size(&layout, 40.0)));
        assert_eq!(c.frozen_unit(), Some(40.0));
        assert_eq!(c.frame(facts(false), Some(&layout), t + 2 * GRACE), vec![], "no repeat while staying compact");
        assert_eq!(c.wake_in(t + 2 * GRACE), None, "nothing to wait for while compact");

        assert_eq!(c.frame(facts(true), Some(&layout), t + 2 * GRACE), leave_commands(Vec2::new(960.0, 460.0)));
        assert_eq!(c.frozen_unit(), None);
    }

    #[test]
    fn compact_waits_out_the_grace_period() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));

        assert_eq!(c.frame(facts(false), Some(&layout), t), vec![]);
        assert_eq!(c.wake_in(t), Some(GRACE));
        let almost = t + GRACE - Duration::from_millis(1);
        assert_eq!(c.frame(facts(false), Some(&layout), almost), vec![]);
        assert_eq!(c.wake_in(almost), Some(Duration::from_millis(1)));
        assert_eq!(c.frame(facts(false), Some(&layout), t + GRACE), enter_commands(window_size(&layout, 40.0)));
    }

    #[test]
    fn focus_back_within_the_grace_period_never_goes_compact() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));

        c.frame(facts(false), Some(&layout), t);
        assert_eq!(c.frame(facts(true), Some(&layout), t + GRACE / 2), vec![]);
        assert_eq!(c.wake_in(t + GRACE / 2), None);
        // Lost again: the wait starts over rather than carrying on from the first loss.
        assert_eq!(c.frame(facts(false), Some(&layout), t + GRACE), vec![]);
        assert_eq!(c.frame(facts(false), Some(&layout), t + GRACE + GRACE / 2), vec![]);
        assert_eq!(c.frozen_unit(), None);
    }

    /// GNOME Wayland: dragging the window's border takes focus for the drag. The window must
    /// stay full while the size keeps changing, or the border (and the drag) would vanish.
    #[test]
    fn resizing_while_unfocused_keeps_restarting_the_wait() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));

        let sized = |w: f32| WindowFacts { content_size: Vec2::new(w, 460.0), ..facts(false) };
        let step = GRACE / 2;
        for i in 0..6u32 {
            assert_eq!(c.frame(sized(960.0 + 10.0 * i as f32), Some(&layout), t + step * i), vec![], "drag step {i}");
        }
        assert_eq!(c.frozen_unit(), None);
        // The drag ends and focus comes back.
        assert_eq!(c.frame(WindowFacts { focused: true, ..sized(1010.0) }, Some(&layout), t + step * 6), vec![]);
    }

    #[test]
    fn losing_the_keyboard_while_compact_restores_the_window() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        settle(&mut c, facts(false), Some(&layout), t);
        assert_eq!(c.frame(facts(false), None, t + GRACE), leave_commands(Vec2::new(960.0, 460.0)));
        assert_eq!(c.frozen_unit(), None);
    }

    #[test]
    fn nothing_happens_with_the_checkbox_off_or_before_a_key_size_is_known() {
        let layout = lily58();
        let t = Instant::now();
        let mut off = Compact::default();
        off.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        assert_eq!(settle(&mut off, facts(false), Some(&layout), t), vec![]);
        assert_eq!(off.wake_in(t), None, "no repaints asked for with the checkbox off");

        let mut unknown = Compact { enabled: true, ..Default::default() };
        assert_eq!(settle(&mut unknown, facts(false), Some(&layout), t), vec![], "no full frame drawn yet");
    }

    #[test]
    fn full_frames_still_at_the_compact_size_are_not_recorded() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        let compact = window_size(&layout, 40.0);
        settle(&mut c, facts(false), Some(&layout), t);
        c.frame(facts(true), Some(&layout), t + GRACE);
        // The resize back hasn't landed: keys fitted into the small window come out smaller.
        c.record_full_unit(38.5, compact);
        settle(&mut c, facts(false), Some(&layout), t + GRACE);
        assert_eq!(c.frozen_unit(), Some(40.0), "the key size didn't drift");
    }

    /// If the window leaves compact and loses focus again before the resize back has landed,
    /// `content_size` still reads as the compact size. The restore size must come from the last
    /// `record_full_unit` call, not from that stale `content_size`, or the full window stays
    /// small on every later round trip.
    #[test]
    fn restore_size_survives_a_focus_blip_before_the_resize_lands() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        let compact = window_size(&layout, 40.0);

        assert_eq!(settle(&mut c, facts(false), Some(&layout), t), enter_commands(compact));

        // Focus regained, but the resize back to 960x460 hasn't landed: this frame's
        // content_size still reads as the compact size.
        let blip = WindowFacts { focused: true, maximized: false, fullscreen: false, content_size: compact };
        assert_eq!(c.frame(blip, Some(&layout), t + GRACE), leave_commands(Vec2::new(960.0, 460.0)));

        // Lost again: still no full frame has been drawn, so content_size is still
        // compact-sized here too.
        let still_small = WindowFacts { focused: false, maximized: false, fullscreen: false, content_size: compact };
        assert_eq!(settle(&mut c, still_small, Some(&layout), t + GRACE), enter_commands(compact));

        // Regaining focus for real must restore the original full size, not the compact one.
        assert_eq!(c.frame(facts(true), Some(&layout), t + 2 * GRACE), leave_commands(Vec2::new(960.0, 460.0)));
    }

    /// A replug while unfocused must not enter compact with a stale key/window size from before
    /// the keyboard went away.
    #[test]
    fn full_size_is_cleared_when_the_keyboard_goes_away() {
        let layout = lily58();
        let t = Instant::now();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));

        // Keyboard unplugged.
        assert_eq!(c.frame(facts(true), None, t), vec![]);

        // Replugged and unfocused before any full frame has recorded a fresh size.
        assert_eq!(settle(&mut c, facts(false), Some(&layout), t), vec![], "no enter commands without a fresh record_full_unit");

        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        assert_eq!(c.frame(facts(false), Some(&layout), t + 2 * GRACE), enter_commands(window_size(&layout, 40.0)));
    }

    #[test]
    fn layer_label_says_when_the_layer_is_unknown() {
        use crate::hostlayout::HostLayout;
        use crate::layers::TriLayer;
        use crate::state::AppState;
        use std::time::Instant;
        let mut state = AppState::new(HostLayout::Gb, TriLayer::default(), false);
        assert_eq!(layer_text(&state, Instant::now()), "Layer ?");
        state.set_matrix_active(true);
        assert_eq!(layer_text(&state, Instant::now()), "Layer 0");
    }
}
