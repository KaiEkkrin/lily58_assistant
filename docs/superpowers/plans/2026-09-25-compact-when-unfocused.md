# Compact When Unfocused Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A session-only "Compact when unfocused" checkbox. With it on, an unfocused window drops to just the keys (at their current size) and two labels, on a transparent, click-through background with no title bar, and returns to the full opaque window on focus.

**Architecture:** One window that switches modes. A new `src/ui/compact.rs` holds a small `Compact` state machine that, each frame, takes the window facts (focused, maximized, fullscreen, content size) and returns the `ViewportCommand`s to send on a mode change, plus the pure geometry (compact window size, label placement in key units) and the compact drawing. `keyboard::show` gains a fixed-key-size mode and reports the key size it used; `App` records it in full mode and freezes it on entering compact.

**Tech Stack:** Rust 2024, eframe/egui 0.36.2 (winit 0.30.13 underneath). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-25-compact-when-unfocused-design.md`

## Global Constraints

- Edition 2024, `rust-version = "1.95"`. Branch: `feat/compact-when-unfocused` (already checked out, already carries the spec commit).
- No new dependencies.
- CI runs `cargo build --locked`, `cargo test --locked`, `cargo clippy --locked --all-targets -- -D warnings`. Every task ends with all three passing.
- Strictly read-only: nothing here touches `src/hid/`.
- Checkbox label, verbatim: **Compact when unfocused**. Off at every start; not saved to config.
- Layer label: `Layer N`, or `Layer ?` when `!state.matrix_active`.
- Compact key fill alpha: 0.9. Compact margin: 8pt.
- Label box: 2.4 × 0.6 key units, anchored to the bottom-left / bottom-right corners of `Layout::bounds()`. If either overlaps a key's bounding box, both move into an extra row 0.8 key units tall below the keys.
- Full mode must look exactly as it does today (opaque).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Background the implementer needs

- egui's `ViewportCommand` derives `PartialEq`, so command lists can be compared in tests.
- `ViewportCommand::InnerSize(Vec2)` is in egui points, the same units as the keyboard's `unit`.
- On Wayland egui reports `viewport().inner_rect`/`outer_rect` as `None`; use `ctx.content_rect().size()` for the window size.
- `viewport().maximized` / `fullscreen` are `Option<bool>`; treat `None` as false.
- A resize lands a frame or two after it's requested. So after leaving compact, the first full frames may still be at the compact size. `Compact::record_full_unit` ignores frames whose content size still equals the compact size it asked for, so the key size doesn't drift smaller on each round trip.
- The Lily58 fixture's key bounds are `(0.5, 0.0, 17.0, 5.75)`. The outer columns end at y = 4.5 and 4.375; the nearest thumb keys start at x = 3.0 (left) and end at x = 14.5 (right). So the left label box `x 0.5–2.9, y 5.15–5.75` and the right one `x 14.6–17.0` are clear.

## File Structure

- **Create `src/ui/compact.rs`:** `WindowFacts`, `Compact` (state machine), `Slots`, `should_be_compact`, `label_slots`, `window_size`, `enter_commands`, `leave_commands`, `layer_text`, and `show` (compact drawing). Its tests.
- **Modify `src/ui/keyboard.rs`:** `Fit` enum, `show` takes `fit` and returns `Option<f32>`, `View::translucent`.
- **Modify `src/ui/status.rs`:** extract `last_key_text`; add the checkbox.
- **Modify `src/ui/mod.rs`:** `mod compact;`, `App::compact` field, per-frame wiring in `ui`, `clear_color`, `.with_transparent(true)` in `run`, integration tests.
- **Modify `README.md`, `docs/manual-test-checklist.md`.**

---

### Task 1: Compact state machine and geometry

**Files:**
- Create: `src/ui/compact.rs`
- Modify: `src/ui/mod.rs` (module declaration only: add `mod compact;` after `mod dialogs;`)

**Interfaces:**
- Consumes: `crate::layout::Layout` (`bounds() -> (f32, f32, f32, f32)`, `keys: Vec<KeyGeom>`, `KeyGeom::corners() -> [(f32, f32); 4]`).
- Produces (all `pub(super)` unless noted, used by Tasks 3–4):
  - `pub const MARGIN: f32 = 8.0;`
  - `pub struct WindowFacts { pub focused: bool, pub maximized: bool, pub fullscreen: bool, pub content_size: egui::Vec2 }` (`Debug, Clone, Copy, PartialEq`)
  - `pub struct Slots { pub left: egui::Rect, pub right: egui::Rect, pub extra_row: bool }` (key units)
  - `pub fn should_be_compact(enabled: bool, focused: bool, has_layout: bool, maximized: bool, fullscreen: bool) -> bool`
  - `pub fn label_slots(layout: &Layout) -> Slots`
  - `pub fn window_size(layout: &Layout, unit: f32) -> egui::Vec2`
  - `pub fn enter_commands(size: egui::Vec2) -> Vec<ViewportCommand>`
  - `pub fn leave_commands(size: egui::Vec2) -> Vec<ViewportCommand>`
  - `pub struct Compact { pub enabled: bool, .. }` (`Debug, Default`) with
    `pub fn frame(&mut self, facts: WindowFacts, layout: Option<&Layout>) -> Vec<ViewportCommand>`,
    `pub fn record_full_unit(&mut self, unit: f32, content_size: egui::Vec2)`,
    `pub fn frozen_unit(&self) -> Option<f32>` (Some exactly while compact).

- [ ] **Step 1: Write the module skeleton and failing tests**

Create `src/ui/compact.rs` with the doc comment, constants and signatures returning `todo!()`, plus the tests below. Add `mod compact;` to `src/ui/mod.rs` after `mod dialogs;`.

```rust
//! Compact mode: while unfocused, the window drops to just the keys and two labels on a
//! transparent, click-through background, and comes back in full on focus.
//!
//! One window switches modes rather than a second overlay window: a Wayland app can't restore a
//! minimized window, and a new window would inherit neither the user's keep-above setting nor its
//! position. The app can't move its window on Wayland either, so the keys shift a little on each
//! change (the title bar goes, and the keys go from centred to top-left anchored).

