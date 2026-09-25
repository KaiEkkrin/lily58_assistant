# Compact When Unfocused — Design

Date: 2026-09-25
Status: Approved (design); not yet implemented

## Purpose

The assistant is often kept always-on-top while the user types in another
window. Most of its window is then blank space — margins around the keyboard,
the gap between the halves, the status bar, the title bar — and all of it hides
whatever is underneath. This feature makes the unfocused window cover as little
as possible without shrinking the keys: it drops to the keys and two small
labels on a transparent, click-through background, and comes back in full when
focused again.

It must behave the same on GNOME and KDE, under Wayland and X11.

## Key non-feature: still strictly read-only

Nothing here touches the keyboard. `hid::guard::ReadOnlyGuard` is untouched.

## Scope

In scope:

- A session-only **Compact when unfocused** checkbox in the status bar.
- A compact mode: no title bar, whole-window click-through, window shrunk to
  the keys at their current size, transparent background, keys about 90%
  opaque, last-key and layer labels in the free space under the outer columns.
- Returning to the normal window, at its previous size, on focus.

Out of scope:

- Setting always-on-top from the app. The user keeps using the window
  manager's "Keep above" / "Always on top".
- A config-file key. The checkbox is off at every start.
- Partial click-through (e.g. a grab handle). winit only offers all-or-nothing;
  a per-region input shape would mean raw Wayland and XShape code.
- Moving the window to cancel the shift when the mode changes.

## Findings that shape the design

From a throwaway probe (eframe 0.36.2 / winit 0.30.13) run on KDE Plasma
under Wayland and XWayland, and from the winit source:

