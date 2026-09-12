# v1 Bug Rollup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the small, well-understood bugs filed after the v1 review (#1, #2, #3, #7, #8, #9) in one branch and one PR, leaving #4 open because it needs a hardware check first.

**Architecture:** No new modules. The fixes stay in the files the issues name. One small refactor comes first: `App::with_device` builds an `App` from channels instead of spawning the device worker, so UI state changes can be unit-tested without a display or keyboard. The #2 fix uses eframe's `App::raw_input_hook`, because egui handles Tab focus in `Memory::begin_pass` before `App::ui` runs.

**Tech Stack:** Rust 2024, eframe/egui 0.36.2, evdev 0.13.2, libc (for `poll(2)`), tempfile (tests only).

**Spec:** the GitHub issues [#1](https://github.com/KaiEkkrin/lily58_assistant/issues/1), [#2](https://github.com/KaiEkkrin/lily58_assistant/issues/2), [#3](https://github.com/KaiEkkrin/lily58_assistant/issues/3), [#7](https://github.com/KaiEkkrin/lily58_assistant/issues/7), [#8](https://github.com/KaiEkkrin/lily58_assistant/issues/8), [#9](https://github.com/KaiEkkrin/lily58_assistant/issues/9), plus `docs/implementation-notes.md` ("Known issues") and the v1 design spec `docs/superpowers/specs/2026-09-12-lily58-assistant-design.md`.

## Global Constraints

- Edition 2024, `rust-version = "1.95"`. **No new dependencies.** `libc` stays limited to `poll(2)`.
- Every byte sent to the keyboard still goes through `hid::guard::ReadOnlyGuard`. Don't touch the allowlist.
- `cargo test` must pass without the keyboard or a display. CI runs `cargo build --locked`, `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`; all three must pass after every task.
- **Do not run `cargo fmt`.** The code isn't in rustfmt's default style (lines run to about 130 characters), so it would reformat every file. Match the surrounding style by hand.
- egui tests that call `ctx.run_ui(...)` must call `.textures_delta.clear()` on the returned `FullOutput`. Otherwise epaint panics with "Dropped TexturesDelta with 1 unapplied deltas".
- Commit messages use the repo's prefixes (`fix:`, `docs:`, `chore:`) and name the issue, e.g. `fix: keys no longer stick after Alt+Tab (#1)`.
- Work on branch `fix/v1-bug-rollup`, off `main`.

## Triage

| Issue / item | Disposition | Task |
|---|---|---|
| #1 keys stick after Alt+Tab | Fix | 2 |
| #2 Tab/Space/Enter press the app's buttons | Fix (strip the keys in `raw_input_hook`) | 3 |
| #3 config errors vanish | Fix | 1 |
| #4 Vial may not detect the keyboard while unlocked | **Excluded.** Unconfirmed; needs a hardware check before choosing a fix | none |
| #7 vanish between discovery and first read | Fix: treat it as `Waiting` | 5 |
| #7 duplicate `Unlocked` | Fix: only `load()` reports it when the layout isn't loaded yet | 6 |
| #7 holder changes not re-reported while paused | Fix | 6 |
| #7 `last_failure` survives unplug | Fix in the worker (reset on `Waiting`) and the UI (clear the error on `Waiting`) | 5, 4 |
| #7 `spawn()` failure gives the UI no events | Fix in the UI: a closed event channel shows an error | 4 |
| #7 `Unlocking` sent on every poll | Fix: send only when the counter moves, and again after a resume | 6 |
| #8 `EINTR` cuts the read timeout short | Fix: retry with the time left | 7 |
| #8 short or interrupted write is a hard error | Retry on `EINTR`. A short write **stays an error**, because hidraw takes a whole report per `write(2)` and the rest can't be sent as a second write | 7 |
| #8 `find_vial_device` passes on `read_dir` errors | **No behaviour change.** Passing the error on is right: the worker reports it once. Document it and pin it with a test | 8 |
| #8 non-permission evdev open failures show as `NotFound` | Fix: new `EvdevStatus::Failed(String)` | 8 |
| #8 event nodes sort as text | Fix: numeric order | 8 |
| #9 "Paused" under a floating unlock window | Fix: the UI forgets the unlock state on `Paused`, `Waiting`, `NoAccess` and `Error` | 4 |
| #9 `device_tx.send` errors ignored | Fix: a failed send shows the same "worker stopped" error | 4 |
| #9 ✔/✖ glyphs unverified | **Verified, no change.** egui never uses system fonts, and both glyphs are in its bundled NotoEmoji-Regular and emoji-icon-font. A test pins it | 4 |

## File Structure

| File | Change |
|---|---|
| `src/config.rs` | `load_or_default`: logs the error and returns it for the UI |
| `src/ui/mod.rs` | `App::with_device` test seam; `config_error`; `typed` + `raw_input_hook`; focus-loss wiring; unlock/error resets; worker-stopped detection; tests |
| `src/ui/status.rs` | Shows `config_error` with a Dismiss button |
| `src/ui/dialogs.rs` | Shows `EvdevStatus::Failed`; glyph test |
| `src/input/mod.rs` | `OsSource` derives `Hash` |
| `src/input/focused.rs` | `FocusedInput::FocusLost`; `operates_widgets` |
| `src/state.rs` | Held OS keys keyed by `(OsSource, name)`; `release_focused_keys` |
| `src/device.rs` | Connect-time vanish, `last_failure` reset, holder changes, unlock event dedup; spawn-failure comment; tests |
| `src/hid/fake.rs` | `KeyboardSim::replug` |
| `src/hid/hidraw.rs` | `retry_interrupted` for `poll` and `write`; tests |
| `src/hid/discover.rs` | Doc comment and test for the `read_dir` error |
| `src/input/evdev.rs` | `EvdevStatus::Failed`; numeric node order; tests |
| `docs/manual-test-checklist.md` | Steps for Alt+Tab (#1) and Tab/Space/Enter (#2) |
| `docs/implementation-notes.md` | Known-issues list brought up to date; new egui facts |
| `Cargo.toml`, `Cargo.lock` | Version 1.0.1 |

---

### Task 1: Config errors stay on screen and reach the log (#3)

The config error goes in its own field, so device events can't clear it. This task also adds the `App::with_device` test seam that later tasks use.

**Files:**
- Modify: `src/config.rs` (new fn after `config_path`, new test)
- Modify: `src/ui/mod.rs:36-91` (`App` struct, `App::new`), plus a new `#[cfg(test)] mod tests` at the end
- Modify: `src/ui/status.rs:45-47`

**Interfaces:**
- Produces: `config::load_or_default(path: &Path) -> (Config, Option<String>)`.
- Produces: `App::with_device(config: &Config, config_error: Option<String>, ctx: egui::Context, device_tx: Sender<DeviceCommand>, device_rx: Receiver<DeviceEvent>) -> App`.
- Produces: `App.config_error: Option<String>`.
- Produces, in `ui::tests`: `fn app(config_error: Option<&str>) -> (App, Sender<DeviceEvent>, Receiver<DeviceCommand>)` and `fn connected() -> DeviceEvent`. Tasks 3 and 4 add tests that use them.

- [ ] **Step 1: Create the branch and commit this plan on it**

```bash
git checkout -b fix/v1-bug-rollup main
git add docs/superpowers/plans/2026-09-12-v1-bug-rollup.md
git commit -m "docs: plan for the v1 bug rollup"
```

- [ ] **Step 2: Write the failing tests**

Append to the `tests` module in `src/config.rs`:

```rust
    #[test]
    fn invalid_file_means_defaults_and_a_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "host_layout = \"fr\"\n").unwrap();
        let (config, error) = load_or_default(&path);
        assert_eq!(config, Config::default());
        let error = error.expect("an error message");
        assert!(error.starts_with("Config ignored, using defaults: "), "{error}");
        assert!(error.contains("config.toml"), "{error}");
        assert_eq!(load_or_default(Path::new("/nonexistent/lily58-assistant.toml")), (Config::default(), None));
    }
```

Append to the end of `src/ui/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceInfo;
    use crate::hid::fake::{SMALL_DEFINITION, SMALL_KEYMAP};
    use crate::keymap::Keymap;
    use crate::layout::Layout;

    /// An `App` wired to channels instead of a device worker.
    fn app(config_error: Option<&str>) -> (App, Sender<DeviceEvent>, Receiver<DeviceCommand>) {
        let (events_tx, device_rx) = mpsc::channel();
        let (device_tx, commands_rx) = mpsc::channel();
        let app =
            App::with_device(&Config::default(), config_error.map(String::from), egui::Context::default(), device_tx, device_rx);
        (app, events_tx, commands_rx)
    }

    /// `Connected` for the 2x3 test keyboard. Its `usb_dir` matches no real device, so no evdev reader starts.
    fn connected() -> DeviceEvent {
        let layout = Layout::from_definition(&serde_json::from_str(SMALL_DEFINITION).unwrap()).unwrap();
        let buf: Vec<u8> = SMALL_KEYMAP.iter().flat_map(|c| c.to_be_bytes()).collect();
        let info = DeviceInfo {
            dev_node: "/dev/hidraw99".into(),
            usb_dir: "/nonexistent/usb".into(),
            product: "Sim58".into(),
            via_protocol: 12,
            vial_protocol: 6,
        };
        DeviceEvent::Connected { info, layout, keymap: Keymap::from_buffer(2, 2, 3, &buf).unwrap() }
    }

    #[test]
    fn config_error_outlives_device_events() {
        let (mut app, _events, _commands) = app(Some("Config ignored, using defaults: bad"));
        app.on_device_event(DeviceEvent::Error("boom".into()), Instant::now());
        app.on_device_event(connected(), Instant::now());
        assert_eq!(app.error, None, "Connected clears device errors");
        assert_eq!(app.config_error.as_deref(), Some("Config ignored, using defaults: bad"));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib config:: ui::`
Expected: compile errors: "cannot find function `load_or_default`", "no function or associated item named `with_device`", "no field `config_error`".

- [ ] **Step 4: Implement `load_or_default`**

In `src/config.rs`, after `config_path`:

```rust
/// Loads the config, falling back to defaults if it is unreadable or invalid. The error is
/// logged and returned so the UI can show it until dismissed.
pub fn load_or_default(path: &Path) -> (Config, Option<String>) {
    match Config::load_from(path) {
        Ok(config) => (config, None),
        Err(e) => {
            log::warn!("config ignored, using defaults: {e}");
            (Config::default(), Some(format!("Config ignored, using defaults: {e}")))
        }
    }
}
```

- [ ] **Step 5: Add `config_error` and the `with_device` seam**

In `src/ui/mod.rs`, add a field after `error: Option<String>,`:

```rust
    /// Kept apart from `error`, which device events clear; this stays until dismissed.
    config_error: Option<String>,
```

Replace `App::new` (lines 65-91) with:

```rust
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (config, config_error) = config::load_or_default(&config::config_path());
        let ctx = cc.egui_ctx.clone();
        let (events_tx, device_rx) = mpsc::channel();
        let repaint = ctx.clone();
        let device_tx = device::spawn(SystemConnector, events_tx, move || repaint.request_repaint());
        Self::with_device(&config, config_error, ctx, device_tx, device_rx)
    }

    /// Everything but starting the device worker, so tests can stand in for it with channels.
    fn with_device(
        config: &Config,
        config_error: Option<String>,
        ctx: egui::Context,
        device_tx: Sender<DeviceCommand>,
        device_rx: Receiver<DeviceEvent>,
    ) -> Self {
        let (tri, always_tri) = config.tri();
        let (input_tx, input_rx) = mpsc::channel();
        Self {
            state: AppState::new(config.host_layout, tri, always_tri),
            connection: Connection::Waiting,
            unlock: Unlock::Unknown,
            unlock_keys: Vec::new(),
            evdev: EvdevStatus::NotFound,
            evdev_usb_dir: None,
            error: None,
            config_error,
            show_hints: false,
            device_rx,
            device_tx,
            input_tx,
            input_rx,
            ctx,
        }
    }
```

- [ ] **Step 6: Show the config error with a Dismiss button**

In `src/ui/status.rs`, replace lines 45-47 (the `if let Some(err) = &app.error` block) with:

```rust
    if let Some(err) = &app.error {
        ui.colored_label(Color32::from_rgb(230, 90, 90), err);
    }
    let mut dismissed = false;
    if let Some(err) = &app.config_error {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(Color32::from_rgb(230, 180, 40), err);
            dismissed = ui.small_button("Dismiss").clicked();
        });
    }
    if dismissed {
        app.config_error = None;
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib config:: ui::`
Expected: PASS, including `invalid_file_means_defaults_and_a_message` and `config_error_outlives_device_events`.

- [ ] **Step 8: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all tests pass, no clippy warnings.

- [ ] **Step 9: Commit**

```bash
git add src/config.rs src/ui/mod.rs src/ui/status.rs
git commit -m "fix: config errors stay on screen until dismissed and are logged (#3)"
```

---

### Task 2: Keys no longer stick after Alt+Tab (#1)

When the window loses focus, forget the keys the focused window saw go down. Wayland never delivers their releases.

**Files:**
- Modify: `src/input/mod.rs:9` (derive `Hash` on `OsSource`)
- Modify: `src/input/focused.rs` (new variant and match arm; update the existing test)
- Modify: `src/state.rs:51-52` (`os_held` key), `:151-178` (`os_key`), new method after `on_text`, new tests
- Modify: `src/ui/mod.rs` (the `focused::translate` match in `ui`)
- Modify: `docs/manual-test-checklist.md` ("Focused window" section)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `FocusedInput::FocusLost` (new variant of `pub enum FocusedInput`).
- Produces: `AppState::release_focused_keys(&mut self)`.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `src/state.rs`:

```rust
    fn focused(name: &str, usages: &[u8], pressed: bool) -> OsKey {
        OsKey { source: OsSource::Focused, usages: usages.to_vec(), pressed, name: name.into() }
    }

    #[test]
    fn focus_loss_forgets_keys_held_in_the_window() {
        let mut s = state();
        let now = Instant::now();
        s.os_key(&focused("ShiftLeft", &[0xE1], true), now); // LSFT is at (1,2)
        s.os_key(&focused("B", &[0x05], true), now);
        assert_eq!(s.held_positions(), BTreeSet::from([(0, 1), (1, 2)]));
        s.release_focused_keys(); // Alt+Tab: the releases never arrive
        assert!(s.held_positions().is_empty());
        s.os_key(&focused("Num3", &[0x20], true), now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("3"), "shift is no longer held");
    }

    #[test]
    fn focus_loss_keeps_keys_seen_by_evdev() {
        let mut s = state();
        s.evdev_active = true;
        let now = Instant::now();
        s.os_key(&os("evdev 42", &[0xE1], true), now);
        s.release_focused_keys();
        assert_eq!(s.held_positions(), BTreeSet::from([(1, 2)]));
        s.os_key(&os("evdev 4", &[0x20], true), now);
        assert_eq!(s.last.as_ref().unwrap().text.as_deref(), Some("£"), "evdev still has shift down");
    }
```

In `src/input/focused.rs`, change the existing test's `FocusedInput::Text(_) => panic!("expected a key"),` arm to `_ => panic!("expected a key"),`, then append:

```rust
    #[test]
    fn focus_loss_is_reported() {
        let out = translate(&[Event::WindowFocused(true), Event::WindowFocused(false)]);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], FocusedInput::FocusLost));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib state:: focused::`
Expected: compile errors: "no variant named `FocusLost`", "no method named `release_focused_keys`".

- [ ] **Step 3: Implement**

`src/input/mod.rs`: change `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` on `OsSource` to `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`.

`src/input/focused.rs`: add the variant, then the match arm after the `Event::Text` arm:

```rust
pub enum FocusedInput {
    Key(OsKey),
    Text(String),
    /// The window lost keyboard focus, so releases of keys held now won't arrive.
    FocusLost,
}
```

```rust
            Event::WindowFocused(false) => Some(FocusedInput::FocusLost),
```

`src/state.rs`: change the import to `use crate::input::{OsKey, OsSource};`, then change the field:

```rust
    /// OS keys currently down (by source and `OsKey::name`) and where we placed them.
    os_held: HashMap<(OsSource, String), Option<(u8, u8)>>,
```

In `os_key`, replace:

```rust
        if !key.pressed {
            self.os_held.remove(&key.name);
            return;
        }
```

with:

```rust
        let held = (key.source, key.name.clone());
        if !key.pressed {
            self.os_held.remove(&held);
            return;
        }
```

Then replace `self.os_held.insert(key.name.clone(), hit.map(|h| (h.row, h.col)));` with `self.os_held.insert(held, hit.map(|h| (h.row, h.col)));`.

Add after `on_text`:

```rust
    /// The window lost focus (e.g. Alt+Tab). On Wayland it never sees the release of a key held
    /// at that moment, so forget the keys it saw go down, and the shift state if it came from them.
    pub fn release_focused_keys(&mut self) {
        self.os_held.retain(|(source, _), _| *source != OsSource::Focused);
        if !self.evdev_active {
            self.os_shift = false; // the UI feeds focused keys to `os_key` only while evdev is off
        }
    }
```

`src/ui/mod.rs`, in `ui`: add an arm to the `focused::translate` match:

```rust
                FocusedInput::FocusLost => self.state.release_focused_keys(),
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib state:: focused::`
Expected: PASS.

- [ ] **Step 5: Add the manual check**

In `docs/manual-test-checklist.md`, append to the "Focused window (no evdev rule, keyboard locked)" section:

```markdown
- [ ] Hold Shift, Alt+Tab to another window, release both there, then come back: no key stays highlighted, and typing 3 shows `3`, not `£` (#1).
```

- [ ] **Step 6: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/input/mod.rs src/input/focused.rs src/state.rs src/ui/mod.rs docs/manual-test-checklist.md
git commit -m "fix: keys no longer stay highlighted after Alt+Tab (#1)"
```

---

### Task 3: Typing Tab, Space or Enter no longer presses the app's buttons (#2)

egui moves focus on Tab in `Memory::begin_pass`, before `App::ui` runs. Removing the events inside `ui` would be too late. eframe's `raw_input_hook` runs before the pass, so it copies the events for the focused tier and then strips Tab/Space/Enter key events from what egui sees. A throwaway test during planning confirmed that Tab, Space, Enter on a plain `egui::Button` gives 2 clicks without the strip and 0 with it.

**Files:**
- Modify: `src/input/focused.rs` (new `operates_widgets`)
- Modify: `src/ui/mod.rs` (`typed` field, `raw_input_hook`, `ui` reads `typed`, new test)
- Modify: `docs/manual-test-checklist.md`

**Interfaces:**
- Consumes: `ui::tests::app` (Task 1); `FocusedInput::FocusLost` arm (Task 2) stays as is.
- Produces: `focused::operates_widgets(event: &Event) -> bool`; `App.typed: Vec<egui::Event>`.

- [ ] **Step 1: Write the failing test**

Append to `ui::tests` in `src/ui/mod.rs`:

```rust
    #[test]
    fn tab_space_and_enter_reach_the_tracker_but_not_the_buttons() {
        let (mut app, _events, _commands) = app(None);
        let ctx = egui::Context::default();
        let key = |k: egui::Key| egui::Event::Key {
            key: k,
            physical_key: Some(k),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let (mut clicks, mut typed) = (0, Vec::new());
        // Unfiltered, Tab focuses the button and Space and Enter each press it.
        let frames = [
            vec![],
            vec![key(egui::Key::Tab)],
            vec![],
            vec![key(egui::Key::Space)],
            vec![],
            vec![key(egui::Key::Enter)],
            vec![],
        ];
        for events in frames {
            let mut raw = egui::RawInput { events, ..Default::default() };
            eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
            typed.append(&mut app.typed);
            ctx.run_ui(raw, |ui| {
                if ui.button("Unlock for layer tracking").clicked() {
                    clicks += 1;
                }
            })
            .textures_delta
            .clear();
        }
        assert_eq!(clicks, 0);
        assert_eq!(typed, vec![key(egui::Key::Tab), key(egui::Key::Space), key(egui::Key::Enter)]);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib ui::tests::tab_space`
Expected: compile error "no field `typed`". The same test without the hook would count 2 clicks.

- [ ] **Step 3: Implement**

`src/input/focused.rs`, after `translate`:

```rust
/// Keys egui uses to move keyboard focus between widgets (Tab) and to press the focused one
/// (Space, Enter).
pub fn operates_widgets(event: &Event) -> bool {
    matches!(event, Event::Key { key: Key::Tab | Key::Space | Key::Enter, .. })
}
```

`src/ui/mod.rs`: add a field to `App` after `show_hints`:

```rust
    /// Everything typed into the window this frame, copied in `raw_input_hook` before egui
    /// sees it.
    typed: Vec<egui::Event>,
```

and `typed: Vec::new(),` in `with_device`'s struct literal after `show_hints: false,`.

In `impl eframe::App for App`, add before `fn ui`:

```rust
    /// The focused tier reads everything typed into the window. Tab, Space and Enter would also
    /// move keyboard focus onto the app's buttons and press them, including the unlock, which
    /// can't be cancelled. egui acts on Tab before `ui` runs, so the keys are taken out here.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.typed.extend(raw_input.events.iter().cloned());
        raw_input.events.retain(|e| !focused::operates_widgets(e));
    }
```

In `ui`, replace:

```rust
        let events = ui.ctx().input(|i| i.events.clone());
        for input in focused::translate(&events) {
```

with:

```rust
        for input in focused::translate(&std::mem::take(&mut self.typed)) {
```

This also stops a second egui pass in the same frame from counting the same key twice, because `typed` is drained on the first pass.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test --lib ui::`
Expected: PASS.

- [ ] **Step 5: Add the manual check**

In `docs/manual-test-checklist.md`, append to the "Focused window" section:

```markdown
- [ ] With the "Tracking tiers" window open, type Tab, Space and Enter several times: no button is pressed and no unlock starts, yet Space and Enter still highlight their keys (#2).
```

- [ ] **Step 6: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/input/focused.rs src/ui/mod.rs docs/manual-test-checklist.md
git commit -m "fix: Tab, Space and Enter typed into the window no longer press buttons (#2)"
```

---

### Task 4: UI device state stays consistent (#9, and #7's spawn failure and stale error)

Three changes, plus one test:

- The UI forgets the unlock state whenever there is no live session (`Waiting`, `NoAccess`, `Paused`, `Error`), and clears the device error on `Waiting`.
- A closed device channel means the worker has stopped. That covers a panic and a thread that never started (#7). The UI clears the keyboard and shows an error. A failed command send does the same (#9).
- A test pins the ✔/✖ glyphs (#9).

**Files:**
- Modify: `src/ui/mod.rs` (import, const, field, `on_device_event` arms, new `drain_device_events` / `on_worker_stopped` / `send`, `reload`, `start_unlock`, `ui`, tests)
- Modify: `src/ui/dialogs.rs` (new test module)
- Modify: `src/device.rs:399-401` (comment only)

**Interfaces:**
- Consumes: `ui::tests::{app, connected}` (Task 1).
- Produces: `const DEVICE_WORKER_STOPPED: &str` in `ui/mod.rs`; `App::drain_device_events(&mut self, now: Instant)`.
- Relied on by Task 6: after `Paused`, the UI's unlock state is `Unknown`, so the worker must re-send `Unlocking` after a resume.

- [ ] **Step 1: Write the failing tests**

Append to `ui::tests` in `src/ui/mod.rs`:

```rust
    #[test]
    fn paused_hides_an_unlock_in_progress() {
        let (mut app, _events, _commands) = app(None);
        let now = Instant::now();
        app.on_device_event(DeviceEvent::Unlocking { counter: 50, unlock_keys: vec![(1, 0), (1, 2)] }, now);
        assert!(!app.unlock_highlight().is_empty());
        app.on_device_event(DeviceEvent::Paused { holders: vec!["vial (7)".into()] }, now);
        assert_eq!(app.unlock, Unlock::Unknown, "no unlock window over \"Paused\"");
        assert!(app.unlock_highlight().is_empty());
    }

    #[test]
    fn losing_the_session_forgets_the_unlock_and_waiting_clears_the_error() {
        let (mut app, _events, _commands) = app(None);
        let now = Instant::now();
        app.on_device_event(DeviceEvent::Unlocking { counter: 40, unlock_keys: vec![] }, now);
        app.on_device_event(DeviceEvent::Error("keyboard sent an unexpected reply: x".into()), now);
        assert_eq!(app.unlock, Unlock::Unknown);
        assert!(app.error.is_some());
        app.on_device_event(DeviceEvent::Waiting, now);
        assert_eq!(app.error, None, "an error about a keyboard that's gone is stale");
    }

    #[test]
    fn a_stopped_device_worker_is_reported() {
        let (mut app, events, _commands) = app(None);
        app.on_device_event(connected(), Instant::now());
        drop(events); // the worker thread ended, or never started
        app.drain_device_events(Instant::now());
        assert_eq!(app.error.as_deref(), Some(DEVICE_WORKER_STOPPED));
        assert!(app.state.layout.is_none(), "the picture can no longer update, so it goes");
    }

    #[test]
    fn commands_to_a_stopped_device_worker_are_reported() {
        let (mut app, _events, commands) = app(None);
        drop(commands);
        app.start_unlock();
        assert_eq!(app.error.as_deref(), Some(DEVICE_WORKER_STOPPED));
    }
```

Append to `src/ui/dialogs.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_and_cross_are_in_eguis_bundled_fonts() {
        // egui draws only its bundled fonts, never the desktop's, so this holds on every desktop.
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear(); // loads the fonts
        let font = egui::FontId::proportional(14.0);
        // epaint 0.36's `has_glyph` wrongly says false for anything in its emoji fonts; a
        // missing glyph shows up as zero width instead.
        let width = |c: char| ctx.fonts_mut(|f| f.glyph_width(&font, c));
        assert_eq!(width('\u{10FFFD}'), 0.0, "control: a character no font has");
        for c in mark(true).chars().chain(mark(false).chars()) {
            assert!(width(c) > 0.0, "{c:?} has no glyph");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib ui::`
Expected: compile errors for `drain_device_events` and `DEVICE_WORKER_STOPPED`. The glyph test compiles and passes already. It records a check that was done, not a bug.

- [ ] **Step 3: Implement**

In `src/ui/mod.rs`:

Change the import to `use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};`.

After the `Unlock` enum, add:

```rust
/// Shown when the device worker has stopped (it panicked, or its thread never started).
const DEVICE_WORKER_STOPPED: &str =
    "The keyboard worker has stopped, so the keyboard can't be read. Restart the assistant; the log says why.";
```

Add a field to `App` after `config_error`:

```rust
    /// The device worker's channel has closed; reported once.
    worker_stopped: bool,
```

and `worker_stopped: false,` in `with_device` after `config_error,`.

Replace these arms of `on_device_event`:

```rust
            DeviceEvent::Waiting => self.connection = Connection::Waiting,
            DeviceEvent::NoAccess(path) => self.connection = Connection::NoAccess(path),
            DeviceEvent::Paused { holders } => {
                self.connection = Connection::Paused(holders);
                self.state.set_matrix_active(false);
            }
```

with:

```rust
            DeviceEvent::Waiting => {
                self.connection = Connection::Waiting;
                self.unlock = Unlock::Unknown;
                self.error = None; // no keyboard, so any error about it is stale
            }
            DeviceEvent::NoAccess(path) => {
                self.connection = Connection::NoAccess(path);
                self.unlock = Unlock::Unknown;
            }
            DeviceEvent::Paused { holders } => {
                self.connection = Connection::Paused(holders);
                // The other program may lock, unlock or finish an unlock; a resume re-reports it.
                self.unlock = Unlock::Unknown;
                self.state.set_matrix_active(false);
            }
```

and replace `DeviceEvent::Error(e) => self.error = Some(e),` with:

```rust
            DeviceEvent::Error(e) => {
                self.error = Some(e);
                self.unlock = Unlock::Unknown; // errors come only when there is no session
            }
```

After `on_device_event`, add:

```rust
    fn drain_device_events(&mut self, now: Instant) {
        loop {
            match self.device_rx.try_recv() {
                Ok(event) => self.on_device_event(event, now),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.on_worker_stopped();
                    return;
                }
            }
        }
    }

    /// The device worker's end of the channels is gone. Nothing about the keyboard will change
    /// again, so clear it and say so.
    fn on_worker_stopped(&mut self) {
        if self.worker_stopped {
            return;
        }
        self.worker_stopped = true;
        log::error!("the device worker has stopped");
        self.on_device_event(DeviceEvent::Disconnected, Instant::now());
        self.error = Some(DEVICE_WORKER_STOPPED.into());
    }

    fn send(&mut self, cmd: DeviceCommand) {
        if self.device_tx.send(cmd).is_err() {
            self.on_worker_stopped();
        }
    }
```

In `reload`, replace `let _ = self.device_tx.send(DeviceCommand::Reload);` with `self.send(DeviceCommand::Reload);`. In `start_unlock`, replace `let _ = self.device_tx.send(DeviceCommand::StartUnlock);` with `self.send(DeviceCommand::StartUnlock);`.

In `ui`, replace:

```rust
        while let Ok(event) = self.device_rx.try_recv() {
            self.on_device_event(event, now);
        }
```

with `self.drain_device_events(now);`.

In `src/device.rs`, change the end of `spawn` to:

```rust
    if let Err(e) = spawned {
        // The closure, and the event sender with it, is dropped, so the UI sees the channel close.
        log::error!("cannot start the device thread: {e}");
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib ui::`
Expected: PASS.

- [ ] **Step 5: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/ui/mod.rs src/ui/dialogs.rs src/device.rs
git commit -m "fix: UI forgets stale unlock state and errors, and reports a stopped worker (#9, #7)"
```

---

### Task 5: Worker connection lifecycle edge cases (#7)

Two changes. A keyboard unplugged between discovery and the first reply counts as not there, not as an error. And `last_failure` is reset when the keyboard is gone, so the same failure is reported again after a replug.

**Files:**
- Modify: `src/device.rs:195-225` (`try_connect`), tests (`FakeConnector`, `Harness::build`, new tests)
- Modify: `src/hid/fake.rs` (new `KeyboardSim::replug` after `unplug`)

**Interfaces:**
- Produces: `KeyboardSim::replug(&self)` (tests only); `FakeConnector.vanish_on_open: bool` (tests only).

- [ ] **Step 1: Write the failing tests**

In `src/hid/fake.rs`, after `unplug`:

```rust
    /// Plugs the keyboard back in. Unlike real hardware, its state (e.g. unlocked) carries over.
    pub fn replug(&self) {
        *self.unplugged.lock().unwrap() = false;
    }
```

In `src/device.rs` tests, add a field to `FakeConnector`:

```rust
        /// Unplug the keyboard as it is opened: it vanishes between discovery and the first read.
        vanish_on_open: bool,
```

In `FakeConnector::open`, after the `if self.deny { ... }` block:

```rust
            if self.vanish_on_open {
                self.sim.unplug();
            }
```

In `Harness::build`, construct it as `FakeConnector { sim: sim.clone(), deny, vanish_on_open: false, holders: Arc::clone(&holders) }`.

Append the tests:

```rust
    #[test]
    fn keyboard_vanishing_before_the_first_reply_is_not_an_error() {
        let mut h = Harness::new(KeyboardSim::small());
        h.worker.connector.vanish_on_open = true;
        assert_eq!(names(&h.steps(2, 0)), ["Waiting"]);
    }

    #[test]
    fn an_identical_error_is_reported_again_after_replug() {
        let bad_definition = r#"{"matrix":{"rows":2,"cols":3},"layouts":{"keymap":[]}}"#;
        let sim = KeyboardSim::new(bad_definition, 2, 2, 3, &SMALL_KEYMAP, &[(1, 0), (1, 2)]);
        let mut h = Harness::new(sim);
        assert_eq!(names(&h.steps(2, 0)), ["Error"]);
        h.sim.unplug();
        assert_eq!(names(&h.steps(1, 5_000)), ["Waiting"]);
        h.sim.replug();
        assert_eq!(names(&h.steps(2, 6_000)), ["Error"], "a new keyboard gets its error shown");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib device::`
Expected: `keyboard_vanishing_before_the_first_reply_is_not_an_error` FAILS with `["Error"]` (the old code reports the I/O error). `an_identical_error_is_reported_again_after_replug` FAILS on its last assertion with `[]`.

- [ ] **Step 3: Implement**

In `try_connect`, replace:

```rust
            Ok(None) => DeviceEvent::Waiting,
```

with:

```rust
            Ok(None) => {
                self.last_failure = None; // a keyboard plugged in later gets its errors shown
                DeviceEvent::Waiting
            }
```

and replace:

```rust
                        Ok(guard) => match start_session(dev, guard, now) {
                            Ok(session) => {
                                self.conn = Conn::Connected(Box::new(session));
                                return Duration::ZERO;
                            }
                            Err(e) => DeviceEvent::Error(e.to_string()),
                        },
```

with:

```rust
                        Ok(guard) => match start_session(dev, guard, now) {
                            Ok(session) => {
                                self.conn = Conn::Connected(Box::new(session));
                                return Duration::ZERO;
                            }
                            // Unplugged between discovery and the first reply.
                            Err(e) if e.is_disconnect() => {
                                log::info!("keyboard went away while connecting: {e}");
                                self.last_failure = None;
                                DeviceEvent::Waiting
                            }
                            Err(e) => DeviceEvent::Error(e.to_string()),
                        },
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib device::`
Expected: PASS (all device tests).

- [ ] **Step 5: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/device.rs src/hid/fake.rs
git commit -m "fix: a keyboard vanishing mid-connect is not an error; errors re-show after replug (#7)"
```

---

### Task 6: Worker sends each state change once (#7)

Three changes:

- `Unlocked` is sent once. When the layout isn't loaded yet, `load()` reports it along with `Connected`.
- A change in *which* programs hold the keyboard is re-sent as `Paused`.
- `Unlocking` is sent only when the counter moves, and again after a resume, because the UI forgets the unlock on `Paused` (Task 4).

**Files:**
- Modify: `src/device.rs`: `Session` (lines 93-114; `paused: bool` becomes `holders: Vec<String>`, new `unlock_counter_sent`), `handle` (line 169), `start_session` (lines 267-282), `poll` (lines 286-321), tests

**Interfaces:**
- Consumes: Task 4's UI behaviour (unlock state is `Unknown` after `Paused`).
- Produces: no public API change. The event order is now `Unlocking…, Connected, Unlocked` when an unlock left by another program completes before the first load (it used to be `Unlocking…, Unlocked, Connected, Unlocked`).

- [ ] **Step 1: Write the failing tests, and update the tests whose expected events change**

Append to `src/device.rs` tests:

```rust
    #[test]
    fn a_change_of_holders_while_paused_is_reported() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        *h.holders.lock().unwrap() = vec!["vial (4242)".into()];
        assert_eq!(h.steps(1, 1000), vec![DeviceEvent::Paused { holders: vec!["vial (4242)".into()] }]);
        *h.holders.lock().unwrap() = vec!["vial (4242)".into(), "vial (4343)".into()];
        assert_eq!(h.steps(1, 2000), vec![DeviceEvent::Paused { holders: vec!["vial (4242)".into(), "vial (4343)".into()] }]);
        assert!(h.steps(1, 3000).is_empty(), "unchanged holders are not re-sent");
    }

    #[test]
    fn unlock_progress_is_sent_only_when_it_moves() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.worker.handle(DeviceCommand::StartUnlock);
        assert_eq!(names(&h.steps(1, 0)), ["Unlocking"]); // counter 50: keys not held
        let before = h.sim.with(|s| s.requests);
        assert!(h.steps(1, 200).is_empty(), "still 50: nothing new to show");
        assert_eq!(h.sim.with(|s| s.requests), before + 1, "but it did poll");
        h.sim.with(|s| s.matrix[1] = 0b101); // the user holds both unlock keys
        assert_eq!(h.steps(1, 400), vec![DeviceEvent::Unlocking { counter: 49, unlock_keys: vec![(1, 0), (1, 2)] }]);
    }

    #[test]
    fn unlock_progress_is_sent_again_after_a_pause() {
        let mut h = Harness::new(KeyboardSim::small());
        h.steps(2, 0);
        h.worker.handle(DeviceCommand::StartUnlock);
        assert_eq!(names(&h.steps(1, 0)), ["Unlocking"]);
        *h.holders.lock().unwrap() = vec!["vial (7)".into()];
        assert_eq!(names(&h.steps(1, 1000)), ["Paused"]);
        h.holders.lock().unwrap().clear();
        // The UI forgot the unlock when it saw Paused, so the unchanged counter is sent again.
        assert_eq!(names(&h.steps(1, 2000)), ["Resumed", "Unlocking"]);
    }
```

In `early_wakeups_do_not_poll_the_unlock_too_soon`, replace the last line:

```rust
        assert_eq!(names(&h.steps(1, UNLOCK_POLL_INTERVAL.as_millis() as u64)), ["Unlocking"]);
```

with:

```rust
        assert!(h.steps(1, UNLOCK_POLL_INTERVAL.as_millis() as u64).is_empty(), "counter unchanged: nothing to send");
        assert_eq!(h.sim.with(|s| s.requests), before + 1, "one unlock poll once the interval has passed");
```

In `unlock_left_in_progress_defers_keymap_reads`, replace:

```rust
        let n = names(&events);
        let unlocked = n.iter().position(|&e| e == "Unlocked").expect("unlocked");
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..unlocked].iter().all(|&e| e == "Unlocking"));
        assert!(connected > unlocked);
```

with:

```rust
        let n = names(&events);
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..connected].iter().all(|&e| e == "Unlocking"), "{n:?}");
        assert_eq!(n[connected + 1], "Unlocked", "{n:?}");
        assert_eq!(n.iter().filter(|&&e| e == "Unlocked").count(), 1, "{n:?}");
