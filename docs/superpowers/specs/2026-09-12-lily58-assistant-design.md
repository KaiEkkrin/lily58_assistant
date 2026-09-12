# Lily58 Assistant — Design

Date: 2026-09-12
Status: Approved in brainstorming; awaiting written-spec review

## Purpose

A desktop app that helps its user learn a Lily58 split keyboard. It draws the
keyboard on screen, highlights the keys being pressed, shows the last key typed
(keycode and character) in a status bar, and reflects the keyboard's current
keymap and active layer, read live from the keyboard over the Vial protocol.

It is designed to grow: a typing-tutor mode and similar features come later as
additional consumers of the same state and event stream.

## Key non-feature: strictly read-only

The app never remaps the keyboard. Remapping is done in Vial. A read-only guard
at the lowest level of the HID stack refuses every command not on an explicit
allowlist, so no present or future feature can write to the keyboard by
accident. The single, deliberate exception is Vial's unlock handshake (two
commands), which changes no keymap data; see "Read-only guard".

## Target environment

| Machine | Distro | Desktop | Session |
|---|---|---|---|
| Home | Fedora 44 | KDE Plasma | Wayland |
| Work | Ubuntu 26.04 | GNOME | Wayland |

The app is built from source on each machine. The keyboard is a Mechboards
Lily58 Pro R2G (USB `7171:0012`) running Vial firmware (serial `vial:f64c2b3c`),
with raw HID on usage page `0xFF60`, usage `0x61`, 32-byte reports.

## Technology choice

**Rust + egui/eframe, single native binary.**

- All hardware access uses kernel interfaces directly, with no system C
  libraries: raw HID via read/write on `/dev/hidrawN` with discovery through
  `/sys/class/hidraw`; `/dev/input` via the pure-Rust `evdev` crate; Vial
  definition decompression via the pure-Rust `lzma-rs` crate. No `hidapi`, no
  `libudev`.
- eframe/winit loads Wayland/X11/xkbcommon/EGL at runtime, which keeps
  distro-specific build dependencies to a minimum. The first implementation
  task is a smoke build of a bare eframe window on Fedora, to find out whether
  any `-dev` packages are needed; the README records the result.
- Minimum compiler is pinned with `rust-version = "1.95"` (eframe 0.36.2's minimum) in `Cargo.toml`. (Fedora's Rust
  is distro-packaged, not rustup, so `rust-toolchain.toml` would be ignored.)
  If Ubuntu's packaged Rust is older than that, the README says to install rustup.
- Other crates: `serde`, `serde_json`, `toml`, `log`, `env_logger`, `thiserror`
  (errors inside modules), `anyhow` (top level and `--probe`). Adding anything that links a system C library needs a stated reason.

Rejected alternatives: Python + PySide6 (Qt platform-plugin and `python-evdev`
build issues that differ by distro; Python version drift); Tauri (system
WebKitGTK, with distro-specific packages and Wayland rendering bugs); Electron
(heavy, native modules still need compiling).

## Platform constraints that shape the design

1. **Wayland only delivers key events to the focused window.** Seeing keys while
   unfocused needs a kernel-level source: `/dev/input` (evdev) or the
   keyboard's own matrix state over raw HID. Both behave the same on KDE and GNOME.
2. **Layer keys (MO, LT, …) send nothing to the host.** Only the matrix state
   can see them.
3. **Vial gates the matrix-state command behind its physical unlock**
   (vial-qmk `quantum/via.c`: "Disable wannabe keylogger unless unlocked"). The
   unlock is lost at power-off.
4. **Always-on-top is not implemented.** Wayland has no portable mechanism for
   it. The user pins the window by hand through the desktop's window menu, and
   the README gives the steps.

## Architecture

```
┌──────────────────────── UI thread (eframe) ─────────────────────────┐
│  KeyboardView    StatusBar    SetupHints    [later: TutorMode]      │
│                        ▲ reads                                      │
│      AppState: keymap, layer state, held keys, last key             │
│                        ▲ events over channels                       │
└────────────────────────┼────────────────────────────────────────────┘
      FocusedInput    EvdevInput (thread)    Device worker (thread)
      (egui events)                           │  owns the Vial connection:
                                              │  connect, read, poll matrix,
                                              │  unlock
                          layout ◄─ VialClient ─► ReadOnlyGuard ─► Transport (hidraw)
```

