# Lily58 Assistant — implementation notes

What we learned building v1 that isn't obvious from the code or the
[design spec](superpowers/specs/2026-09-12-lily58-assistant-design.md): facts about
the hardware, firmware behaviour found the hard way, decisions made in review,
and known issues. The v1 history was squashed into one commit, so this file
carries what the individual commits and review notes held.

## The keyboard

Measured on the Mechboards Lily58 Pro R2G with Vial firmware (`--probe` shows
most of this):

- USB `7171:0012`, product "Lily58 Pro R2G", serial `vial:f64c2b3c`. Vial
  protocol 6, VIA protocol `0x0009`, 4 layers, matrix 10×6.
- USB interfaces: `if00` is the keyboard (`event-kbd`); `if01` is the Vial raw-HID
  interface (usage page `0xFF60`); `if02` carries mouse, joystick and
  extra-key event nodes. So there are four `/dev/input/event*` nodes, and only
  the `if00` one carries ordinary key presses.
- The definition has 60 keys, not 58: the extra two are the encoder push
  switches (`4,5` and `9,0`). Its four encoder-rotation entries (`e` in the
  label) are skipped because they have no matrix position.
- The left half's matrix columns run **from the inside out**: `0,0` is the "5"
  key and `0,5` is Esc. The right half is rows 5–9. Don't guess key positions
  from the usual Lily58 matrix; read them from the definition.
- The Vial unlock keys are matrix `(0,0)` + `(0,1)`, the **4 and 5** keys.
- Layer 3 of the stock keymap has keys `0x7842`–`0x784A`: QMK's RGB Matrix
  controls (`RM_TOGG`, next/previous effect, hue, saturation and brightness
  up/down). `keycodes.rs` doesn't name them yet, so they show as raw codes.

## Firmware behaviour (vial-qmk)

- **Unlock polls must be more than 100 ms apart.** `vial_unlock_poll` in
  vial-qmk's `quantum/vial.c` steps the 50-step countdown only when the keys
  are held *and* more than 100 ms have passed since the last step. Any other
  poll restarts the countdown at 50. An early version polled every 50 ms and
  could never unlock. The worker now polls every 200 ms (Vial's GUI uses a
  200 ms `QTimer` in `unlocker.py`), so an unlock takes about 10 s. The worker
  also keeps that spacing when a command wakes it early.
- An in-progress unlock doesn't block typing in the firmware source. In
  `quantum/via.c`, `raw_hid_receive` only skips non-Vial raw-HID commands.
  During the broken 50 ms attempt, though, the keyboard seemed to stop typing
  until it was replugged. That was never confirmed or explained.
- Unlocked stays unlocked until the keyboard loses power. Restarting the
  assistant finds it unlocked and shows live layers straight away (checked on
  hardware).

## Linux

- **Joystick nodes are readable without any setup.** systemd's uaccess rule
  gives the seat user the Lily58's `if02` joystick node, on Fedora and Ubuntu
  alike, while the keyboard node needs our udev rule. v1 at first counted
  all-windows tracking as working if *any* node opened. It then ignored
  focused-window input, while the joystick node delivered no key presses.
  Tracking now counts as working only when no node is denied
  (`input::evdev::status_for`).

## Decisions made in review

- `TT(n)` turns on its layer as soon as it is held, like `MO`. The plan had
  gated it on the tapping term.
- On resume after another program released the keyboard, the worker re-reads
  `unlock_status` before anything else. That program may have locked it,
  unlocked it, or left an unlock running.
- A failure that will simply repeat (e.g. a bad definition) is reported once
  and retried every 5 s (`RETRY_AFTER_ERROR`), not every scan.
- OS key releases are applied even while the matrix tier is active, and OS
  held keys are cleared when the tier changes, so no key stays highlighted.

## Testing lesson

`hid::fake::KeyboardSim` stands in for the firmware in tests. It first left out
`vial.c`'s restart-on-early-poll branch, so every unlock test passed while the
real keyboard could never unlock. When the app starts depending on a new
firmware behaviour, check the simulator against vial-qmk's source, not
against the spec's summary of it.

## Known issues

From the final whole-branch review (not yet fixed):

1. **Keys stick after Alt+Tab.** On Wayland a key held while the window loses
   focus never gets its release. Fix: on `egui::Event::WindowFocused(false)`,
   clear focused-source held keys and the shift state.
2. **Tab, Space and Enter work the app's own buttons**, including the unlock,
   which can't be cancelled. Fix: remove those key events from egui's input
   after `focused::translate` has read them, or make the buttons unfocusable.
3. **Config errors vanish.** The config error goes into the same field as
   device errors, the first `Connected` clears it, and it is never logged.
   Fix: a separate `config_error` field plus `log::warn!`.
4. **Vial may not detect the keyboard while the assistant is unlocked** (not
   yet checked on hardware). Every program with the hidraw node open receives
   every reply. The assistant polls the matrix every 10 ms, but checks for
   other holders only once a second. So Vial's quick identify request can
   read one of our replies. Remedies range from documenting "close the
   assistant first" to watching the node with inotify.

Also left for later: add `. "$HOME/.cargo/env"` to the README's Ubuntu steps.
The manual checklist needs steps for Alt+Tab, Tab/Space in the window, Vial
detection while unlocked, and TG followed by Reload.

Minor findings from the per-task reviews, all judged fine to defer:

- Device worker: a device vanishing between discovery and the first read is
  reported as an error, not a disconnect. A leftover unlock yields a
  duplicate `Unlocked` event. Changes to the list of other programs holding
  the keyboard aren't re-reported mid-session. After unplug and replug, an
  identical error stays suppressed and the old one stays on screen. If the
  worker thread fails to start, the UI gets no events at all. Every unlock
  poll sends an `Unlocking` event and a repaint, even when the counter hasn't
  changed.
- hidraw: an `EINTR` from `poll` shortens the read timeout; a short or
  interrupted write is treated as a hard error. `find_vial_device` passes on
  `read_dir` errors other than NotFound.
- evdev: open failures other than permission denied end up as `NotFound`; the
  node list sorts as text (`event10` before `event2`).
- UI: the central panel can show "Paused" while the unlock window is open.
  The ✔/✖ glyphs are unverified in egui's default fonts. Send errors on the
  device command channel are ignored.
- Small things: `protocol::report()` panics without a message on more than
  32 bytes. `Layout::key()` and `keycodes::basic_name` are linear scans (the
  latter runs per key per frame). Unparseable slot-0 layout labels are
  dropped silently. `config_path` is relative when neither `HOME` nor
  `XDG_CONFIG_HOME` is set. `VialError` has two I/O variants (`Io` and
  `Guard(Io)`).
- Tests: no `LayerTracker` test for `LM(layer, mods)`. The decode tests omit
  `PersistentDefault`/`TriLayerUpper`. There's no dedicated test that a
  locked keyboard gets no matrix requests. `fake.rs` holds both the scripted
  transport and the firmware simulator.