```

In `resume_into_unlock_in_progress_waits_for_the_unlock`, replace:

```rust
        let unlocked = n.iter().position(|&e| e == "Unlocked").expect("unlocked");
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..unlocked].iter().all(|&e| e == "Resumed" || e == "Unlocking"), "{n:?}");
        assert!(connected > unlocked);
```

with:

```rust
        let connected = n.iter().position(|&e| e == "Connected").expect("connected");
        assert!(n[..connected].iter().all(|&e| e == "Resumed" || e == "Unlocking"), "{n:?}");
        assert_eq!(n[connected + 1], "Unlocked", "{n:?}");
        assert_eq!(n.iter().filter(|&&e| e == "Unlocked").count(), 1, "{n:?}");
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib device::`
Expected failures:
- `a_change_of_holders_while_paused_is_reported`: the second `Paused` is missing.
- `unlock_progress_is_sent_only_when_it_moves` and `early_wakeups_…`: an unwanted `Unlocking`.
- `unlock_left_in_progress_…` and `resume_into_unlock_…`: `Unlocked` is sent twice.
- `unlock_progress_is_sent_again_after_a_pause` passes before the change (every poll sends today). It guards the reset added in Step 3.

- [ ] **Step 3: Implement**

In `Session`, replace the field `paused: bool,` with:

```rust
    /// Other programs holding the device, as last reported; non-empty means paused.
    holders: Vec<String>,
    /// The unlock counter last sent to the UI, so a poll that finds it unchanged sends nothing.
    unlock_counter_sent: Option<u8>,