A single device worker thread owns the hidraw connection, so all requests to
the keyboard happen one at a time. The UI sends it commands (`Reload`,
`StartUnlock`) over a channel. The worker sends back events (`Connected {
layout, keymap }`, `Disconnected`, `MatrixChanged`, `UnlockStatus`, `Paused`,
`Error`) and calls `ctx.request_repaint()` after each one. The evdev thread
does the same with its key events.

### Units

| Unit | Responsibility | Depends on |
|---|---|---|
| `hid::discover` | Find the Vial raw-HID node: scan `/sys/class/hidraw`, match serial containing `vial:f64c2b3c` and report descriptor usage page `0xFF60`/usage `0x61`. Also reports the USB parent path, so evdev can match the same physical device. | std |
| `hid::transport` | `Transport` trait (send 32-byte report, receive with timeout) plus a `Hidraw` implementation over `/dev/hidrawN`. | std |
| `hid::guard` | `ReadOnlyGuard`: the only way to reach a `Transport`. Checks every outgoing report against the allowlist and refuses the rest. | `transport` |
| `vial` | `VialClient`: typed queries (protocol version, keyboard id, definition, layer count, keymap buffer, unlock status, matrix state, unlock start/poll). VIA replies echo the request and are matched by those bytes; Vial (`0xFE`) replies overwrite the buffer and cannot be matched. | `guard` |
| `layout` | Parse the Vial definition JSON (KLE layout plus matrix size) into key rectangles, each tagged with matrix `(row, col)`. | serde_json |
| `keycodes` | Decode QMK 16-bit keycodes into a label and meaning (`KC_A`, `MO(1)`, `LT(2,KC_SPC)`, mod-tap, `TG`, `TO`, `DF`, `OSL`, `TT`, `TL_LOWR`/`TL_UPPR`, …) using QMK's current numbering (Vial protocol v6+). | — |
| `hostlayout` | Keycode + shift state → character for the host OS layout (`gb` default, `us`). | — |
| `input::focused` | Turn egui key and text events into `KeyEvent`s. | egui |
| `input::evdev` | Read the Lily58's own event nodes (matched through the USB parent from `discover`). Emits `KeyEvent`s and reports its availability. | evdev |
| `device` | The worker thread: connection lifecycle, hotplug, matrix polling, detecting another program holding the device. | `vial`, `layout`, `discover` |
| `state` | `AppState` + `LayerTracker`: merge events into held keys, active layer, last key; map OS keycodes back to physical keys. | `keycodes`, `hostlayout` |
| `hints` | The single source of setup instructions (rule text, commands), used by both the UI and the README. | — |
| `ui` | Keyboard widget, status bar, hint popups, unlock dialog. | `state`, `hints` |
| `probe` | `--probe` CLI mode: print diagnostics without the GUI. | `discover`, `vial`, `layout`, `input::evdev` (availability check only) |
| `config` | Load `$XDG_CONFIG_HOME/lily58-assistant/config.toml` (default `~/.config/...`). Keys: `host_layout` (`"gb"` default, or `"us"`) and `tri_layer` (default `[1, 2, 3]`; `[]` disables). A missing file means defaults. | toml |

## Read-only guard

Allowlist (everything else is refused before it reaches the device):

| Command | Byte(s) | Purpose |
|---|---|---|
| `id_get_protocol_version` | `0x01` | VIA protocol version |
| `id_get_keyboard_value` | `0x02` | Read-only by definition; used for `id_switch_matrix_state` (`0x03`) and `id_layout_options` (`0x02`) |
| `id_dynamic_keymap_get_layer_count` | `0x11` | Layer count |
| `id_dynamic_keymap_get_buffer` | `0x12` | Keymap bytes |
| Vial `get_keyboard_id` | `0xFE 0x00` | Vial protocol version, keyboard UID |
| Vial `get_size` | `0xFE 0x01` | Definition size |
| Vial `get_def` | `0xFE 0x02` | Definition blocks |
| Vial `get_unlock_status` | `0xFE 0x05` | Unlocked flag, in-progress flag, unlock key positions |
| Vial `unlock_start` | `0xFE 0x06` | **Exception:** begin unlock handshake |
| Vial `unlock_poll` | `0xFE 0x07` | **Exception:** poll unlock handshake |

