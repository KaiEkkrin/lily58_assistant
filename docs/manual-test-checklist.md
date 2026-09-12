# Manual test checklist

Run on each PC after changing input, device or UI code. Close Vial unless a step says otherwise.

## Startup
- [ ] Keyboard unplugged: the window shows "Waiting for Lily58…".
- [ ] Plug in: the keyboard picture appears within about a second, as two halves with the thumb clusters in place.
- [ ] `--probe` output matches what the window shows (layer 0 labels).

## Focused window (no evdev rule, keyboard locked)
- [ ] Typing into the window highlights the right keys; the status bar shows e.g. `KC_QUOT  →  '`.
- [ ] Shift+2 shows `"` and Shift+3 shows `£` (UK layout).
- [ ] Typing in another window changes nothing.
- [ ] Hold Shift, Alt+Tab to another window, release both there, then come back: no key stays highlighted, and typing 3 shows `3`, not `£` (#1).
- [ ] With the "Tracking tiers" window open, type Tab, Space and Enter several times: no button is pressed and no unlock starts, yet Space and Enter still highlight their keys (#2).

## All windows (evdev rule installed)
- [ ] After installing the rule and pressing Reload, the status bar says "all windows, no layer tracking".
- [ ] Typing in another window highlights keys in the assistant.
- [ ] A symbol that exists only on a layer shows "(probably layer N)".

## Live layers (unlocked)
- [ ] "how to enable more…" → "Unlock for layer tracking": the unlock keys turn orange; holding them fills the bar in about 10 s; the window closes.
- [ ] Holding LOWER switches the labels to layer 1 before any other key is pressed; releasing it returns to layer 0.
- [ ] Holding LOWER + RAISE shows layer 3 (with `tri_layer = [1, 2, 3]`).
- [ ] Transparent keys show the lower layer's label, dimmed.
- [ ] The status bar shows "Layer N" and "all windows, live layers".

## Robustness
- [ ] Opening Vial while the assistant runs shows "paused: vial (…) has the keyboard open", and Vial works normally. Closing Vial makes the assistant reconnect.
- [ ] Remapping a key in Vial, then closing Vial, shows the new label after the reconnect.
- [ ] Unplugging mid-session goes back to "Waiting…"; replugging reconnects, locked again.
- [ ] Start an unlock, then open Vial while it's in progress, then close Vial: the status bar no longer says "paused" and the unlock window comes back (#9).

## Desktops
- [ ] Fedora/KDE: runs; Alt+F3 → More Actions → Keep Above Others keeps it on top.
- [ ] Ubuntu/GNOME: builds from the README; runs; Alt+Space → Always on Top keeps it on top.