```

In `start_session`'s struct literal, replace `paused: false,` with:

```rust
        holders: Vec::new(),
        unlock_counter_sent: None,
```

In `handle`, replace `DeviceCommand::StartUnlock if s.lock == Lock::Locked && !s.paused => {` with `DeviceCommand::StartUnlock if s.lock == Lock::Locked && s.holders.is_empty() => {`.

In `poll`, replace everything from `if now >= s.next_holder_check {` through the end of the `if s.lock == Lock::Unlocking { … }` block (lines 286-321) with:

```rust
    if now >= s.next_holder_check {
        s.next_holder_check = now + HOLDER_CHECK_INTERVAL;
        let holders = connector.other_holders(&s.dev);
        if holders != s.holders {
            s.holders = holders;
            if !s.holders.is_empty() {
                // Sent again whenever the set of other programs changes while paused.
                out.push(DeviceEvent::Paused { holders: s.holders.clone() });
            } else {
                // The other program may have locked, unlocked, or changed the keymap while it
                // held the device: re-read the lock state before anything else runs, so the
                // Unlocking branch below sees it and no VIA read is sent to a keyboard that
                // is mid-handshake or newly locked.
                let status = s.client.unlock_status()?;
                s.unlock_keys = status.keys;
                s.lock = lock_from_status(status.unlocked, status.in_progress);
                s.loaded = false;
                // The UI forgot the unlock state when it saw `Paused`, so send progress afresh.
                s.unlock_counter_sent = None;
                out.push(DeviceEvent::Resumed);
            }
        }
    }
    if !s.holders.is_empty() {
        return Ok(HOLDER_CHECK_INTERVAL);
    }

    if s.lock == Lock::Unlocking {
        if now < s.next_unlock_poll {
            return Ok(s.next_unlock_poll - now);
        }
        s.next_unlock_poll = now + UNLOCK_POLL_INTERVAL;
        let p = s.client.unlock_poll()?;
        if !p.unlocked {
            if s.unlock_counter_sent != Some(p.counter) {
                s.unlock_counter_sent = Some(p.counter);
                out.push(DeviceEvent::Unlocking { counter: p.counter, unlock_keys: s.unlock_keys.clone() });
            }
            return Ok(UNLOCK_POLL_INTERVAL);
        }
        s.lock = Lock::Unlocked;
        s.unlock_counter_sent = None;
        // Before the first load, `load()` below sends `Unlocked` along with `Connected`.
        if s.loaded {
            out.push(DeviceEvent::Unlocked);
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib device::`
Expected: PASS, all device tests, including the unchanged `unlock_handshake_then_matrix`, `pauses_while_another_program_holds_the_device` and `resume_rereads_lock_state_after_vial_locks`.

- [ ] **Step 5: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/device.rs
git commit -m "fix: device worker sends Unlocked once, re-sends holder changes, skips unchanged unlock polls (#7)"
```

---

### Task 7: hidraw reads and writes survive EINTR (#8)

`poll(2)` interrupted by a signal is retried with the time left, instead of returning "no report" early, which costs the caller a 500 ms retry. An interrupted `write(2)` sent nothing and is retried. A short write stays an error, with a comment explaining why.

**Files:**
- Modify: `src/hid/hidraw.rs` (imports, `write_report`, `read_report`, new `retry_interrupted`, new tests)

**Interfaces:**
- Produces: private `fn retry_interrupted<T>(op: impl FnMut() -> io::Result<T>) -> io::Result<T>`.

- [ ] **Step 1: Write the failing tests**

Append to `src/hid/hidraw.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::OwnedFd;

    #[test]
    fn interrupted_calls_are_retried() {
        let mut calls = 0;
        let result = retry_interrupted(|| {
            calls += 1;
            if calls < 3 { Err(io::ErrorKind::Interrupted.into()) } else { Ok(calls) }
        });
        assert_eq!(result.unwrap(), 3);
    }

    #[test]
    fn other_errors_are_returned_at_once() {
        let mut calls = 0;
        let result: io::Result<()> = retry_interrupted(|| {
            calls += 1;
            Err(io::ErrorKind::BrokenPipe.into())
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(calls, 1);
    }

    #[test]
    fn read_report_waits_out_the_timeout_then_reads_a_report() {
        let (reader, mut writer) = io::pipe().unwrap();
        let mut hidraw = Hidraw { file: File::from(OwnedFd::from(reader)) };
        let start = Instant::now();
        assert_eq!(hidraw.read_report(Duration::from_millis(50)).unwrap(), None);
        assert!(start.elapsed() >= Duration::from_millis(45), "waited about the whole timeout");
        writer.write_all(&[7; REPORT_LEN]).unwrap();
        assert_eq!(hidraw.read_report(Duration::from_millis(50)).unwrap(), Some([7; REPORT_LEN]));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib hidraw::`
Expected: compile error "cannot find function `retry_interrupted`" (and `Instant` not imported).

- [ ] **Step 3: Implement**

Change the time import to `use std::time::{Duration, Instant};`.

Replace `write_report`'s body with:

```rust
        // hidraw wants the report number first; QMK's raw-HID interface uses unnumbered reports (0).
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[1..].copy_from_slice(report);
        // hidraw takes a whole report per write(2) or none of it, so a short count can't be made
        // up with a second write: it is an error. An interrupted write sent nothing; retry it.
        let n = retry_interrupted(|| self.file.write(&buf))?;
        if n != buf.len() {
            return Err(io::Error::new(io::ErrorKind::WriteZero, format!("short hidraw write ({n} bytes)")));
        }
        Ok(())
```

Replace `read_report`'s body up to and including the `if rc == 0 { return Ok(None); }` check, and the `pfd.revents` check, with:

```rust
        let fd = self.file.as_raw_fd();
        let deadline = Instant::now() + timeout;
        let (rc, revents) = retry_interrupted(|| {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // Round up, so a wait never ends before the deadline.
            let left = deadline.saturating_duration_since(Instant::now());
            let ms = left.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
            // SAFETY: `pfd` is a valid pollfd for the duration of the call, and nfds is 1.
            let rc = unsafe { libc::poll(&mut pfd, 1, ms) };
            if rc < 0 { Err(io::Error::last_os_error()) } else { Ok((rc, pfd.revents)) }
        })?;
        if rc == 0 {
            return Ok(None);
        }
        if revents & libc::POLLIN == 0 {
            // POLLHUP / POLLERR / POLLNVAL without data: the device went away.
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "hidraw device gone"));
        }
```

Leave the rest (the `read` into `report`, the `n == 0` check, `Ok(Some(report))`) unchanged.

Add after the `impl Transport for Hidraw` block:

```rust
/// Runs `op` until it finishes with something other than `EINTR`: a signal arriving mid-call
/// is not a failure.
fn retry_interrupted<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        match op() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib hidraw::`
Expected: PASS.

- [ ] **Step 5: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/hid/hidraw.rs
git commit -m "fix: hidraw poll and write retry after EINTR instead of failing early (#8)"
```

---

### Task 8: evdev open failures are shown; event nodes in numeric order; discovery errors documented (#8)

**Files:**
- Modify: `src/input/evdev.rs` (`EvdevStatus`, `find_event_nodes`, new `event_number`, `start`, `status_for`, tests)
- Modify: `src/ui/dialogs.rs:45-50` (show `Failed`)
- Modify: `src/hid/discover.rs:23` (doc comment), new test

**Interfaces:**
- Produces: `EvdevStatus::Failed(String)` (new variant of a `pub enum`). The UI matches it in `dialogs.rs`. Everywhere else compares with `== EvdevStatus::Active` and needs no change.

- [ ] **Step 1: Write the failing tests**

In `src/input/evdev.rs` tests, update both `status_for` calls in `a_denied_node_means_no_access_even_if_another_opened` to pass a third argument, `vec![]`:

```rust
        assert_eq!(status_for(1, vec![kbd.clone()], vec![]), EvdevStatus::NoAccess(vec![kbd]));
        assert_eq!(status_for(4, vec![], vec![]), EvdevStatus::Active);
        assert_eq!(status_for(0, vec![], vec![]), EvdevStatus::NotFound);
```

and append:

```rust
    #[test]
    fn a_node_that_fails_to_open_is_reported_not_hidden() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let status = start(&[PathBuf::from("/nonexistent/event0")], tx, || {});
        assert!(matches!(&status, EvdevStatus::Failed(m) if m.contains("/nonexistent/event0")), "{status:?}");
        // Like a denied node, a failed one means no all-windows tracking even if another opened.
        let why = "cannot open /dev/input/event9: No such device".to_string();
        assert_eq!(status_for(1, vec![], vec![why.clone()]), EvdevStatus::Failed(why));
    }

    #[test]
    fn event_nodes_are_in_numeric_order() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let usb = root.join("devices/usb1/1-3");
        fs::create_dir_all(&usb).unwrap();
        fs::write(usb.join("idVendor"), "7171\n").unwrap();
        for (i, name) in ["event10", "event2", "event9"].iter().enumerate() {
            let input_dir = usb.join(format!("1-3:1.{i}/input/input{i}"));
            fs::create_dir_all(&input_dir).unwrap();
            let class_dir = root.join("class/input").join(name);
            fs::create_dir_all(&class_dir).unwrap();
            symlink(&input_dir, class_dir.join("device")).unwrap();
        }
        let nodes = find_event_nodes(root, Path::new("/dev"), &fs::canonicalize(&usb).unwrap());
        let names: Vec<_> = nodes.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();
        assert_eq!(names, ["event2", "event9", "event10"]);
    }
```

Append to `src/hid/discover.rs` tests:

```rust
    #[test]
    fn unreadable_hidraw_class_is_an_error() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("class")).unwrap();
        fs::write(t.path().join("class/hidraw"), "").unwrap(); // a file, so read_dir fails
        assert!(find_vial_device(t.path(), Path::new("/dev")).is_err());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib evdev:: discover::`
Expected: compile errors: `status_for` takes 2 arguments; no variant `Failed`. `unreadable_hidraw_class_is_an_error` passes already. It pins behaviour that is being kept on purpose.

- [ ] **Step 3: Implement**

In `src/input/evdev.rs`, replace the `EvdevStatus` enum with:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvdevStatus {
    Active,
    /// Nodes exist but some could not be opened (no udev rule yet).
    NoAccess(Vec<PathBuf>),
    /// Some node failed to open for another reason; the message says why.
    Failed(String),
    NotFound,
}
```

In `find_event_nodes`, replace `nodes.sort();` with `nodes.sort_by_key(|node| event_number(node));`, and add after the function:

```rust
/// `N` of an `eventN` node, so `event2` sorts before `event10`.
fn event_number(node: &Path) -> Option<u32> {
    node.file_name()?.to_str()?.strip_prefix("event")?.parse().ok()
}
```

Replace `start` and `status_for` with:

```rust
/// Opens every node and, only if all of them opened, spawns one reader thread per node.
pub fn start(nodes: &[PathBuf], tx: Sender<InputMsg>, notify: impl Fn() + Send + Clone + 'static) -> EvdevStatus {
    let (mut opened, mut denied, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for node in nodes {
        match Device::open(node) {
            Ok(dev) => opened.push((node.clone(), dev)),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => denied.push(node.clone()),
            Err(e) => {
                log::warn!("cannot open {}: {e}", node.display());
                failed.push(format!("cannot open {}: {e}", node.display()));
            }
        }
    }
    let status = status_for(opened.len(), denied, failed);
    if status == EvdevStatus::Active {
        for (node, dev) in opened {
            spawn_reader(node, dev, tx.clone(), notify.clone());
        }
    }
    status
}

/// Some of the keyboard's nodes can be readable without the udev rule (systemd gives the
/// seat user its joystick nodes), and those carry no key events. So tracking counts as
/// active only when every node opened.
fn status_for(opened: usize, denied: Vec<PathBuf>, failed: Vec<String>) -> EvdevStatus {
    if !denied.is_empty() {
        EvdevStatus::NoAccess(denied)
    } else if !failed.is_empty() {
        EvdevStatus::Failed(failed.join("; "))
    } else if opened > 0 {
        EvdevStatus::Active
    } else {
        EvdevStatus::NotFound
    }
}
```

In `src/ui/dialogs.rs`, replace:

```rust
        if !evdev_ok {
            hint_block(ui, &hints::evdev_hint());
            ui.label("Then press Reload (Ctrl+R).");
        }
```

with:

```rust
        if let EvdevStatus::Failed(why) = &app.evdev {
            ui.label(RichText::new(why.as_str()).color(ui.visuals().warn_fg_color));
            ui.label("Press Reload (Ctrl+R) to try again.");
        } else if !evdev_ok {
            hint_block(ui, &hints::evdev_hint());
            ui.label("Then press Reload (Ctrl+R).");
        }
```

In `src/hid/discover.rs`, replace the doc comment on `find_vial_device` with:

```rust
/// First Vial raw-HID interface. `sys_root` is normally `/sys`, `dev_root` `/dev`.
/// A missing `class/hidraw` means no device. Any other error is passed on on purpose: the
/// worker reports it once ("scanning for the keyboard failed"), which beats waiting silently.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib evdev:: discover::`
Expected: PASS.

- [ ] **Step 5: Full check**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all pass, no warnings. If clippy suggests `sort_by_key(event_number)` (redundant closure), take the suggestion only if it compiles; `&PathBuf` → `&Path` may need the closure.

- [ ] **Step 6: Commit**

```bash
git add src/input/evdev.rs src/ui/dialogs.rs src/hid/discover.rs
git commit -m "fix: show evdev open failures, sort event nodes numerically, document discovery errors (#8)"
```

---

### Task 9: Docs, version, verification and PR

**Files:**
- Modify: `docs/implementation-notes.md` (lines 78-131, "Known issues" to the end; new "egui" section before "Decisions made in review")
- Modify: `Cargo.toml` (`version`), `Cargo.lock` (regenerated)

- [ ] **Step 1: Add the new facts to the implementation notes**

Insert before `## Decisions made in review`:

```markdown
## egui

- **Keys that operate widgets must be removed in `raw_input_hook`.** egui moves focus on
  Tab in `Memory::begin_pass`, before `App::ui` runs, and a focused button treats Space and
  Enter as a click. The focused tier copies every event in `App::raw_input_hook`, then removes
  Tab, Space and Enter key events from what egui sees (#2).
- On Wayland a key held while the window loses focus never gets its release. egui clears
  its own `keys_down` on `Event::WindowFocused(false)`, so the app does the same for its
  focused-tier keys (#1).
- epaint 0.36's `has_glyph` returns false for anything in its emoji fonts. Check
  `glyph_width(..) > 0.0` instead. egui draws only its bundled fonts (Ubuntu-Light,
  NotoEmoji-Regular, emoji-icon-font), so glyph coverage is the same on every desktop.
- Tests that call `Context::run_ui` must `clear()` the output's `textures_delta`, or
  epaint panics.
```

Add to the end of the `## Decisions made in review` list:

```markdown
- hidraw short writes stay errors: the kernel takes a whole report per `write(2)` or none,
  so the rest can't be sent as a second write. `EINTR` is retried.
- `find_vial_device` passes on `read_dir` errors other than NotFound. The worker reports
  them once, which beats waiting silently.
```

- [ ] **Step 2: Rewrite the known-issues list**

Replace everything from `## Known issues` to the end of the file with:

```markdown
## Known issues

All of these are tracked as [GitHub issues](https://github.com/KaiEkkrin/lily58_assistant/issues).
The v1.0.1 rollup fixed #1, #2, #3, #7, #8 and #9. Still open:

1. **Vial may not detect the keyboard while the assistant is unlocked** (#4; not yet checked on
   hardware). Every program with the hidraw node open receives every reply. The assistant polls
   the matrix every 10 ms, but checks for other holders only once a second. So Vial's quick
   identify request can read one of our replies. Remedies range from documenting "close the
   assistant first" to watching the node with inotify.

Also left for later (#5): add `. "$HOME/.cargo/env"` to the README's Ubuntu steps.
The manual checklist still needs steps for Vial detection while unlocked, and TG followed by
Reload.

Small things from the per-task reviews (#10):

- `protocol::report()` panics without a message on more than 32 bytes. `Layout::key()` and
  `keycodes::basic_name` are linear scans (the latter runs per key per frame). Unparseable
  slot-0 layout labels are dropped silently. `config_path` is relative when neither `HOME` nor
  `XDG_CONFIG_HOME` is set. `VialError` has two I/O variants (`Io` and `Guard(Io)`).
- Tests: no `LayerTracker` test for `LM(layer, mods)`. The decode tests omit
  `PersistentDefault`/`TriLayerUpper`. There's no dedicated test that a locked keyboard gets
  no matrix requests. `fake.rs` holds both the scripted transport and the firmware simulator.
```

- [ ] **Step 3: Bump the version**

In `Cargo.toml`, change `version = "1.0.0"` to `version = "1.0.1"`. Then run `cargo build` to update `Cargo.lock`. CI builds with `--locked`, so the lock file must be committed.

- [ ] **Step 4: Full verification**

Run: `cargo build --locked && cargo test --locked && cargo clippy --locked --all-targets -- -D warnings`
Expected: build OK; every test passes; no clippy warnings. Record the test count in the PR (83 before this plan; this plan adds 22, so expect 105).

- [ ] **Step 5: Commit**

```bash
git add docs/implementation-notes.md Cargo.toml Cargo.lock
git commit -m "docs: known issues after the v1.0.1 rollup; chore: version 1.0.1"
```

- [ ] **Step 6: Push and open the PR**

The SSH key is usually locked, so push over HTTPS with gh's credential helper. Write the PR body to a scratch file outside the repo first (the session's temp directory, or `mktemp`):

```bash
git -c credential.helper= -c "credential.helper=!gh auth git-credential" push https://github.com/KaiEkkrin/lily58_assistant.git fix/v1-bug-rollup:fix/v1-bug-rollup
body=$(mktemp --suffix=.md)   # then write the body described below into "$body"
gh pr create --base main --head fix/v1-bug-rollup --title "Fix the small v1 bugs (#1, #2, #3, #7, #8, #9)" --body-file "$body"
```

The PR body lists what each commit fixes. It includes `Closes #1`, `Closes #2`, `Closes #3`, `Closes #7`, `Closes #8` and `Closes #9`, and it says:
- which items were closed without a code change and why: #8 `find_vial_device` and #9 glyphs;
- that #4 stays open pending a hardware check;
- that the new checklist steps are part of #5;
- the manual checks still needed on a real desktop: the two new "Focused window" checklist steps, and the Dismiss button on a config error. For the Dismiss check, put `colour = "red"` in `~/.config/lily58-assistant/config.toml` and start the app.

- [ ] **Step 7: Check CI**

Run: `gh run list --branch fix/v1-bug-rollup --limit 1`
Expected: the run for the PR's head commit completes with `success`.
