# Lily58 Assistant

> 🤖 **Vibe coded with [Claude Code](https://claude.com/claude-code).** Claude designed, planned and wrote this; a human steered and tested it on a real keyboard. Expect rough edges, and see the [open issues](https://github.com/KaiEkkrin/lily58_assistant/issues) for the known ones.

An on-screen companion for learning a Lily58 split keyboard running Vial firmware. It draws your keyboard, lights up the keys you press, shows what the last key typed, and follows layer changes as they happen. Everything is read from the keyboard itself.

**Strictly read-only.** The assistant never changes your keymap: every message it sends to the keyboard passes an allowlist of read commands (`src/hid/guard.rs`). The one exception is Vial's two-command unlock handshake, which enables live layer tracking and changes no keymap data. Use [Vial](https://get.vial.today) to remap.

## What it can see

| Tier | Needs | Shows |
|---|---|---|
| Focused window | nothing | keys typed into the assistant's own window |
| All windows | the optional `/dev/input` udev rule | keys typed anywhere; layer keys are invisible, so the layer is guessed |
| Live layers | unlocking the keyboard (each time it's plugged in) | every physical key, including LOWER/RAISE, with instant layer switching |

The status bar shows the active tier; **how to enable more…** opens these instructions in the app.

## Building

Needs Rust 1.95 or newer (`rustc --version`).

**Fedora**

```bash
sudo dnf install rust cargo gcc
cargo build --release
```

**Ubuntu**: if the packaged Rust is older than 1.95, use rustup:

```bash
sudo apt install build-essential curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo build --release
```

Run it with `./target/release/lily58-assistant`.

## Permissions

### Keyboard access (needed for the keyboard picture)

The assistant reads the layout and keymap over the keyboard's raw-HID interface, the same access Vial needs. If Vial works on this PC, you already have this. Otherwise:

```bash
echo 'KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{serial}=="*vial:f64c2b3c*", MODE="0660", GROUP="users", TAG+="uaccess", TAG+="udev-acl"' | sudo tee /etc/udev/rules.d/59-vial.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

### All-windows tracking (optional)

```bash
echo 'SUBSYSTEM=="input", KERNEL=="event*", ATTRS{idVendor}=="7171", ATTRS{idProduct}=="0012", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/70-lily58-assistant.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Then press **Reload** (Ctrl+R) in the assistant. Trade-off: with this rule in place, any program running as you can read keystrokes from the Lily58 (only that keyboard).

## Live layer tracking

Vial only reports the switch matrix after its physical unlock. In the assistant, open **how to enable more…**, click **Unlock for layer tracking**, and hold the highlighted keys until the bar fills (about 10 seconds). The keyboard forgets the unlock when it's unplugged.

An unlock can't be cancelled once started: until it completes, the keyboard answers nothing else over raw HID. To abort, unplug the keyboard. Vial behaves the same way.

## Typing tutor

Press **Ctrl+T** (or the button in the status bar) for drill mode. It only reads keys typed into
the assistant's own window, and it needs the keyboard connected, because every drill is built
from your actual keymap.

Seven drills select keys by position — home keys, stretch up, stretch down, the number row, the
outer column, the index reach, and Shift combinations — and five drill the punctuation you hit
writing Markdown, HTML, Rust, TypeScript or Elixir. Each batch is generated fresh, so you never
learn the text instead of the keys.

While it's open, every key is outlined in its finger's colour, and the next key to press is
highlighted — along with the Shift or layer key you need to hold to reach it, which is the part
a general-purpose typing tutor can't tell you about this keyboard. Both can be turned off in the
panel.

Remap something in Vial, close Vial, and the drills follow the change: the assistant re-reads
the keymap when the keyboard comes back.

## Configuration

Optional: `~/.config/lily58-assistant/config.toml` (or `$XDG_CONFIG_HOME/lily58-assistant/config.toml`):

```toml
# OS keyboard layout used to show characters: "gb" (default) or "us"
host_layout = "gb"

# Holding layers 1 and 2 shows layer 3, like the stock Lily58 firmware
# (update_tri_layer_state in keymap.c). Set to [] if your firmware doesn't do this.
tri_layer = [1, 2, 3]
```

## Keeping the window on top

The assistant doesn't do this itself, because Wayland offers no portable way. Use your desktop instead:

- **KDE Plasma:** Alt+F3 → More Actions → Keep Above Others
- **GNOME:** Alt+Space → Always on Top

## Known limits

- Layer logic compiled into the firmware isn't visible over Vial; `tri_layer` covers the common case.
- Tap/hold keys (e.g. `LT`) are approximated with QMK's default 200 ms tapping term.
- `TT` keys show their layer while held, but tapping one 5 times to toggle the layer on (QMK's default) isn't tracked.
- Without live layers, the position of a key typed elsewhere is a best guess: layer 0 first, then the other layers.
- While Vial or another program has the keyboard open, the assistant pauses and resumes when it's closed.

## Troubleshooting

```bash
./target/release/lily58-assistant --probe
```

Prints the device, permissions, protocol versions, layout, full keymap and unlock state, using only read commands. For logs, run with `RUST_LOG=debug`.

## License

MIT; see [LICENSE.md](LICENSE.md).