use eframe::egui::{Rect, Vec2, ViewportCommand, pos2};

use crate::layout::Layout;

/// Space around the keys in compact mode, so their outlines aren't clipped. Points.
pub const MARGIN: f32 = 8.0;
/// Label box, in key units.
const LABEL_W: f32 = 2.4;
const LABEL_H: f32 = 0.6;
/// Height of the extra row the labels move to when a layout has no gap under its corners.
const LABEL_ROW: f32 = 0.8;

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
    todo!()
}

pub fn label_slots(layout: &Layout) -> Slots {
    todo!()
}

pub fn window_size(layout: &Layout, unit: f32) -> Vec2 {
    todo!()
}

pub fn enter_commands(size: Vec2) -> Vec<ViewportCommand> {
    todo!()
}

pub fn leave_commands(size: Vec2) -> Vec<ViewportCommand> {
    todo!()
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
    /// Key size on the last full frame.
    full_unit: Option<f32>,
    /// The size last asked for on entering compact; full frames still at this size are the
    /// resize back not having landed yet, and mustn't be recorded as the full key size.
    compact_size: Option<Vec2>,
    /// Set exactly while compact.
    frozen: Option<Frozen>,
}

impl Compact {
    pub fn frame(&mut self, facts: WindowFacts, layout: Option<&Layout>) -> Vec<ViewportCommand> {
        todo!()
    }

    pub fn record_full_unit(&mut self, unit: f32, content_size: Vec2) {
        todo!()
    }