Command IDs must be checked against vial-qmk's `quantum/via.h` and
`quantum/vial.h` during implementation. The allowlist is a small static table in
one file. The guard's tests try every first byte `0x00`–`0xFF` and every `0xFE`
subcommand `0x00`–`0xFF`, and assert that exactly the allowlist passes. A refusal
returns an error, is logged, and shows in the UI as an error, since it can only
be a programming bug.

## Data flow

### On connect (at startup and on each hotplug)

The worker scans `/sys` once a second while no keyboard is connected. Then:

1. `get_keyboard_id`; if the Vial protocol is below v6, show an error naming the
   version found and the version needed, and stop.
2. `get_size`, then `get_def` block by block; decompress (xz/LZMA; check the
   container format against the real device) and parse into the layout.
3. `get_layer_count`, then `get_buffer` in chunks of ≤28 bytes for
   layers × rows × cols × 2 bytes (Lily58: about 480 bytes).
4. `get_unlock_status`; start matrix polling if unlocked.

The keymap is re-read only on connect and on **Reload** (Ctrl+R or button),
which also resets the layer state to layer 0.

### Input tiers

All available tiers run at the same time. The best one available decides what
gets highlighted: matrix > unfocused > focused.

| Tier | Needs | Sees | Doesn't see |
|---|---|---|---|
| Focused | nothing | keys and actual characters while the app is focused | anything while unfocused; layer keys |
| Unfocused (`/dev/input`) | udev rule | the Lily58's keys in any window (other keyboards ignored) | layer keys |
| Matrix | Vial unlock | every physical switch, including layer keys; polled 100×/sec | — |

- **Matrix:** each poll's bitmap is compared with the previous one to produce
  press/release events with exact `(row, col)`.
- **Without matrix:** an OS keycode is mapped to a physical key by finding the
  key whose keycode on the active layer produces it, falling back to layer 0 and
  then to each other layer, so symbols that exist only on a layer still get a
  position, shown as "probably layer N". The first match in matrix order wins.
- While the keyboard is locked, the worker polls unlock status every 2 s,
  so an unlock done elsewhere (e.g. in the Vial GUI) is picked up.

### Unlock

The **Unlock for layer tracking** button sends `unlock_start` and then polls
`unlock_poll` every 50 ms. The firmware counts down from 50 at most once per
100 ms while the keys are held, so an unlock takes about 5 s. The keyboard picture
highlights the unlock key positions reported by `get_unlock_status`, and a
window shows the progress.

Firmware facts (vial-qmk `via.c`/`vial.c`) that shape this:

- Once `unlock_start` is sent, the firmware answers only Vial commands, and
  **echoes every VIA command back unprocessed**, until the unlock completes or
  the keyboard loses power. Nothing can cancel an unlock, so the unlock window
  has no Cancel button and tells the user to unplug to abort (Vial's own dialog
  works the same way).
- An unlock may be left in progress by an earlier run. On connect, the worker
  therefore sends only Vial commands (keyboard id, unlock status). If an unlock
  is in progress, it resumes the unlock window and reads the keymap only after
  the unlock completes.
- While locked, a matrix-state request is echoed back, which reads as "nothing
  pressed". Matrix polling therefore only runs once `get_unlock_status` says
  unlocked.

### Layer tracking (matrix tier only)