| | Wayland (GNOME and KDE) | X11 / XWayland |
|---|---|---|
| Transparent background | works (tested) | works (tested) |
| Click-through, whole window | works (tested; empty input region) | works (XShape input) |
| Removing the title bar at runtime | works (tested on KDE's server-side decorations; GNOME's frame is winit's own sctk-adwaita frame, which is hidden) | works (tested) |
| Resizing at runtime | works unless maximized, fullscreen or tiled (tested) | works (tested) |
| Moving its own window | ignored by winit | works |
| Setting always-on-top | ignored by winit | works |
| Window position and outer size from egui | `None` | reported |

A focused window that is transparent between the keys looks as though clicks
there will pass through, and they don't. So the full window stays opaque; only
compact mode is transparent.

## Decisions taken, and why

| Decision | Why |
|---|---|
| One window switching modes, not a second overlay window | A Wayland app can't restore a minimized window, and a new window would inherit neither the user's keep-above setting nor its position. |
| Checkbox only, off at start | The user's choice. Compact mode changes how the window behaves on every focus change, which would surprise anyone not using keep-above. |
| Go compact even while the tutor is open | Unfocused always means compact. A drill already pauses on focus loss (`Attempt::pause`), and the panel comes back on refocus. |
| Click-through always on while compact | Tested and wanted. While unfocused the user is typing elsewhere and never needs to click the assistant. |
| Keys keep the size they had in full mode | The user wants less screen space covered without smaller keys. |
| Accept the shift on mode change | The title bar disappears and the keys go from centred to top-left anchored; Wayland won't let the app move itself to compensate. It is small when the window is sized snugly. |

## Behaviour

The window is compact when **all** of these hold:

1. the checkbox is on;
2. the window is unfocused (`i.focused` is false);
3. the keyboard picture is showing (`state.layout` is `Some`);
4. the window is neither maximized nor fullscreen (`viewport().maximized` and
   `viewport().fullscreen` are not `Some(true)`).

Otherwise it is full. When a condition stops holding while compact — e.g. the
keyboard is unplugged — the window returns to full, so *Waiting for Lily58…*
is visible.

**Entering compact** (on the first frame the conditions hold):

- freeze the key size (`unit`) recorded on the last full frame;
- record the size to restore, `ctx.content_rect().size()`, read once here and
  never while changing modes, since a resize lands a frame or two later;
- send `Decorations(false)`, `MousePassthrough(true)`, `InnerSize(compact size)`.

**Leaving compact** (on the first frame they don't):

- send `Decorations(true)`, `MousePassthrough(false)`, `InnerSize(restore size)`.

**Shown while compact:** the keys and the two labels, nothing else. The status
bar, tutor panel, dialogs (`dialogs::show`) and error lines are not drawn; they
come back on refocus. The checkbox can't be unticked while compact, which is
fine, since compact ends on refocus.

**Full mode** is unchanged from today, including its opaque background.

## Drawing and sizing

### Keeping the key size

`keyboard::show` today scales keys to fill the space it is given and centres
them. It gains a sizing mode:

- `Fit::Fill` — today's behaviour;
- `Fit::Unit(f32)` — use this key size, anchored at the top-left of the space
  with the compact margin.

It returns the key size it used. Full mode records that every frame; entering
compact freezes the last value.

### Compact window size

`width = span_x × unit + 2 × MARGIN` and
`height = span_y × unit + 2 × MARGIN (+ one label row when needed)`, where
`span` comes from `Layout::bounds()` and `MARGIN` is 8pt, enough that the key
outlines aren't clipped. Sizes are in egui points, which is what
`ViewportCommand::InnerSize` takes.

### Transparency

- The viewport is built with `.with_transparent(true)`.
- `App::clear_color` returns fully transparent.
- In full mode the panels paint `panel_fill` as they do now, so the window
  looks exactly as it does today.
- In compact mode only a `CentralPanel` with `Frame::NONE` is drawn, and key
  fills use alpha about 0.9 (a `View` field, default 1.0).

### Labels

- **Left:** the last key, formatted as in the status bar (e.g. `KC_QUOT → '`),
  or empty before the first key.
- **Right:** `Layer N`, or `Layer ?` when the layer is unknown
  (`!state.matrix_active`, matching the status bar's "Layer: unknown").
- Each has a small dark rounded background so it reads over any content, with
  text the size of the key labels.
- **Placement:** anchored to the bottom-left and bottom-right corners of the
  key bounds. If a label's rectangle overlaps any key's bounding box, the
  window gains one label row below the keys and both labels go there instead.
  On the Lily58 there is a gap of about 1.3 keys under the outer columns
  (LCTL/Z and `/`/RSFT, beside the thumb clusters), so no row is added.

### Known edge case

If the compositor ignores the shrink (e.g. GNOME edge-tiling, which isn't
reported as maximized), the window stays big but transparent and
click-through, with the keys at the top-left. It covers nothing, just less
tidily.

## Code layout

- **`src/ui/compact.rs` (new).** Pure logic:
  - `should_be_compact(enabled, focused, has_layout, maximized, fullscreen) -> bool`
  - `window_size(bounds, unit, label_row) -> Vec2`
  - label placement: `label_slots(layout, unit, label_size) -> Slots`, which
    reports the two rectangles and whether a label row is needed
  - `enter_commands(size) -> Vec<ViewportCommand>` and
    `leave_commands(size) -> Vec<ViewportCommand>`
  
  and the compact drawing: keys at the frozen size plus the labels.
- **`src/ui/mod.rs`.** A `Compact` state on `App`: the checkbox flag, the key
  size recorded on the last full frame, and while compact, the frozen key size
  and the size to restore. Each frame, `ui` gathers the window facts and calls
  a separate method (see Testing) that decides the mode and returns any
  transition commands; `ui` sends them and draws either the full layout or the
  compact one. `clear_color` is added; `run` adds `.with_transparent(true)`.
- **`src/ui/status.rs`.** The checkbox, beside Finger colours.
- **`src/ui/keyboard.rs`.** `Fit`, the returned key size, and the key alpha in
  `View`.

## Testing

Test-driven, in the existing `#[cfg(test)]` style; CI runs build, test and
`clippy -D warnings`.

### `compact.rs`

- `should_be_compact` for each combination of the four conditions.
- `window_size` from bounds and unit, with and without a label row.
- On the captured Lily58 layout (`tests/fixtures/lily58-definition.json`): both
  label rectangles overlap no key, and no label row is needed.
- On a small layout with no gap under the corners: a label row is needed.
- `enter_commands` and `leave_commands` contain the expected commands.

### `ui/mod.rs`, with the existing `app()`/`connected()` harness

- Focus lost with the checkbox on: the app is compact, with the frozen key size
  and restore size recorded.
- Focus regained: back to full.
- Keyboard disconnected while compact: back to full.
- Checkbox off: focus loss changes nothing.

To make these testable without a real window, the per-frame decision is split
from `ui` into a method taking the focus/maximized/fullscreen facts and the
current content size, and returning the commands to send (as the tutor
split `apply_focused` from `logic`).

### `keyboard.rs`

- `Fit::Unit` uses the given size; `Fit::Fill` returns the size it computed.

### Manual checklist additions

A *Compact when unfocused* section in `docs/manual-test-checklist.md`, to run
on both GNOME and KDE:

- [ ] The checkbox is off at startup.
- [ ] With it on and keep-above set, clicking another window turns the
      assistant into keys plus labels, keys the same size, see-through between
      them.
- [ ] Clicks between the keys and on them reach the window underneath.
- [ ] Alt+Tab, the taskbar or the Overview brings back the full, opaque window
      at its previous size.
- [ ] The labels update while typing in another window (all-windows or
      live-layers tier).
- [ ] A drill in progress pauses and resumes on refocus.
- [ ] Unplugging the keyboard while compact brings the full window back.
- [ ] Maximized, the window doesn't go compact.

## Documentation

A short README section: what the checkbox does, that it pairs with the window
manager's keep-above, and that the compact window is brought back with the
keyboard (Alt+Tab, taskbar, Overview), not the mouse.

## Deliberate omissions

- No persistence of the checkbox.
- No partial click-through or grab handle.
- No compensation for the shift on mode change.
- No attempt to set keep-above from the app.