    pub fn frozen_unit(&self) -> Option<f32> {
        todo!()
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
            ViewportCommand::Decorations(false),
            ViewportCommand::MousePassthrough(true),
            ViewportCommand::InnerSize(size),
        ]);
        assert_eq!(leave_commands(size), vec![
            ViewportCommand::Decorations(true),
            ViewportCommand::MousePassthrough(false),
            ViewportCommand::InnerSize(size),
        ]);
    }

    #[test]
    fn losing_focus_freezes_the_key_size_and_regaining_it_restores_the_window() {
        let layout = lily58();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        assert_eq!(c.frame(facts(true), Some(&layout)), vec![]);
        assert_eq!(c.frozen_unit(), None);

        assert_eq!(c.frame(facts(false), Some(&layout)), enter_commands(window_size(&layout, 40.0)));
        assert_eq!(c.frozen_unit(), Some(40.0));
        assert_eq!(c.frame(facts(false), Some(&layout)), vec![], "no repeat while staying compact");

        assert_eq!(c.frame(facts(true), Some(&layout)), leave_commands(Vec2::new(960.0, 460.0)));
        assert_eq!(c.frozen_unit(), None);
    }

    #[test]
    fn losing_the_keyboard_while_compact_restores_the_window() {
        let layout = lily58();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        c.frame(facts(false), Some(&layout));
        assert_eq!(c.frame(facts(false), None), leave_commands(Vec2::new(960.0, 460.0)));
        assert_eq!(c.frozen_unit(), None);
    }

    #[test]
    fn nothing_happens_with_the_checkbox_off_or_before_a_key_size_is_known() {
        let layout = lily58();
        let mut off = Compact::default();
        off.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        assert_eq!(off.frame(facts(false), Some(&layout)), vec![]);

        let mut unknown = Compact { enabled: true, ..Default::default() };
        assert_eq!(unknown.frame(facts(false), Some(&layout)), vec![], "no full frame drawn yet");
    }

    #[test]
    fn full_frames_still_at_the_compact_size_are_not_recorded() {
        let layout = lily58();
        let mut c = Compact { enabled: true, ..Default::default() };
        c.record_full_unit(40.0, Vec2::new(960.0, 460.0));
        let compact = window_size(&layout, 40.0);
        c.frame(facts(false), Some(&layout));
        c.frame(facts(true), Some(&layout));
        // The resize back hasn't landed: keys fitted into the small window come out smaller.
        c.record_full_unit(38.5, compact);
        c.frame(facts(false), Some(&layout));
        assert_eq!(c.frozen_unit(), Some(40.0), "the key size didn't drift");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked ui::compact`
Expected: FAIL — each test panics with `not yet implemented`.

- [ ] **Step 3: Implement**

Replace the `todo!()` bodies:

```rust
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
    vec![ViewportCommand::Decorations(false), ViewportCommand::MousePassthrough(true), ViewportCommand::InnerSize(size)]
}

pub fn leave_commands(size: Vec2) -> Vec<ViewportCommand> {
    vec![ViewportCommand::Decorations(true), ViewportCommand::MousePassthrough(false), ViewportCommand::InnerSize(size)]
}