`LayerTracker` models QMK's layer state: a default layer (`DF`), toggled layers
(`TG`, `TO`), and momentary layers from held keys (`MO`; `LT` and `TT` while
held; `OSL` until the next key press). The active layer is the highest one set.
`TL_LOWR`/`TL_UPPR` use the tri-layer layers (default lower 1, upper 2, adjust 3).
The stock Lily58 Vial keymap computes ADJUST in firmware code
(`update_tri_layer_state(state, _RAISE, _LOWER, _ADJUST)` in `keymap.c`), which
Vial can't report. The `tri_layer` config key (default `[1, 2, 3]`) therefore
emulates it: with layers 1 and 2 both active, layer 3 is shown too. `[]` turns
the emulation off for firmware without it.
An `LT` key counts as held once another key is pressed while it's down, or it's
been held for more than 200 ms (QMK's default tapping term).

Known limits, documented in the README: other layer logic compiled into the
firmware isn't visible over Vial, and tap/hold timing is approximate.

### Display

- **Keyboard view:** drawn from the layout geometry, with labels for the active
  layer. Transparent keys (`KC_TRNS`) show the label from the layer beneath,
  dimmed. Held keys are highlighted.
- **Status bar:**
  - the last key's keycode name and character (e.g. `KC_QUOT → '`)
  - the active layer
  - the current tier, with a "how to enable…" link that opens the hints for
    the missing tiers
- **Characters:** from the egui text event when focused; otherwise from
  `hostlayout` using the keycode and the tracked shift state.

## Error handling

| Situation | Behaviour |
|---|---|
| Lily58 not plugged in | "Waiting for Lily58…"; connects on its own when plugged in |
| Raw-HID node not accessible | No keyboard picture; focused keystrokes still appear in the status bar; a hint shows the hidraw udev rule and commands |
| Unplugged mid-session | Sources stop cleanly, state clears, back to waiting; the matrix hint returns after reconnect (unlock lost) |
| Vial protocol < v6 | Error naming the version found and the version needed |
| Reply timeout or mismatch | Retry once after 500 ms, then show the error |
| Another program holds the hidraw node (e.g. Vial GUI) | Every opener receives every reply, so the worker checks `/proc/*/fd` for other holders and pauses matrix polling while one exists ("paused: Vial is open") |
| `/dev/input` not accessible | Unfocused tier shows its hint; other tiers carry on |
| Layer state out of sync | Reload resets to layer 0 |
| Guard refusal | Command not sent; logged and shown as an error |

Logs go to stderr through `env_logger` (`RUST_LOG=debug`).

### Setup hints

All hint text lives in `hints`. A test asserts that the README contains each
rule and command verbatim, so the two can't drift apart. Hints give the exact
rule file, a `sudo tee` command to install it, and
`sudo udevadm control --reload-rules && sudo udevadm trigger`.

- **hidraw:** Vial's standard rule (serial match on `vial:f64c2b3c`, `uaccess`).
- **`/dev/input`:** `/etc/udev/rules.d/70-lily58-assistant.rules` containing
  `SUBSYSTEM=="input", KERNEL=="event*", ATTRS{idVendor}=="7171", ATTRS{idProduct}=="0012", TAG+="uaccess"`.
  The number must sort before `73-seat-late.rules`. Stated trade-off: any
  program running as the user can then read keystrokes from this keyboard.

## Testing

**Automated (no hardware):**
- Guard: exhaustive allowlist test (see above).
- `VialClient` against a scripted fake `Transport`: chunked definition read,
  keymap read, reply matching, timeout and retry, protocol-version rejection.
- `layout`: parse a fixture of the real Lily58 definition, captured once from
  the device with read-only commands and stored under `tests/fixtures/`.
- `keycodes`, `hostlayout`, `LayerTracker`, OS-keycode → position mapping:
  table-driven tests; `LayerTracker` with scripted matrix sequences.
- README/hints consistency test.

**Hardware diagnostics:** `lily58-assistant --probe` prints:
- device path and permissions
- VIA and Vial protocol versions
- layout summary
- the keymap, layer by layer
- unlock status
- whether `/dev/input` is accessible

It sends only allowlisted commands, through the same guard.

**Manual:** `docs/manual-test-checklist.md` covers highlights, each tier,
hotplug, unlock, Reload, the "Vial open" pause, and running on KDE and GNOME.

**CI:** `.github/workflows/ci.yml` runs `cargo build`, `cargo test` and
`cargo clippy` on the latest Ubuntu runner image. It's added now and takes
effect once the repo is on GitHub.

## README.md contents

1. What the app is; the read-only guarantee and the unlock exception.
2. Building on Fedora and on Ubuntu (rustup if the packaged Rust is too old;
   any `-dev` packages the smoke build shows are needed).
3. Permissions: the hidraw rule, the optional `/dev/input` rule with its
   trade-off, how to apply both.
4. Unlocking for layer tracking.
5. Configuration (`host_layout`, `tri_layer`).
6. Keeping the window on top by hand: KDE, Alt+F3 → More Actions → Keep Above;
   GNOME, Alt+Space → Always on Top.
7. Known limits (firmware-side layer logic, tap/hold approximation).
8. Troubleshooting, starting with `--probe`.

## Out of scope for v1

- Tutor mode (the architecture leaves room for it)
- Rotary encoders
- Keyboards other than Lily58-style Vial boards
- Caching the keymap between runs
- In-app always-on-top
- Any keymap writes, ever