impl Compact {
    /// Decides this frame's mode and returns the commands for a change, if any. The size to
    /// restore is read here, once, on entering; never while a resize may be in flight.
    pub fn frame(&mut self, facts: WindowFacts, layout: Option<&Layout>) -> Vec<ViewportCommand> {
        let want = should_be_compact(self.enabled, facts.focused, layout.is_some(), facts.maximized, facts.fullscreen);
        match (self.frozen, want, layout, self.full_unit) {
            (None, true, Some(layout), Some(unit)) => {
                let size = window_size(layout, unit);
                self.frozen = Some(Frozen { unit, restore: facts.content_size });
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

    /// Called on each full frame with the key size `keyboard::show` used.
    pub fn record_full_unit(&mut self, unit: f32, content_size: Vec2) {
        let still_small = self.compact_size.is_some_and(|s| (s - content_size).length() < 1.0);
        if !still_small {
            self.full_unit = Some(unit);
        }
    }

    pub fn frozen_unit(&self) -> Option<f32> {
        self.frozen.map(|f| f.unit)
    }
}
```

Keep the test helper `overlaps_a_key` as written rather than calling `covers_a_key`: it's an independent check of the same property.

`Rect::from_points` exists in egui 0.36 (`emath::Rect::from_points(&[Pos2])`). If the compiler says otherwise, fold the four corners with `min`/`max` as the original test helper did.

Until Task 3 uses them, the `pub` items would trip `dead_code` under clippy `-D warnings`. Add `#![allow(dead_code)] // wired up in the next task` as the first line after the module doc comment, and remove it in Task 3.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked ui::compact`
Expected: 9 passed.

Then: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/ui/compact.rs src/ui/mod.rs
git commit -m "feat: compact-mode state machine and geometry

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Fixed key size and translucent keys in `keyboard::show`

**Files:**
- Modify: `src/ui/keyboard.rs` (`View`, `show`, tests)
- Modify: `src/ui/mod.rs:374` (the one call site in `App::central`)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub enum Fit { Fill, Fixed { unit: f32, margin: f32 } }` (`Debug, Clone, Copy, PartialEq`)
  - `pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, view: View<'_>, fit: Fit) -> Option<f32>` — returns the key size used, `None` if there is no layout or keymap.
  - `View` gains `pub translucent: bool` (default false): key fills drawn at alpha 0.9.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/ui/keyboard.rs`:

```rust
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
        let _ = ctx.run_ui(raw, |ui| {
            avail = ui.available_size();
            unit = show(ui, state, Instant::now(), View::default(), fit);
        });
        (unit, avail)
    }

    #[test]
    fn fill_reports_the_key_size_it_chose() {
        let (unit, avail) = run(&lily58_state(), Fit::Fill);
        // Lily58 spans 16.5 × 5.75 key units; Fill leaves 24pt around the keys.
        let expected = ((avail.x - 24.0) / 16.5).min((avail.y - 24.0) / 5.75).max(8.0);
        let unit = unit.expect("a keyboard is loaded");
        assert!((unit - expected).abs() < 1e-3, "{unit} vs {expected}");
    }

    #[test]
    fn fixed_uses_the_key_size_it_is_given() {
        let (unit, _) = run(&lily58_state(), Fit::Fixed { unit: 37.0, margin: 8.0 });
        assert_eq!(unit, Some(37.0));
    }

    #[test]
    fn no_keyboard_no_key_size() {
        let state = AppState::new(HostLayout::Gb, crate::layers::TriLayer::default(), false);
        assert_eq!(run(&state, Fit::Fill).0, None);
    }
```

Extend the existing `a_default_view_highlights_nothing` test with `assert!(!view.translucent);`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked ui::keyboard`
Expected: compile error — `Fit` not found, `show` takes 4 arguments.

- [ ] **Step 3: Implement**

In `src/ui/keyboard.rs`, after the `FINGER_STROKE` constant:

```rust
/// Key fill alpha in compact mode, so what's underneath shows faintly through.
const TRANSLUCENT_ALPHA: f32 = 0.9;
/// Space `Fit::Fill` leaves around the keys, in total per axis. Points.
const FILL_PADDING: f32 = 24.0;

/// How big to draw the keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fit {
    /// As large as fits, centred.
    Fill,
    /// This key size in points, anchored `margin` points from the top-left.
    Fixed { unit: f32, margin: f32 },
}
```

Add to `View`, after `hint`:

```rust
    /// Draw key fills slightly see-through (compact mode).
    pub translucent: bool,
```

Change `show`'s signature and its sizing lines. The current body:

```rust
pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, view: View<'_>) {
    let (Some(layout), Some(keymap)) = (&state.layout, &state.keymap) else { return };
    ...
    let avail = ui.available_size();
    let unit = ((avail.x - 24.0) / span_x).min((avail.y - 24.0) / span_y).max(8.0);
    let (response, painter) = ui.allocate_painter(avail, Sense::hover());
    let origin = response.rect.center() - Vec2::new(span_x, span_y) * unit / 2.0;
```

becomes:

```rust
/// Draws the keyboard and returns the key size used, in points.
pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, view: View<'_>, fit: Fit) -> Option<f32> {
    let (Some(layout), Some(keymap)) = (&state.layout, &state.keymap) else { return None };
    ...
    let avail = ui.available_size();
    let (response, painter) = ui.allocate_painter(avail, Sense::hover());
    let (unit, origin) = match fit {
        Fit::Fill => {
            let unit = ((avail.x - FILL_PADDING) / span_x).min((avail.y - FILL_PADDING) / span_y).max(8.0);
            (unit, response.rect.center() - Vec2::new(span_x, span_y) * unit / 2.0)
        }
        Fit::Fixed { unit, margin } => (unit, response.rect.min + Vec2::splat(margin)),
    };
```

In the per-key loop, after `fill` is chosen, apply the alpha:

```rust
        let fill = if view.translucent { fill.gamma_multiply(TRANSLUCENT_ALPHA) } else { fill };
```

End the function with `Some(unit)` after the loop.

In `src/ui/mod.rs` `App::central`, change the call to pass `keyboard::Fit::Fill` and add `translucent: false` to the `View` literal; discard the return value for now with `let _ =` (Task 3 records it):

```rust
            let _ = keyboard::show(ui, &self.state, now, keyboard::View {
                unlock_keys: self.unlock_highlight(),
                fingers: self.finger_colours_shown(),
                hint: self.tutor.hint(),
                translucent: false,
            }, keyboard::Fit::Fill);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked ui::keyboard`
Expected: 6 passed.

Run: `cargo test --locked && cargo clippy --locked --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/ui/keyboard.rs src/ui/mod.rs
git commit -m "feat: keyboard picture can draw at a fixed key size, translucent

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Wire compact mode into the app

**Files:**
- Modify: `src/ui/compact.rs` (add `layer_text` and `show`; remove the `dead_code` allow)
- Modify: `src/ui/status.rs` (extract `last_key_text`; add the checkbox)
- Modify: `src/ui/mod.rs` (`App::compact`, `run`, `central`, `ui`, `clear_color`, tests)

**Interfaces:**
- Consumes: from Task 1, `compact::{Compact, WindowFacts, MARGIN, label_slots}`, `Compact::{frame, record_full_unit, frozen_unit}`, `Compact.enabled`; from Task 2, `keyboard::{Fit, View { translucent }}` and `keyboard::show(..) -> Option<f32>`.
- Produces:
  - `status::last_key_text(last: &LastKey) -> String` (`pub(super)`), e.g. `"KC_QUOT  →  '"`.
  - `compact::layer_text(state: &AppState, now: Instant) -> String` — `"Layer N"` / `"Layer ?"`.
  - `compact::show(ui: &mut egui::Ui, app: &App, now: Instant, unit: f32)`.
  - `App::compact: compact::Compact`.

- [ ] **Step 1: Write the failing tests**

In `src/ui/compact.rs` tests:

```rust
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
```

In `src/ui/mod.rs` tests (these use the existing `app()` and `connected_lily58()` helpers):

```rust
    fn unfocused() -> compact::WindowFacts {
        compact::WindowFacts { focused: false, maximized: false, fullscreen: false, content_size: egui::Vec2::new(960.0, 460.0) }
    }

    #[test]
    fn unplugging_while_compact_brings_the_full_window_back() {
        let (mut app, _events, _commands) = app(None);
        app.on_device_event(connected_lily58(), Instant::now());
        app.compact.enabled = true;
        app.compact.record_full_unit(40.0, egui::Vec2::new(960.0, 460.0));
        assert!(!app.compact_frame(unfocused()).is_empty());
        assert_eq!(app.compact.frozen_unit(), Some(40.0));

        app.on_device_event(DeviceEvent::Disconnected, Instant::now());
        assert_eq!(app.compact_frame(unfocused()), compact::leave_commands(egui::Vec2::new(960.0, 460.0)));
        assert_eq!(app.compact.frozen_unit(), None);
    }

    #[test]
    fn compact_is_off_at_startup() {
        let (mut app, _events, _commands) = app(None);
        app.on_device_event(connected_lily58(), Instant::now());
        app.compact.record_full_unit(40.0, egui::Vec2::new(960.0, 460.0));
        assert!(!app.compact.enabled);
        assert!(app.compact_frame(unfocused()).is_empty());
    }

    #[test]
    fn a_full_frame_records_the_key_size_for_compact_mode() {
        let (mut app, _events, _commands) = app(None);
        app.on_device_event(connected_lily58(), Instant::now());
        app.compact.enabled = true;
        let ctx = app.ctx.clone();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(960.0, 460.0))),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.central(ui, Instant::now()));
        let commands = app.compact_frame(unfocused());
        assert_eq!(commands.first(), Some(&egui::ViewportCommand::Decorations(false)));
        assert!(app.compact.frozen_unit().is_some_and(|u| u > 8.0));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked ui::`
Expected: compile errors — no `layer_text`, no field `compact` on `App`, no method `compact_frame`.

- [ ] **Step 3: Implement `status::last_key_text` and the checkbox**

In `src/ui/status.rs`, add `use crate::state::LastKey;` to the imports (beside `use crate::state::Tier;` — make it `use crate::state::{LastKey, Tier};`) and add:

```rust
/// The last key as the status bar shows it, e.g. `KC_QUOT  →  '`.
pub(super) fn last_key_text(last: &LastKey) -> String {
    let typed = last.text.as_deref().map(|t| format!("  →  {}", visible(t))).unwrap_or_default();
    format!("{}{typed}", last.label)
}
```

and use it in `show`:

```rust
            Some(last) => {
                ui.label(RichText::new(last_key_text(last)).monospace().strong());
```

After the Finger colours checkbox block (before the `ui.separator();` that precedes Reload), add:

```rust
        ui.checkbox(&mut app.compact.enabled, "Compact when unfocused").on_hover_text(
            "When this window loses focus, shrink it to just the keys, see-through and click-through. \
             Switch back with Alt+Tab, the taskbar or the Overview.",
        );
```

- [ ] **Step 4: Implement `compact::layer_text` and `compact::show`**

In `src/ui/compact.rs`, remove the `#![allow(dead_code)]` line and extend the imports:

```rust
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Vec2, ViewportCommand, pos2};

use super::{App, keyboard, status};
use crate::layout::Layout;
use crate::state::AppState;
```

Add:

```rust
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
        painter.rect_filled(bg, CornerRadius::same(4), LABEL_BG);
        painter.galley(pos2(bg.min.x + pad, bg.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    }
}
```

`unlock_highlight`, `finger_colours_shown` and the `tutor`/`state` fields are private to `App` but visible to the child module `compact`, the same way `status.rs` reaches `app.tutor`.

- [ ] **Step 5: Wire it into `App`**

In `src/ui/mod.rs`:

1. Add the field after `finger_colours` in `struct App`:

```rust
    /// Compact-when-unfocused: the checkbox, and while compact, the frozen key size.
    compact: compact::Compact,
```

and `compact: compact::Compact::default(),` in `with_device`'s initialiser after `finger_colours: true,`.

2. In `run`, add `.with_transparent(true)` to the `ViewportBuilder` chain after `.with_min_inner_size([480.0, 240.0])`. Full mode stays opaque because the panels paint `panel_fill`.

3. In `central`, record the key size:

```rust
        if self.state.layout.is_some() {
            let unit = keyboard::show(ui, &self.state, now, keyboard::View {
                unlock_keys: self.unlock_highlight(),
                fingers: self.finger_colours_shown(),
                hint: self.tutor.hint(),
                translucent: false,
            }, keyboard::Fit::Fill);
            if let Some(unit) = unit {
                self.compact.record_full_unit(unit, ui.ctx().content_rect().size());
            }
            return;
        }
```

4. Add the per-frame decision method to `impl App` (after `unlock_highlight`):

```rust
    /// Split from `ui` so tests can drive the mode changes without a real window.
    fn compact_frame(&mut self, facts: compact::WindowFacts) -> Vec<egui::ViewportCommand> {
        self.compact.frame(facts, self.state.layout.as_ref())
    }
```

5. In `eframe::App for App`, add:

```rust
    /// Transparent so compact mode can show what's underneath; full mode's panels paint over it.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }
```

6. In `ui`, after the Ctrl+T handling and before `egui::Panel::bottom("status")`, decide the mode and branch:

```rust
        let ctx = ui.ctx().clone();
        let facts = ctx.input(|i| compact::WindowFacts {
            focused: i.focused,
            maximized: i.viewport().maximized == Some(true),
            fullscreen: i.viewport().fullscreen == Some(true),
            content_size: ctx.content_rect().size(),
        });
        for command in self.compact_frame(facts) {
            ctx.send_viewport_cmd(command);
        }
        if let Some(unit) = self.compact.frozen_unit() {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| compact::show(ui, self, now, unit));
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
```

Note `ctx.content_rect()` inside `ctx.input(..)` would re-lock the context; if that deadlocks or the borrow checker objects, read `let content_size = ctx.content_rect().size();` on its own line first and use it in the struct.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --locked`
Expected: all pass, including the 4 new tests.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Smoke-run**

Run: `cargo build --release && ./target/release/lily58-assistant`
Tick **Compact when unfocused**, click another window: the assistant becomes keys plus labels on a transparent background. Alt+Tab back: full window at its previous size. Close it.

- [ ] **Step 8: Commit**

```bash
git add src/ui/compact.rs src/ui/status.rs src/ui/mod.rs
git commit -m "feat: compact when unfocused

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Documentation

**Files:**
- Modify: `README.md` (the "Keeping the window on top" section, around line 104)
- Modify: `docs/manual-test-checklist.md` (new section before "## Robustness")

**Interfaces:** none.

- [ ] **Step 1: README**

Append to the "## Keeping the window on top" section, after the GNOME bullet:

```markdown

To cover less of what's underneath, tick **Compact when unfocused** in the status bar. When you click another window, the assistant shrinks to just the keys (at the same size), with the last key and layer beneath them, on a see-through background that lets clicks through to the window below. It comes back in full when focused again. Because clicks pass through it, switch back with the keyboard or desktop: Alt+Tab, the taskbar, or the Overview. The setting isn't saved. It has no effect while the window is maximized or fullscreen.
```

- [ ] **Step 2: Manual checklist**

Insert before `## Robustness`:

```markdown
## Compact when unfocused (run on GNOME and KDE)
- [ ] The **Compact when unfocused** checkbox is off at startup.
- [ ] With it on and keep-above set, clicking another window turns the assistant into keys plus labels, keys the same size, see-through between them.
- [ ] Clicks between the keys and on them reach the window underneath.
- [ ] Alt+Tab, the taskbar or the Overview brings back the full, opaque window at its previous size.
- [ ] The labels update while typing in another window (all-windows or live-layers tier); the layer label shows `Layer ?` without live layers.
- [ ] A drill in progress pauses and resumes on refocus.
- [ ] Unplugging the keyboard while compact brings the full window back, showing "Waiting for Lily58…".
- [ ] Maximized, the window doesn't go compact.
- [ ] Several focus round trips in a row don't make the keys smaller.
```

- [ ] **Step 3: Commit**

```bash
git add README.md docs/manual-test-checklist.md
git commit -m "docs: compact when unfocused

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Verify on this desktop and open the PR

**Files:** none changed unless verification finds a bug (then fix via TDD in the task that owns the code, and commit).

- [ ] **Step 1: Full CI locally**

Run: `cargo build --locked && cargo test --locked && cargo clippy --locked --all-targets -- -D warnings`
Expected: all pass.

- [ ] **Step 2: KDE Wayland screenshots**

Run the release build, tick the checkbox, focus another window, and capture with `spectacle -b -n -f -o <scratchpad>/compact_wayland.png`. Confirm: no title bar, keys at the same size as full mode, labels under the outer bottom keys, background see-through. Refocus and confirm the full opaque window at its previous size.

Kill the app by PID (`./target/release/lily58-assistant & APP=$!; …; kill $APP`), not `pkill -f`, which can match the invoking shell.

- [ ] **Step 3: XWayland**

Repeat with `env -u WAYLAND_DISPLAY ./target/release/lily58-assistant`.

- [ ] **Step 4: Push and open the PR**

```bash
git push -u origin feat/compact-when-unfocused
gh pr create --base main --title "feat: compact when unfocused" --body "<summary; what was verified on KDE Wayland/XWayland; manual checklist items left for GNOME>

🤖 Generated with [Claude Code](https://claude.com/claude-code)"
```

If `git push` fails on SSH auth, push through the gh credential helper:
`git push https://github.com/KaiEkkrin/lily58_assistant.git feat/compact-when-unfocused` with `gh auth setup-git` configured.

- [ ] **Step 5: Clean up the spike**

Delete the throwaway probe's build output: `rm -rf target/probe`.
