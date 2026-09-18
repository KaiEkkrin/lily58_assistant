# Typing Tutor Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A modal typing-tutor mode in the Lily58 Assistant: runtime-generated drills drawn from the live keymap, free-run scoring, finger-group colouring on the keyboard picture, and optional next-key hints that include the Shift or layer key needed to reach a character.

**Architecture:** A new `src/tutor/` module tree containing all the logic, none of which depends on egui — finger map, drill catalogue, batch generation, hint resolution, scoring, and a `Session` state machine. `App` owns a `Session` beside its existing `AppState` and routes focused-window keystrokes to it; `AppState` is untouched and stays the single source of truth for the layout and keymap. One new UI file (`src/ui/tutor.rs`) draws the top panel, and `src/ui/keyboard.rs` grows a `View` params struct for the colours and hints.

**Tech Stack:** Rust 2024, eframe/egui 0.36.2, rand 0.10.2 (new), serde_json (existing, tests), evdev, libc, lzma-rs.

**Spec:** `docs/superpowers/specs/2026-09-18-typing-tutor-design.md`

## Global Constraints

- Edition 2024, `rust-version = "1.95"`. Branch: `feat/typing-tutor` (already checked out, already carries the two spec commits).
- **One new dependency: `rand = "0.10.2"`.** `getrandom` arrives transitively via the default `sys_rng` feature; do not add it directly. No other new dependencies.
- rand 0.10 API, verified against the vendored source — earlier versions differ, so do not copy idioms from older tutorials:
  - OS-seeded: `rand::make_rng::<StdRng>()`. There is no `from_os_rng` in 0.10.
  - Deterministic: `StdRng::seed_from_u64(n)` via the `SeedableRng` trait.
  - `random_range(0..n)` lives on the `RngExt` trait, not `Rng`. Generator functions take `rng: &mut impl RngExt`.
  - Imports: `use rand::{RngExt, SeedableRng, rngs::StdRng};`
- The app stays **strictly read-only**. The tutor sends nothing to the keyboard; `hid::guard::ReadOnlyGuard` and its allowlist are not touched.
- `cargo test` must pass with no keyboard and no display. CI runs `cargo build --locked`, `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`; all three must pass after every task.
- **Do not run `cargo fmt`.** The code is not in rustfmt's default style (lines run to about 130 characters). Match the surrounding style by hand.
- egui tests that call `ctx.run_ui(...)` must call `.textures_delta.clear()` on the returned `FullOutput`, or epaint panics with "Dropped TexturesDelta with 1 unapplied deltas".
- Commit messages use the repo's prefixes (`feat:`, `fix:`, `docs:`, `chore:`). End every commit message with:
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`
- Doc comments explain *why*, in the voice of the existing modules. Do not add comments that restate the code.

## File Structure

| File | Responsibility |
|---|---|
| `src/tutor/mod.rs` | `Session`: the state machine, totals, availability. The only tutor type `ui/` touches. |
| `src/tutor/fingers.rs` | Hardwired matrix-position → `Spot` table; `validate` against a reported layout. |
| `src/tutor/drills.rs` | The drill catalogue: groups, sources, token lists. Data only. |
| `src/tutor/generate.rs` | Alphabet resolution and batch construction. Owns `words.txt`. |
| `src/tutor/words.txt` | Whitespace-separated common English words, `include_str!`'d. |
| `src/tutor/hint.rs` | `char` → `KeyPath` (which key, and what to hold). |
| `src/tutor/score.rs` | `Attempt`, `Summary`, and the display line splitter. |
| `src/tutor/fixture.rs` | `#[cfg(test)]` only: parses a `--probe` dump into a `Keymap`. |
| `src/ui/tutor.rs` | The top panel: drill picker, text block, live score, result. |
| `src/ui/keyboard.rs` | Gains a `View` params struct, finger-colour strokes and hint fills. |
| `src/ui/mod.rs` | Owns the `Session`, routes focused input to it, handles Ctrl+T. |
| `src/ui/status.rs` | Gains the tutor button. |
| `src/hostlayout.rs` | Gains `usages_for`: the reverse of `char_for`. |
| `tests/fixtures/lily58-keymap.txt` | The reference board's keymap, from `--probe`. |

---

### Task 1: The finger map

**Files:**
- Create: `src/tutor/mod.rs`
- Create: `src/tutor/fingers.rs`
- Modify: `src/lib.rs` (add `pub mod tutor;`)

**Interfaces:**
- Consumes: `crate::layout::Layout` (existing: `rows`, `cols`, `keys: Vec<KeyGeom>` with `row`/`col`).
- Produces: `fingers::{Hand, Finger, Band, Reach, Spot, spot, validate, ROWS, COLS}`. `spot(row, col) -> Option<Spot>` is the lookup every later task uses; `validate(&Layout) -> Result<(), String>` gates the whole feature.

- [ ] **Step 1: Write the failing test**

Create `src/tutor/fingers.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;

    fn reference_layout() -> Layout {
        let def = serde_json::from_str(include_str!("../../tests/fixtures/lily58-definition.json")).unwrap();
        Layout::from_definition(&def).unwrap()
    }

    fn at(row: u8, col: u8) -> Spot {
        spot(row, col).unwrap_or_else(|| panic!("({row}, {col}) should have a spot"))
    }

    /// Landmarks read off the reference board's layer 0, which is
    /// `KC_G KC_F KC_D KC_S KC_A KC_LCTL` across row 2 (columns 0 to 5).
    #[test]
    fn home_row_matches_the_keys_it_types() {
        assert_eq!(at(2, 4), Spot { hand: Hand::Left, finger: Finger::Pinky, band: Band::Home, reach: Reach::Normal }); // A
        assert_eq!(at(2, 3), Spot { hand: Hand::Left, finger: Finger::Ring, band: Band::Home, reach: Reach::Normal }); // S
        assert_eq!(at(2, 2), Spot { hand: Hand::Left, finger: Finger::Middle, band: Band::Home, reach: Reach::Normal }); // D
        assert_eq!(at(2, 1), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Home, reach: Reach::Normal }); // F
        assert_eq!(at(2, 0), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Home, reach: Reach::Inward }); // G
        assert_eq!(at(2, 5), Spot { hand: Hand::Left, finger: Finger::Pinky, band: Band::Home, reach: Reach::Outward }); // LCTL
        // The right half mirrors it: `KC_QUOT KC_SCLN KC_L KC_K KC_J KC_H` across row 7.
        assert_eq!(at(7, 0), Spot { hand: Hand::Right, finger: Finger::Pinky, band: Band::Home, reach: Reach::Outward }); // '
        assert_eq!(at(7, 4), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Home, reach: Reach::Normal }); // J
        assert_eq!(at(7, 5), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Home, reach: Reach::Inward }); // H
    }

    /// Row 4 is `KC_LBRC KC_SPC MO(1) KC_LGUI KC_LALT KC_MPLY`: the inner bottom key, four
    /// thumbs, and one position this build doesn't populate (an OLED sits there).
    #[test]
    fn the_extras_row_is_thumbs_an_inner_key_and_one_unpopulated_position() {
        assert_eq!(at(4, 0), Spot { hand: Hand::Left, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward });
        assert_eq!(at(9, 5), Spot { hand: Hand::Right, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward });
        for col in 1..=4 {
            assert_eq!(at(4, col).finger, Finger::Thumb, "left thumb {col}");
            assert_eq!(at(9, col).finger, Finger::Thumb, "right thumb {col}");
        }
        assert_eq!(spot(4, 5), None, "the left OLED position is not a key on this build");
        assert_eq!(spot(9, 0), None, "the right OLED position is not a key on this build");
    }

    #[test]
    fn the_table_covers_the_definition_exactly() {
        let layout = reference_layout();
        assert_eq!(layout.keys.len(), 60);
        let spots = (0..ROWS).flat_map(|r| (0..COLS).map(move |c| (r, c))).filter(|&(r, c)| spot(r, c).is_some()).count();
        assert_eq!(spots, 58, "58 real keys plus the 2 unpopulated positions is the 60 in the definition");
        assert_eq!(validate(&layout), Ok(()));
    }

    #[test]
    fn a_layout_that_does_not_match_is_rejected() {
        let mut layout = reference_layout();
        layout.keys.retain(|k| (k.row, k.col) != (2, 4));
        assert!(validate(&layout).unwrap_err().contains("(2, 4)"));

        let mut wrong_size = reference_layout();
        wrong_size.cols = 5;
        assert!(validate(&wrong_size).unwrap_err().contains("10x6"));
    }
}
```

Create `src/tutor/mod.rs`:

```rust
//! Typing tutor mode: drills generated from the keyboard's own keymap.

pub mod fingers;
```

Add to `src/lib.rs`, in alphabetical order (after `pub mod state;`):

```rust
pub mod tutor;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked tutor::fingers`
Expected: FAIL to compile — `cannot find type Spot in this scope`, `cannot find function spot`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `src/tutor/fingers.rs`:

```rust
//! Which finger each key belongs to: the basis for drill selection and the finger colouring.
//!
//! Hardwired for the Lily58's 10x6 matrix rather than derived from the layout geometry. The
//! columnar stagger is half the row pitch, so clustering keys by their `y` coordinate merges
//! the home and bottom rows, and a heuristic that misfires produces subtly wrong colours that
//! are hard to notice. `validate` checks this table against the layout the keyboard reported,
//! so a definition that doesn't match fails loudly instead.

use crate::layout::Layout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hand {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finger {
    Pinky,
    Ring,
    Middle,
    Index,
    Thumb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Number,
    Top,
    Home,
    Bottom,
    Thumb,
}

/// How far a key is from the finger's resting column: the pinky's outer column and the index
/// finger's inner column are stretches, everything else is where the finger already sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Normal,
    Outward,
    Inward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub hand: Hand,
    pub finger: Finger,
    pub band: Band,
    pub reach: Reach,
}

pub const ROWS: u8 = 10;
pub const COLS: u8 = 6;

/// Columns from each half's outer edge inwards.
const COLUMNS: [(Finger, Reach); COLS as usize] = [
    (Finger::Pinky, Reach::Outward),
    (Finger::Pinky, Reach::Normal),
    (Finger::Ring, Reach::Normal),
    (Finger::Middle, Reach::Normal),
    (Finger::Index, Reach::Normal),
    (Finger::Index, Reach::Inward),
];

const BANDS: [Band; 4] = [Band::Number, Band::Top, Band::Home, Band::Bottom];

/// Positions the firmware's matrix supports that this build doesn't populate: the OLED screens
/// sit there, and the keymap still assigns them something (`KC_MPLY` and `KC_MUTE`).
const NO_FINGER: [(u8, u8); 2] = [(4, 5), (9, 0)];

/// Which finger reaches this key, or `None` for a position no finger types.
pub fn spot(row: u8, col: u8) -> Option<Spot> {
    if col >= COLS {
        return None;
    }
    let (hand, half_row) = match row {
        0..=4 => (Hand::Left, row),
        5..=9 => (Hand::Right, row - 5),
        _ => return None,
    };
    // The left half's columns run from the inside out, so its outer edge is column 5.
    let from_outside = if hand == Hand::Left { COLS - 1 - col } else { col };
    if let Some(&band) = BANDS.get(half_row as usize) {
        let (finger, reach) = COLUMNS[from_outside as usize];
        return Some(Spot { hand, finger, band, reach });
    }
    // The extras row: the key between the halves, then four thumbs.
    match (hand, col) {
        (Hand::Left, 0) | (Hand::Right, 5) => {
            Some(Spot { hand, finger: Finger::Index, band: Band::Bottom, reach: Reach::Inward })
        }
        (_, 1..=4) => Some(Spot { hand, finger: Finger::Thumb, band: Band::Thumb, reach: Reach::Normal }),
        _ => None,
    }
}

/// Checks the layout the keyboard reported against the table, both ways round. A mismatch means
/// a different board or a changed definition, and the tutor refuses to run rather than colour
/// keys wrongly.
pub fn validate(layout: &Layout) -> Result<(), String> {
    if layout.rows != ROWS || layout.cols != COLS {
        return Err(format!(
            "the typing tutor expects a {ROWS}x{COLS} matrix; this keyboard reports {}x{}",
            layout.rows, layout.cols
        ));
    }
    let mut present = [[false; COLS as usize]; ROWS as usize];
    for key in &layout.keys {
        if key.row >= ROWS || key.col >= COLS {
            return Err(format!("this keyboard has a key at ({}, {}), outside the {ROWS}x{COLS} matrix", key.row, key.col));
        }
        present[key.row as usize][key.col as usize] = true;
    }
    for row in 0..ROWS {
        for col in 0..COLS {
            match (spot(row, col).is_some(), present[row as usize][col as usize]) {
                (true, false) => return Err(format!("this keyboard has no key at ({row}, {col}), which the finger map expects")),
                (false, true) if !NO_FINGER.contains(&(row, col)) => {
                    return Err(format!("this keyboard has a key at ({row}, {col}) that the finger map doesn't know"));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked tutor::fingers`
Expected: PASS, 4 tests.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/tutor/mod.rs src/tutor/fingers.rs
git commit -m "$(cat <<'MSG'
feat: finger map for the typing tutor

A hardwired matrix-position table rather than geometry: the columnar stagger is
half the row pitch, so clustering keys by y merges the home and bottom rows, and
a misfiring heuristic is hard to notice. validate() checks it against the layout
the keyboard reported, in both directions.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 2: Reverse the host layout

**Files:**
- Modify: `src/hostlayout.rs` (add `usages_for` after `char_for`, and tests in the existing test module)

**Interfaces:**
- Consumes: `HostLayout::char_for(usage, shift) -> Option<char>` (existing).
- Produces: `HostLayout::usages_for(self, c: char) -> Vec<(u8, bool)>` — every usage and Shift state that types `c`, ascending by usage. Task 4 depends on this being a list.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `src/hostlayout.rs`:

```rust
    /// GB maps both `KC_BSLS` (0x31) and `KC_NUHS` (0x32) to `#`, and a keymap may carry only
    /// one of them — the reference board has no `KC_BSLS` anywhere. Returning a single answer
    /// would declare `#` untypeable and silently strip every heading from the Markdown drill.
    #[test]
    fn a_character_can_have_more_than_one_usage() {
        assert_eq!(HostLayout::Gb.usages_for('#'), vec![(0x31, false), (0x32, false)]);
        assert_eq!(HostLayout::Us.usages_for('*'), vec![(0x25, true), (0x55, false)], "Shift+8 and the keypad");
    }

    #[test]
    fn ordinary_characters_resolve_to_one_usage() {
        let gb = HostLayout::Gb;
        assert_eq!(gb.usages_for('a'), vec![(0x04, false)]);
        assert_eq!(gb.usages_for('A'), vec![(0x04, true)]);
        assert_eq!(gb.usages_for('£'), vec![(0x20, true)]);
        assert_eq!(gb.usages_for('@'), vec![(0x34, true)]);
        assert_eq!(gb.usages_for(' '), vec![(0x2C, false)], "the shifted duplicate is dropped");
        assert_eq!(gb.usages_for('\\'), vec![(0x64, false)], "KC_NUBS on GB; KC_BSLS types # there");
        assert_eq!(gb.usages_for('€'), vec![], "not on either layout");
    }

    /// The main keyboard usage comes before its keypad duplicate, so a hint points at the key
    /// a Lily58 actually has.
    #[test]
    fn the_main_usage_comes_before_the_keypad_duplicate() {
        assert_eq!(HostLayout::Gb.usages_for('/'), vec![(0x38, false), (0x54, false)]);
    }

    /// Whatever `char_for` produces must resolve back to that character. Not necessarily to the
    /// same usage: the keypad duplicates make that a deliberately weaker claim.
    #[test]
    fn every_character_round_trips() {
        for layout in [HostLayout::Gb, HostLayout::Us] {
            for usage in 0x04..=0x67u8 {
                for shift in [false, true] {
                    let Some(c) = layout.char_for(usage, shift) else { continue };
                    let found = layout.usages_for(c);
                    assert!(!found.is_empty(), "{layout:?} {usage:#04x} shift={shift} -> {c:?} resolves to nothing");
                    for (u, s) in found {
                        assert_eq!(layout.char_for(u, s), Some(c), "{layout:?} {u:#04x} shift={s}");
                    }
                }
            }
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked hostlayout`
Expected: FAIL to compile — `no method named usages_for`.

- [ ] **Step 3: Write the implementation**

Add to `impl HostLayout` in `src/hostlayout.rs`, directly after `char_for`:

```rust
    /// Every HID usage and Shift state that types `c` on this layout, ascending by usage.
    ///
    /// A list rather than one answer, because a character can have several usages and a keymap
    /// may carry only some of them: on GB both `KC_BSLS` and `KC_NUHS` type `#`, and `/` exists
    /// on the main block and the keypad. Callers pick whichever their keymap actually has.
    pub fn usages_for(self, c: char) -> Vec<(u8, bool)> {
        let mut out = Vec::new();
        for usage in 0x04..=0x67u8 {
            let plain = self.char_for(usage, false);
            if plain == Some(c) {
                out.push((usage, false));
            } else if self.char_for(usage, true) == Some(c) {
                out.push((usage, true));
            }
        }
        out
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked hostlayout`
Expected: PASS.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/hostlayout.rs
git commit -m "$(cat <<'MSG'
feat: hostlayout::usages_for, the reverse of char_for

Returns every usage and Shift state that types a character, not just the first:
GB maps both KC_BSLS and KC_NUHS to #, and the reference keymap carries only
KC_NUHS, so a single answer would declare # untypeable.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 3: The reference keymap fixture

**Files:**
- Create: `tests/fixtures/lily58-keymap.txt` (from `probe_output.txt` in the working directory)
- Create: `src/tutor/fixture.rs`
- Modify: `src/tutor/mod.rs` (add the module)

**Interfaces:**
- Consumes: `keycodes::basic_name`, `keymap::Keymap::from_buffer`.
- Produces: `fixture::reference_keymap() -> Keymap` — the reference board's four layers. Tasks 4, 5 and 6 test against it. Test-only (`#[cfg(test)]`).

- [ ] **Step 1: Create the fixture file**

`probe_output.txt` sits untracked in the working directory. Take only the layer blocks — the header lines carry the device path and the keyboard uid, which the tests don't need:

```bash
awk '/^Layer 0:/{f=1} /^Input node:/{f=0} f' probe_output.txt > tests/fixtures/lily58-keymap.txt
wc -l tests/fixtures/lily58-keymap.txt   # must print 44: four layers of 1 header + 10 rows
```

Expected first line: `Layer 0:`. Expected last line: `  row 9:       KC_TRNS       KC_TRNS       KC_TRNS       KC_TRNS       KC_TRNS         KC_NO`

- [ ] **Step 2: Write the failing test**

Create `src/tutor/fixture.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_reference_dump() {
        let km = reference_keymap();
        assert_eq!((km.layers(), km.rows(), km.cols()), (4, 10, 6));
        assert_eq!(km.get(0, 2, 4), 0x0004, "layer 0 row 2 is KC_G KC_F KC_D KC_S KC_A KC_LCTL, so col 4 is KC_A");
        assert_eq!(km.get(0, 4, 2), 0x5221, "MO(1) on the left thumb");
        assert_eq!(km.get(0, 9, 3), 0x5222, "MO(2) on the right thumb");
        assert_eq!(km.get(0, 4, 1), 0x002C, "KC_SPC on the big left thumb key");
        assert_eq!(km.get(0, 4, 5), 0x00AE, "KC_MPLY at the unpopulated left position");
        assert_eq!(km.get(1, 8, 2), 0x022F, "LSFT(KC_LBRC) = {{ on layer 1");
        assert_eq!(km.get(1, 2, 5), 0x0032, "KC_NUHS = # on layer 1");
        assert_eq!(km.get(3, 2, 3), 0x7847, "a raw RGB keycode passes through as hex");
        assert_eq!(km.get(1, 0, 0), 0x0001, "KC_TRNS");
        assert_eq!(km.get(1, 8, 5), 0x0000, "KC_NO");
    }
}
```

Add to `src/tutor/mod.rs`:

```rust
#[cfg(test)]
pub mod fixture;
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked tutor::fixture`
Expected: FAIL to compile — `cannot find function reference_keymap`.

- [ ] **Step 4: Write the implementation**

Put this above the test module in `src/tutor/fixture.rs`:

```rust
//! The reference keyboard's keymap, for tests.
//!
//! Hint and generation tests are only worth much against a realistic keymap, and
//! `hid::fake::SMALL_KEYMAP` is a 2x3 toy. This parses the layer blocks a `--probe` run prints,
//! so the fixture stays human-readable and can be diffed against a fresh dump after a remap.

use crate::keycodes;
use crate::keymap::Keymap;

const PROBE_DUMP: &str = include_str!("../../tests/fixtures/lily58-keymap.txt");

pub fn reference_keymap() -> Keymap {
    keymap_from_probe(PROBE_DUMP)
}

/// Parses the `Layer N:` / `row R:` blocks `probe.rs` prints. Rows arrive layer-major, which is
/// the order `Keymap::from_buffer` wants.
pub fn keymap_from_probe(text: &str) -> Keymap {
    let mut codes: Vec<u16> = Vec::new();
    for line in text.lines() {
        let Some((_, rest)) = line.split_once("row ") else { continue };
        let (_, cells) = rest.split_once(':').expect("a row line has a colon after the row number");
        codes.extend(cells.split_whitespace().map(code_for));
    }
    let layers = u8::try_from(codes.len() / (10 * 6)).expect("the dump is a whole number of layers");
    let buf: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
    Keymap::from_buffer(layers, 10, 6, &buf).expect("the dump is a whole number of 10x6 layers")
}

fn code_for(name: &str) -> u16 {
    if let Some(hex) = name.strip_prefix("0x") {
        return u16::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad hex keycode {name:?}"));
    }
    if let Some(inner) = name.strip_prefix("LSFT(").and_then(|s| s.strip_suffix(')')) {
        return 0x0200 | code_for(inner);
    }
    if let Some(layer) = name.strip_prefix("MO(").and_then(|s| s.strip_suffix(')')) {
        return 0x5220 + layer.parse::<u16>().unwrap_or_else(|_| panic!("bad layer in {name:?}"));
    }
    (0x00..=0xFFu16)
        .find(|&c| keycodes::basic_name(c as u8) == Some(name))
        .unwrap_or_else(|| panic!("unknown keycode name {name:?}"))
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked tutor::fixture`
Expected: PASS.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add tests/fixtures/lily58-keymap.txt src/tutor/fixture.rs src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
test: the reference keyboard's keymap as a fixture

Parsed from a --probe dump, header stripped, so hint and generation tests can
assert against a real keymap instead of the 2x3 toy in hid::fake. Keeping the
fixture in the probe's own format means it can be diffed against a fresh dump
after a remap.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 4: Next-key hints

**Files:**
- Create: `src/tutor/hint.rs`
- Modify: `src/tutor/mod.rs` (add the module)

**Interfaces:**
- Consumes: `fingers::spot` (Task 1), `HostLayout::usages_for` (Task 2), `fixture::reference_keymap` (Task 3, tests only), `keycodes::{decode, tap_basic, adds_shift, Action}`, `Keymap::{layers, rows, cols, get}`.
- Produces: `hint::KeyPath { key: (u8, u8), hold: Vec<(u8, u8)> }` and `hint::resolve(&Keymap, HostLayout, char) -> Option<KeyPath>`. Tasks 5, 6, 9 and 10 use both.

- [ ] **Step 1: Write the failing test**

Create `src/tutor/hint.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycodes::{KC_NO, KC_TRNS};
    use crate::tutor::fixture::reference_keymap;

    fn path(c: char) -> KeyPath {
        resolve(&reference_keymap(), HostLayout::Gb, c).unwrap_or_else(|| panic!("{c:?} should resolve"))
    }

    #[test]
    fn equal_cost_paths_prefer_the_lower_layer_even_against_a_dedicated_key() {
        // `{` has three routes here. Shift plus layer 0's `[` at (4, 0) and layer 1's dedicated
        // LSFT(KC_LBRC) at (8, 2) holding MO(1) both cost one hold, so the lower layer wins.
        // Layer 2's `[` needs MO(2) and Shift, which is two.
        assert_eq!(path('{'), KeyPath { key: (4, 0), hold: vec![(8, 0)] });
    }

    /// `#` is only reachable as KC_NUHS on layer 1. This is the case that needs `usages_for` to
    /// return a list: the reference keymap has no KC_BSLS, which is the other GB usage for `#`.
    #[test]
    fn a_character_with_two_usages_finds_the_one_this_keymap_has() {
        assert_eq!(path('#'), KeyPath { key: (2, 5), hold: vec![(4, 2)] });
    }

    /// `!` is LSFT(KC_1) on layer 1 and plain KC_1 on layer 0. Both cost one hold, so the lower
    /// layer wins and the drill teaches Shift+1.
    #[test]
    fn equal_cost_paths_prefer_the_lower_layer() {
        assert_eq!(path('!'), KeyPath { key: (0, 4), hold: vec![(8, 0)] });
    }

    #[test]
    fn shift_comes_from_the_opposite_hand() {
        assert_eq!(path('A'), KeyPath { key: (2, 4), hold: vec![(8, 0)] }, "left-hand A, right Shift");
        assert_eq!(path('?'), KeyPath { key: (8, 1), hold: vec![(3, 5)] }, "right-hand /, left Shift");
    }

    #[test]
    fn unshifted_base_layer_keys_need_no_holds() {
        assert_eq!(path(' '), KeyPath { key: (4, 1), hold: vec![] });
        assert_eq!(path('['), KeyPath { key: (4, 0), hold: vec![] });
        assert_eq!(path('g'), KeyPath { key: (2, 0), hold: vec![] });
    }

    #[test]
    fn characters_this_keyboard_cannot_type_resolve_to_nothing() {
        assert_eq!(resolve(&reference_keymap(), HostLayout::Gb, '€'), None);
    }

    /// Layer keys are looked for on layer 0 only. On the reference keymap `MO(3)` exists solely
    /// on layers 1 and 2, so a wider search would name a key that does nothing from the base
    /// layer and hand out an impossible hint.
    #[test]
    fn characters_behind_a_second_layer_key_are_unreachable() {
        // 4 layers, 1 row, 2 cols. Layer 0: MO(1), KC_NO. Layer 1: TRNS, MO(3). Layer 3: TRNS, KC_A.
        let codes: [u16; 8] = [0x5221, KC_NO, KC_TRNS, 0x5223, KC_NO, KC_NO, KC_TRNS, 0x0004];
        let buf: Vec<u8> = codes.iter().flat_map(|c| c.to_be_bytes()).collect();
        let km = Keymap::from_buffer(4, 1, 2, &buf).unwrap();
        assert_eq!(resolve(&km, HostLayout::Gb, 'a'), None);
    }
}
```

Add to `src/tutor/mod.rs` (keep the modules alphabetical, `#[cfg(test)]` ones last):

```rust
pub mod hint;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked tutor::hint`
Expected: FAIL to compile — `cannot find type KeyPath`, `cannot find function resolve`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `src/tutor/hint.rs`:

```rust
//! Where a character lives on this keymap, and what to hold to reach it.

use crate::hostlayout::HostLayout;
use crate::keycodes::{self, Action};
use crate::keymap::Keymap;
use crate::tutor::fingers;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPath {
    /// The key that emits the character.
    pub key: (u8, u8),
    /// Keys to hold first: a layer key, a Shift key, or neither.
    pub hold: Vec<(u8, u8)>,
}

/// The cheapest way to type `c` on this keymap, or `None` if it can't be typed.
///
/// Cheapest means fewest keys held, tie-broken by the lower layer and then matrix order. Every
/// candidate is enumerated rather than taking the first match, because a preference only exists
/// if you can see them all: on the reference keymap `{` can be Shift plus layer 0's `[`, layer
/// 1's dedicated `LSFT(KC_LBRC)`, or layer 2's `[` plus Shift — the first two tie at one hold
/// and the lower layer breaks it. (`Keymap::find_position` still serves the OS-key inference it
/// was written for; it stops at the first hit and knows nothing about Shift.)
pub fn resolve(keymap: &Keymap, host: HostLayout, c: char) -> Option<KeyPath> {
    let usages = host.usages_for(c);
    if usages.is_empty() {
        return None;
    }
    let mut best: Option<(usize, u8, u8, u8, KeyPath)> = None;
    for layer in 0..keymap.layers() {
        for row in 0..keymap.rows() {
            for col in 0..keymap.cols() {
                let code = keymap.get(layer, row, col);
                let Some(basic) = keycodes::tap_basic(code) else { continue };
                let Some(&(_, shift)) = usages.iter().find(|&&(usage, _)| usage == basic) else { continue };
                let mut hold = Vec::new();
                if layer != 0 {
                    let Some(key) = layer_key(keymap, layer) else { continue };
                    hold.push(key);
                }
                if shift && !keycodes::adds_shift(code) {
                    let Some(key) = shift_key(keymap, (row, col)) else { continue };
                    hold.push(key);
                }
                let rank = (hold.len(), layer, row, col);
                if best.as_ref().is_none_or(|b| rank < (b.0, b.1, b.2, b.3)) {
                    best = Some((rank.0, rank.1, rank.2, rank.3, KeyPath { key: (row, col), hold }));
                }
            }
        }
    }
    best.map(|(.., path)| path)
}

/// A key on **layer 0** that turns `layer` on while it is held, preferring hold-to-use kinds.
///
/// Layer 0 only, deliberately. On the reference keymap `MO(3)` exists only on layers 1 and 2, so
/// searching every layer would name a key that does nothing from the base layer. Anything deeper
/// than one layer key from base is treated as unreachable, which fails closed.
fn layer_key(keymap: &Keymap, layer: u8) -> Option<(u8, u8)> {
    let rank = |code: u16| match keycodes::decode(code) {
        Action::Momentary(l) | Action::LayerTap { layer: l, .. } if l == layer => Some(0u8),
        Action::LayerMod { layer: l, .. } if l == layer => Some(1),
        Action::OneShotLayer(l) if l == layer => Some(2),
        Action::Toggle(l) | Action::TapToggle(l) | Action::To(l) if l == layer => Some(3),
        _ => None,
    };
    let mut best: Option<(u8, (u8, u8))> = None;
    for row in 0..keymap.rows() {
        for col in 0..keymap.cols() {
            if let Some(r) = rank(keymap.get(0, row, col))
                && best.is_none_or(|(br, _)| r < br)
            {
                best = Some((r, (row, col)));
            }
        }
    }
    best.map(|(_, key)| key)
}

/// A Shift key, preferring one on the opposite hand to `target` — the way you'd actually type it.
fn shift_key(keymap: &Keymap, target: (u8, u8)) -> Option<(u8, u8)> {
    let target_hand = fingers::spot(target.0, target.1).map(|s| s.hand);
    let mut fallback = None;
    for row in 0..keymap.rows() {
        for col in 0..keymap.cols() {
            let shifts = match keycodes::decode(keymap.get(0, row, col)) {
                Action::Basic(b) => b == 0xE1 || b == 0xE5,
                Action::ModTap { mods, .. } => mods & 0x02 != 0,
                _ => false,
            };
            if !shifts {
                continue;
            }
            let hand = fingers::spot(row, col).map(|s| s.hand);
            if hand.is_some() && hand != target_hand {
                return Some((row, col));
            }
            fallback.get_or_insert((row, col));
        }
    }
    fallback
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked tutor::hint`
Expected: PASS, 7 tests.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tutor/hint.rs src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: resolve a character to the key and holds that type it

Enumerates every key that emits the character across all layers and picks the
cheapest: fewest holds, then lower layer, then matrix order. Layer keys are
looked for on layer 0 only, because MO(3) exists only on layers 1 and 2 of the
reference keymap and a wider search would hand out an impossible hint. Shift is
taken from the opposite hand.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 5: The drill catalogue

**Files:**
- Create: `src/tutor/drills.rs`
- Modify: `src/tutor/mod.rs` (add the module)

**Interfaces:**
- Consumes: `fingers::{Band, Reach, Spot, spot, ROWS, COLS}` (Task 1), `hint::resolve` (Task 4, tests only).
- Produces: `drills::{Group, Shift, Style, Kind, Source, Drill, DrillId, DRILLS, drill, ids_of}`. `DrillId` is an index into `DRILLS`; Tasks 6, 8 and 10 use it.

- [ ] **Step 1: Write the failing test**

Create `src/tutor/drills.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostlayout::HostLayout;
    use crate::tutor::fingers;
    use crate::tutor::fixture::reference_keymap;
    use crate::tutor::hint;

    #[test]
    fn the_catalogue_is_the_twelve_drills_the_spec_lists() {
        assert_eq!(DRILLS.len(), 12);
        assert_eq!(ids_of(Kind::Position).count(), 7);
        assert_eq!(ids_of(Kind::Programmer).count(), 5);
        assert_eq!(drill(0).name, "Home keys");
        let names: Vec<&str> = DRILLS.iter().map(|d| d.name).collect();
        for expected in ["Stretch up", "Stretch down", "Number row", "Outer column", "Index reach",
                         "Shift combinations", "Markdown", "HTML", "Rust", "TypeScript", "Elixir"] {
            assert!(names.contains(&expected), "{expected} is missing");
        }
    }

    /// An item has to be buildable: every key the focus selects must also be in `include`, or the
    /// drill could never satisfy its own rule.
    #[test]
    fn every_focus_group_is_inside_its_include_list() {
        for d in DRILLS {
            let Source::Keys { include, focus: Some(focus) } = d.source else { continue };
            for row in 0..fingers::ROWS {
                for col in 0..fingers::COLS {
                    let Some(spot) = fingers::spot(row, col) else { continue };
                    if focus.matches(&spot) {
                        assert!(include.iter().any(|g| g.matches(&spot)), "{}: ({row}, {col}) is focused but not included", d.name);
                    }
                }
            }
        }
    }

    /// The shipped snippets have to be typeable, or the drill quietly shrinks.
    #[test]
    fn every_token_can_be_typed_on_the_reference_keymap() {
        let km = reference_keymap();
        for d in DRILLS {
            let Source::Tokens(tokens) = d.source else { continue };
            assert!(!tokens.is_empty(), "{} has no tokens", d.name);
            for token in tokens {
                for c in token.chars() {
                    assert!(hint::resolve(&km, HostLayout::Gb, c).is_some(), "{}: {token:?} needs {c:?}", d.name);
                }
            }
        }
    }

    #[test]
    fn home_keys_selects_the_eight_resting_keys() {
        let Source::Keys { focus: Some(home), .. } = drill(0).source else { panic!("Home keys uses key groups") };
        let selected: Vec<(u8, u8)> = (0..fingers::ROWS)
            .flat_map(|r| (0..fingers::COLS).map(move |c| (r, c)))
            .filter(|&(r, c)| fingers::spot(r, c).is_some_and(|s| home.matches(&s)))
            .collect();
        assert_eq!(selected.len(), 8, "four fingers on each hand, no stretches: {selected:?}");
    }
}
```

Add to `src/tutor/mod.rs`:

```rust
pub mod drills;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked tutor::drills`
Expected: FAIL to compile — `cannot find value DRILLS`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `src/tutor/drills.rs`:

```rust
//! The drill catalogue: what each difficulty draws from. Data only.

use crate::tutor::fingers::{Band, Reach, Spot};

/// A conjunctive predicate over key positions. `Group { band: Home, reach: Normal }` is the eight
/// resting keys — G, H and the modifier columns are excluded rather than argued about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub band: Option<Band>,
    pub reach: Option<Reach>,
}

impl Group {
    const fn at(band: Band, reach: Reach) -> Group {
        Group { band: Some(band), reach: Some(reach) }
    }

    const fn reaching(reach: Reach) -> Group {
        Group { band: None, reach: Some(reach) }
    }

    pub fn matches(self, spot: &Spot) -> bool {
        self.band.is_none_or(|b| b == spot.band) && self.reach.is_none_or(|r| r == spot.reach)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    Never,
    Allowed,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Words,
    Syllables,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Position,
    Programmer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Characters come from the keys these groups select, resolved through the live keymap. The
    /// alphabet is everything `include` selects; an item is kept only if it uses something from
    /// `focus`, so a drill mixes new keys with familiar ones yet always exercises the new ones.
    Keys { include: &'static [Group], focus: Option<Group> },
    /// Fixed snippets, filtered to the ones this keymap can type.
    Tokens(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drill {
    pub name: &'static str,
    pub kind: Kind,
    pub source: Source,
    pub shift: Shift,
    /// Ignored for `Source::Tokens`, which always emits whole snippets.
    pub style: Style,
}

/// An index into `DRILLS`.
pub type DrillId = usize;

pub fn drill(id: DrillId) -> &'static Drill {
    &DRILLS[id]
}

pub fn ids_of(kind: Kind) -> impl Iterator<Item = DrillId> {
    (0..DRILLS.len()).filter(move |&id| DRILLS[id].kind == kind)
}

const HOME: Group = Group::at(Band::Home, Reach::Normal);
const TOP: Group = Group::at(Band::Top, Reach::Normal);
const BOTTOM: Group = Group::at(Band::Bottom, Reach::Normal);
const NUMBER: Group = Group::at(Band::Number, Reach::Normal);
const OUTWARD: Group = Group::reaching(Reach::Outward);
const INWARD: Group = Group::reaching(Reach::Inward);

pub const DRILLS: &[Drill] = &[
    Drill {
        name: "Home keys",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME], focus: Some(HOME) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    Drill {
        name: "Stretch up",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, TOP], focus: Some(TOP) },
        shift: Shift::Never,
        style: Style::Words,
    },
    // Letter groups, not words: the bottom row is z x c v m , . / and English barely uses
    // those letters together — the word list yields six. The Words fallback exists for
    // surprises, not for a standard drill that would apologise on every batch.
    Drill {
        name: "Stretch down",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, BOTTOM], focus: Some(BOTTOM) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    Drill {
        name: "Number row",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, NUMBER], focus: Some(NUMBER) },
        shift: Shift::Never,
        style: Style::Syllables,
    },
    // Right-pinky only on the reference board, and permanently so: the left outer column is
    // Esc/Tab/Ctrl/Shift, which type nothing. Still position-driven, so it would pick up a
    // character if one ever appeared there.
    Drill {
        name: "Outer column",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, OUTWARD], focus: Some(OUTWARD) },
        shift: Shift::Allowed,
        style: Style::Syllables,
    },
    Drill {
        name: "Index reach",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, INWARD], focus: Some(INWARD) },
        shift: Shift::Never,
        style: Style::Words,
    },
    // No focus group: `Shift::Required` already guarantees every item uses Shift, which is the
    // whole point of the drill.
    Drill {
        name: "Shift combinations",
        kind: Kind::Position,
        source: Source::Keys { include: &[HOME, TOP, BOTTOM, NUMBER], focus: None },
        shift: Shift::Required,
        style: Style::Words,
    },
    Drill { name: "Markdown", kind: Kind::Programmer, source: Source::Tokens(MARKDOWN), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "HTML", kind: Kind::Programmer, source: Source::Tokens(HTML), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "Rust", kind: Kind::Programmer, source: Source::Tokens(RUST), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "TypeScript", kind: Kind::Programmer, source: Source::Tokens(TYPESCRIPT), shift: Shift::Allowed, style: Style::Words },
    Drill { name: "Elixir", kind: Kind::Programmer, source: Source::Tokens(ELIXIR), shift: Shift::Allowed, style: Style::Words },
];

const MARKDOWN: &[&str] = &[
    "# Heading", "## Section", "### Detail", "**bold**", "_italic_", "`code`", "[text](url)",
    "![alt](img.png)", "- [ ]", "- [x]", "> quote", "---", "|---|---|", "1. first", "2. second",
    "~~struck~~", "&nbsp;", "```rust", "<!-- note -->", "* bullet",
];

const HTML: &[&str] = &[
    "<div>", "</div>", "<span>", "</span>", "<p>", "</p>", "<ul>", "<li>", "<br />", "<hr />",
    // `r#"..."#` will not do for the first one: its content contains `"#`, which ends the
    // literal early. Two hashes, and the token text is unchanged.
    r##"<a href="#">"##, r#"class="row""#, r#"id="main""#, r#"<input type="text">"#,
    "<!-- note -->", "&amp;", "&lt;", "&gt;", "</html>", "<h1>",
];

const RUST: &[&str] = &[
    "fn main()", "let mut x = 0;", "-> Result<(), E>", "&mut self", "Vec<u8>", "Option<&str>",
    "impl Trait", "match x {", "=> {}", "#[derive(Debug)]", "|x| x + 1", "0..=9", "self.field",
    "::<T>", "?;", "&[u8]", "if let Some(v)", "pub(crate)", "'static", "format!(\"{x}\")",
];

const TYPESCRIPT: &[&str] = &[
    "const x = 1;", "=> {}", "?.", "??", ": string", "<T>", "${value}", "async () =>", "await fn()",
    "...rest", "interface X {", "export default", r#"import { a } from "b";"#, "as const",
    "!== null", "Array<number>", "type Id = string;", "readonly", "#private", "obj?.key",
];

const ELIXIR: &[&str] = &[
    "|>", "->", "defmodule X do", "def run(x) do", "end", "%{key: 1}", ":atom", "<<1, 2>>",
    "fn x -> x end", "case x do", "{:ok, v}", "{:error, e}", "=~", "@moduledoc", "&1",
    "Enum.map(list)", "|> Enum.filter()", "do:", "..", "when is_map(x)",
];
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked tutor::drills`
Expected: PASS, 4 tests. If `every_token_can_be_typed_on_the_reference_keymap` fails, the named character genuinely isn't on the reference keymap — remove that token rather than weakening the test.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tutor/drills.rs src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: the typing tutor's drill catalogue

Twelve drills: seven selecting key positions through the live keymap, five
fixed token lists for Markdown, HTML, Rust, TypeScript and Elixir. The
include/focus split mixes new keys with familiar ones while guaranteeing every
item exercises the new ones. Tests assert every shipped token is typeable on the
reference keymap.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 6: Batch generation

**Files:**
- Create: `src/tutor/generate.rs`
- Create: `src/tutor/words.txt`
- Modify: `src/tutor/mod.rs` (add the module)
- Modify: `Cargo.toml` (add `rand`)

**Interfaces:**
- Consumes: everything from Tasks 1–5.
- Produces: `generate::{Letter, Alphabet, Batch, StartError, alphabet, batch, keys_text, SYLLABLE_NOTE, BATCH_CHARS}`. `Batch { target: Vec<char>, paths: Vec<Option<KeyPath>>, note: Option<&'static str> }` is what Tasks 7, 8 and 10 consume.

- [ ] **Step 1: Add the dependency and the word list**

In `Cargo.toml`, under `[dependencies]`, on its own line between `lzma-rs` and `serde`, which
is where the existing alphabetical order puts it:

```toml
rand = "0.10.2"
```

Create `src/tutor/words.txt`. Whitespace-separated, so it can be wrapped; the loader uses `split_whitespace`:

```
able about above across act add after again age agree air all allow almost alone along already
also always among amount and animal another answer any appear apple area arm around arrive art ask
away baby back bad bag ball bank base basic beach bear beat beauty because become bed before begin
behind believe below best better between big bird bit black block blood blue board boat body book
born both bottom box boy break bridge bright bring broad brother brown build burn business busy buy
call calm camp can capital car card care carry case catch cause cell center century certain chair
chance change chapter charge check child choose church circle city claim class clean clear climb
clock close cloth cloud coast cold collect college color come common company compare complete
computer concern condition consider contain continue control cook cool copy corner cost could count
country couple course cover create cross crowd cry culture cup current cut dance danger dark date
daughter day dead deal dear death decide deep degree deliver depend describe desert design desk
detail develop die differ difficult dinner direct discover discuss distance divide doctor dog dollar
door double doubt down draw dream dress drink drive drop dry during duty each early earth ease east
easy eat edge effect effort egg eight either elect else empty end enemy energy enjoy enough enter
entire equal escape even evening event ever every exact example except exchange exercise exist
expect experience explain express extra eye face fact fail fair fall family famous far farm fast
father fear feed feel feet fell felt few field fight figure fill film final find fine finger finish
fire first fish fit five fix flat floor flow flower fly follow food foot force forest forget form
former forward found four free fresh friend from front fruit full fun future game garden gas gather
general gentle get gift girl give glad glass goal gold gone good grade grand grass great green
ground group grow guard guess guide hair half hand hang happen happy hard hat hate have head health
hear heart heat heavy held help here high hill him his hit hold hole home hope horse hospital hot
hour house how huge human hundred hunt hurry hurt husband ice idea ill image imagine important
improve include increase indeed industry inside instead interest into iron island issue job join joy
judge jump just keep key kid kill kind king kiss kitchen knee knew know lack lady lake land language
large last late laugh law lay lead leaf learn least leave led left leg legal length less let letter
level lie life lift light like limit line lip list listen little live local lock long look lose loss
lot loud love low luck lunch machine main major make man many map march mark market marry mass
master match material matter may maybe mean measure meat medical meet member memory mention method
middle might mile milk mind mine minute miss mistake mix model modern moment money month moon more
morning most mother mountain mouth move movie much music must name nation natural near neck need
neither nerve net never new news next nice night nine noise none noon nor north nose not note
nothing notice now number ocean odd off offer office often oil old once one only open operate
opinion orange order other our out outside over own page pain paint pair paper parent park part
party pass past path patient pattern pay peace people perfect perhaps period person phone photo pick
picture piece place plan plant play please plenty point police policy poor pop popular port position
possible post pound pour power practice prepare present press pretty prevent price print private
prize probably problem produce program promise protect prove provide public pull purpose push put
quality quarter queen question quick quiet quite quote race radio rail rain raise range rapid rare
rate rather reach read ready real reason receive recent record red reduce refer reflect refuse
region regular relate remain remember remove repeat reply report request require rest result return
rich ride right ring rise risk river road rock roll room root rope rose round row rule run safe sail
salt same sand save say scale scene school science score sea search season seat second secret
section see seed seek seem sell send sense separate serious serve service set settle seven several
shall shape share sharp sheet shelf shell shine ship shoe shoot shop short should shoulder shout
show side sign silent silver similar simple since sing single sir sister sit site situation six size
skill skin sky sleep slow small smell smile smoke snow social soft soil soldier solid solve some son
song soon sort sound source south space speak special speed spell spend spirit spot spread spring
square staff stage stand star start state stay step stick still stock stone stop store storm story
straight strange street strike strong student study stuff style subject success sudden suffer sugar
suggest summer sun supply support suppose sure surface surprise sweet swim system table take talk
tall task taste teach team tear tell ten term test than thank that the their them then there these
they thick thin thing think third this those though thought thousand three through throw thus tie
time tiny tire title today together told tomorrow tone tonight too took tool top total touch toward
town trade train travel treat tree trip trouble true trust truth try turn twelve twenty twice two
type uncle under understand union unit until upon use usual valley value various very view village
visit voice vote wait walk wall want war warm wash waste watch water wave way weak wear weather week
weight welcome well were west wet what wheel when where which while white who whole why wide wife
wild will win wind window wine wing winter wire wise wish with within without woman wonder wood word
work world worry worth would write wrong yard year yellow yes yet you young your youth
```

- [ ] **Step 2: Write the failing test**

Create `src/tutor/generate.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use crate::tutor::drills::{self, DRILLS};
    use crate::tutor::fixture::reference_keymap;

    fn seeded() -> StdRng {
        StdRng::seed_from_u64(20260918)
    }

    fn find(name: &str) -> &'static drills::Drill {
        DRILLS.iter().find(|d| d.name == name).expect("the catalogue has this drill")
    }

    fn made_of(chars: &str) -> Alphabet {
        let letters = chars
            .chars()
            .map(|ch| Letter { ch, hand: Some(Hand::Left), shifted: false, focus: ch != ' ' })
            .collect();
        Alphabet { letters }
    }

    #[test]
    fn the_home_drill_resolves_to_the_eight_resting_keys() {
        let alpha = alphabet(drills::drill(0), &reference_keymap(), HostLayout::Gb);
        let mut focus = alpha.focus_chars();
        focus.sort_unstable();
        assert_eq!(focus, vec![';', 'a', 'd', 'f', 'j', 'k', 'l', 's']);
        assert!(alpha.chars().any(|c| c == ' '), "space is always available, whatever the drill");
    }

    /// The left outer column is Esc/Tab/Ctrl/Shift, which type nothing, so this drill is the
    /// right pinky alone. `Shift::Allowed` brings in the shifted forms.
    #[test]
    fn the_outer_column_drill_is_the_right_pinky_alone() {
        let alpha = alphabet(find("Outer column"), &reference_keymap(), HostLayout::Gb);
        let mut focus = alpha.focus_chars();
        focus.sort_unstable();
        assert_eq!(focus, vec!['\'', '-', '@', '_', '`', '¬']);
        assert!(
            alpha.letters.iter().filter(|l| l.focus).all(|l| l.hand == Some(Hand::Right)),
            "nothing on the left outer column types a character"
        );
    }

    #[test]
    fn every_batch_stays_inside_its_alphabet_and_exercises_its_focus() {
        let km = reference_keymap();
        for d in DRILLS {
            let Source::Keys { .. } = d.source else { continue };
            let alpha = alphabet(d, &km, HostLayout::Gb);
            let focus = alpha.focus_chars();
            let batch = batch(d, &km, HostLayout::Gb, &mut seeded()).unwrap_or_else(|e| panic!("{}: {e}", d.name));
            assert!(batch.target.len() >= BATCH_CHARS, "{}: only {} characters", d.name, batch.target.len());
            for &c in &batch.target {
                assert!(alpha.chars().any(|a| a == c), "{}: {c:?} is not in the alphabet", d.name);
            }
            let text: String = batch.target.iter().collect();
            for item in text.split(' ') {
                assert!(item.chars().any(|c| focus.contains(&c)), "{}: {item:?} exercises nothing", d.name);
            }
            assert_eq!(batch.paths.len(), batch.target.len());
            assert!(batch.paths.iter().all(Option::is_some), "{}: every character must resolve", d.name);
            assert_eq!(batch.note, None, "{}: the reference keymap needs no fallback", d.name);
        }
    }

    #[test]
    fn the_shift_drill_asks_for_shift_in_every_item() {
        let km = reference_keymap();
        let batch = batch(find("Shift combinations"), &km, HostLayout::Gb, &mut seeded()).unwrap();
        let text: String = batch.target.iter().collect();
        for item in text.split(' ') {
            assert!(item.chars().any(char::is_uppercase), "{item:?} has nothing shifted in it");
        }
    }

    #[test]
    fn token_drills_emit_whole_tokens_only() {
        let km = reference_keymap();
        let d = find("Rust");
        let Source::Tokens(tokens) = d.source else { panic!("Rust is a token drill") };
        let batch = batch(d, &km, HostLayout::Gb, &mut seeded()).unwrap();
        let text: String = batch.target.iter().collect();
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let token = tokens.iter().find(|t| rest.starts_with(*t)).unwrap_or_else(|| panic!("no token starts {rest:?}"));
            rest = rest[token.len()..].strip_prefix(' ').unwrap_or("");
        }
    }

    /// A remapped keymap could leave a `Words` drill with nothing to say. It uses letter groups
    /// and reports that, rather than serving gibberish where words were promised.
    #[test]
    fn a_thin_word_pool_falls_back_to_letter_groups_and_says_so() {
        let alpha = made_of("qzxj ");
        let (text, note) = keys_text(find("Stretch up"), &alpha, &mut seeded()).unwrap();
        assert_eq!(note, Some(SYLLABLE_NOTE));
        assert!(text.chars().all(|c| "qzxj ".contains(c)), "{text:?}");
    }

    #[test]
    fn a_drill_with_almost_nothing_to_type_is_refused() {
        let err = keys_text(find("Home keys"), &made_of("ab "), &mut seeded()).unwrap_err();
        assert_eq!(err, StartError::TooFewKeys { drill: "Home keys", found: 2 });
        assert!(err.to_string().contains("Home keys"));
    }

    /// Syllables shouldn't hammer one finger: the same key never repeats immediately.
    #[test]
    fn letter_groups_never_repeat_a_key_back_to_back() {
        let alpha = alphabet(drills::drill(0), &reference_keymap(), HostLayout::Gb);
        let (text, _) = keys_text(drills::drill(0), &alpha, &mut seeded()).unwrap();
        for item in text.split(' ') {
            let chars: Vec<char> = item.chars().collect();
            for pair in chars.windows(2) {
                assert_ne!(pair[0], pair[1], "{item:?} repeats a key");
            }
        }
    }
}
```

Add to `src/tutor/mod.rs`:

```rust
pub mod generate;
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked tutor::generate`
Expected: FAIL to compile — `cannot find function alphabet`, `cannot find type Alphabet`.

- [ ] **Step 4: Write the implementation**

Put this above the test module in `src/tutor/generate.rs`:

```rust
//! Turning a drill into a batch of text to type.

use rand::RngExt;

use crate::hostlayout::HostLayout;
use crate::keycodes;
use crate::keymap::Keymap;
use crate::tutor::drills::{Drill, Shift, Source, Style};
use crate::tutor::fingers::{self, Hand};
use crate::tutor::hint::{self, KeyPath};

const WORDS: &str = include_str!("words.txt");

/// Characters a batch aims for. Items are added whole, so a batch overshoots slightly. One
/// number, so every drill costs comparable effort whatever the length of its items.
pub const BATCH_CHARS: usize = 110;
/// Below this many words a `Words` drill would repeat itself into nonsense.
const MIN_WORD_POOL: usize = 12;
/// Fewer typeable keys than this and the drill teaches nothing.
const MIN_LETTERS: usize = 4;
const MIN_FOCUS: usize = 2;

pub const SYLLABLE_NOTE: &str = "Not enough words on this keymap for this drill, so it's letter groups instead.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Letter {
    pub ch: char,
    pub hand: Option<Hand>,
    pub shifted: bool,
    /// One of the keys this drill exists to exercise.
    pub focus: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Alphabet {
    pub letters: Vec<Letter>,
}

impl Alphabet {
    pub fn chars(&self) -> impl Iterator<Item = char> + '_ {
        self.letters.iter().map(|l| l.ch)
    }

    pub fn focus_chars(&self) -> Vec<char> {
        self.letters.iter().filter(|l| l.focus && l.ch != ' ').map(|l| l.ch).collect()
    }

    /// Everything but the separator.
    fn typeable(&self) -> Vec<Letter> {
        self.letters.iter().copied().filter(|l| l.ch != ' ').collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Batch {
    pub target: Vec<char>,
    /// Parallel to `target`: how to type each character. Resolved once here rather than per
    /// frame, so drawing a hint is an index lookup.
    pub paths: Vec<Option<KeyPath>>,
    pub note: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    TooFewKeys { drill: &'static str, found: usize },
    NoTokens { drill: &'static str },
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::TooFewKeys { drill, found } => {
                write!(f, "\"{drill}\" needs more typeable keys than this keymap gives it (found {found})")
            }
            StartError::NoTokens { drill } => write!(f, "\"{drill}\" has nothing this keymap can type"),
        }
    }
}

/// The characters a drill may use, resolved through the keymap's base layer.
pub fn alphabet(drill: &Drill, keymap: &Keymap, host: HostLayout) -> Alphabet {
    let Source::Keys { include, focus } = drill.source else { return Alphabet::default() };
    let mut letters: Vec<Letter> = Vec::new();
    for row in 0..fingers::ROWS {
        for col in 0..fingers::COLS {
            let Some(spot) = fingers::spot(row, col) else { continue };
            if !include.iter().any(|g| g.matches(&spot)) {
                continue;
            }
            let code = keymap.get(0, row, col);
            let Some(basic) = keycodes::tap_basic(code) else { continue };
            let in_focus = focus.is_none_or(|g| g.matches(&spot));
            // A keycode that carries its own Shift (KC_EXLM and friends) already types the
            // shifted character, so that is its plain form.
            let carries_shift = keycodes::adds_shift(code);
            let base = host.char_for(basic, carries_shift);
            if let Some(c) = base {
                add(&mut letters, Letter { ch: c, hand: Some(spot.hand), shifted: carries_shift, focus: in_focus });
            }
            if drill.shift != Shift::Never
                && !carries_shift
                && let Some(c) = host.char_for(basic, true)
                && Some(c) != base
            {
                add(&mut letters, Letter { ch: c, hand: Some(spot.hand), shifted: true, focus: in_focus });
            }
        }
    }
    // Words need separators, so space is always available — which drills the thumbs for free.
    if !letters.iter().any(|l| l.ch == ' ') {
        let hand = hint::resolve(keymap, host, ' ').and_then(|p| fingers::spot(p.key.0, p.key.1)).map(|s| s.hand);
        letters.push(Letter { ch: ' ', hand, shifted: false, focus: false });
    }
    Alphabet { letters }
}

fn add(letters: &mut Vec<Letter>, letter: Letter) {
    match letters.iter_mut().find(|l| l.ch == letter.ch) {
        Some(existing) => existing.focus |= letter.focus,
        None => letters.push(letter),
    }
}

pub fn batch(drill: &Drill, keymap: &Keymap, host: HostLayout, rng: &mut impl RngExt) -> Result<Batch, StartError> {
    let (text, note) = match drill.source {
        Source::Tokens(tokens) => tokens_text(drill, tokens, keymap, host, rng)?,
        Source::Keys { .. } => keys_text(drill, &alphabet(drill, keymap, host), rng)?,
    };
    let target: Vec<char> = text.chars().collect();
    let paths = target.iter().map(|&c| hint::resolve(keymap, host, c)).collect();
    Ok(Batch { target, paths, note })
}

/// Split out from `batch` so tests can hand in an alphabet the reference keymap never produces.
pub fn keys_text(drill: &Drill, alpha: &Alphabet, rng: &mut impl RngExt) -> Result<(String, Option<&'static str>), StartError> {
    let letters = alpha.typeable();
    let focus = alpha.focus_chars();
    if letters.len() < MIN_LETTERS || focus.len() < MIN_FOCUS {
        return Err(StartError::TooFewKeys { drill: drill.name, found: letters.len() });
    }
    let pool = if drill.style == Style::Words { word_pool(alpha) } else { Vec::new() };
    let (words, note) = match drill.style {
        Style::Words if pool.len() >= MIN_WORD_POOL => (true, None),
        Style::Words => (false, Some(SYLLABLE_NOTE)),
        Style::Syllables => (false, None),
    };
    let mut text = String::new();
    while text.chars().count() < BATCH_CHARS {
        if !text.is_empty() {
            text.push(' ');
        }
        let item = if words { word_item(&pool, drill, rng) } else { syllable_item(alpha, drill, &focus, rng) };
        text.push_str(&item);
    }
    Ok((text, note))
}

fn tokens_text(
    drill: &Drill,
    tokens: &'static [&'static str],
    keymap: &Keymap,
    host: HostLayout,
    rng: &mut impl RngExt,
) -> Result<(String, Option<&'static str>), StartError> {
    let usable: Vec<&'static str> =
        tokens.iter().copied().filter(|t| t.chars().all(|c| hint::resolve(keymap, host, c).is_some())).collect();
    if usable.is_empty() {
        return Err(StartError::NoTokens { drill: drill.name });
    }
    let mut text = String::new();
    while text.chars().count() < BATCH_CHARS {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(usable[rng.random_range(0..usable.len())]);
    }
    Ok((text, None))
}

fn word_pool(alpha: &Alphabet) -> Vec<&'static str> {
    let chars: Vec<char> = alpha.chars().collect();
    let focus = alpha.focus_chars();
    WORDS
        .split_whitespace()
        .filter(|w| w.chars().all(|c| chars.contains(&c)))
        .filter(|w| w.chars().any(|c| focus.contains(&c)))
        .collect()
}

fn word_item(pool: &[&'static str], drill: &Drill, rng: &mut impl RngExt) -> String {
    let word = pool[rng.random_range(0..pool.len())];
    match drill.shift {
        Shift::Required => capitalise(word, rng),
        Shift::Allowed if rng.random_range(0..5) == 0 => capitalise(word, rng),
        _ => word.to_owned(),
    }
}

/// One of the shapes a programmer's hands meet every day: a shout, a leading capital, or a
/// capital in the middle of a name.
fn capitalise(word: &str, rng: &mut impl RngExt) -> String {
    let chars: Vec<char> = word.chars().collect();
    let shape = if chars.len() < 2 { 0 } else { rng.random_range(0..3) };
    match shape {
        0 => word.to_uppercase(),
        1 => chars.iter().enumerate().map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { *c }).collect(),
        _ => {
            let at = rng.random_range(1..chars.len());
            chars.iter().enumerate().map(|(i, c)| if i == at { c.to_ascii_uppercase() } else { *c }).collect()
        }
    }
}

fn syllable_item(alpha: &Alphabet, drill: &Drill, focus: &[char], rng: &mut impl RngExt) -> String {
    const LENGTHS: [usize; 7] = [2, 3, 3, 4, 4, 5, 6];
    let letters = alpha.typeable();
    let len = LENGTHS[rng.random_range(0..LENGTHS.len())];
    let mut out: Vec<char> = Vec::with_capacity(len);
    let (mut prev, mut prev_hand): (Option<char>, Option<Hand>) = (None, None);
    for _ in 0..len {
        // Alternate hands most of the time, so a group flows instead of hammering one hand.
        let alternate = rng.random_range(0..10) < 6;
        let mut choices: Vec<Letter> = letters
            .iter()
            .copied()
            .filter(|l| Some(l.ch) != prev)
            .filter(|l| !alternate || prev_hand.is_none() || l.hand != prev_hand)
            .collect();
        if choices.is_empty() {
            choices = letters.iter().copied().filter(|l| Some(l.ch) != prev).collect();
        }
        if choices.is_empty() {
            choices.clone_from(&letters);
        }
        let pick = choices[rng.random_range(0..choices.len())];
        out.push(pick.ch);
        (prev, prev_hand) = (Some(pick.ch), pick.hand);
    }
    // Make sure the group exercises the drill, and uses Shift when the drill is about Shift.
    if !focus.is_empty() && !out.iter().any(|c| focus.contains(c)) {
        let at = rng.random_range(0..out.len());
        out[at] = focus[rng.random_range(0..focus.len())];
    }
    if drill.shift == Shift::Required && !out.iter().any(|&c| is_shifted(alpha, c)) {
        let shifted: Vec<char> = alpha.letters.iter().filter(|l| l.shifted).map(|l| l.ch).collect();
        if !shifted.is_empty() {
            let at = rng.random_range(0..out.len());
            out[at] = shifted[rng.random_range(0..shifted.len())];
        }
    }
    out.into_iter().collect()
}

fn is_shifted(alpha: &Alphabet, c: char) -> bool {
    alpha.letters.iter().any(|l| l.ch == c && l.shifted)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked tutor::generate`
Expected: PASS, 8 tests.

Run: `cargo test --locked` and `cargo clippy --locked --all-targets -- -D warnings`
Expected: everything passes, no warnings.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/tutor/generate.rs src/tutor/words.txt src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: generate drill batches from the live keymap

Resolves each drill's key groups into the characters this keymap actually types,
then builds a batch of words or letter groups from them. Letter groups avoid
repeating a key and prefer alternating hands; a Words drill whose filtered pool
is too thin falls back to letter groups and says so rather than serving
gibberish where words were promised.

Adds rand 0.10 rather than a hand-rolled PRNG: small generators are subtle, and
a slightly wrong one feels off in a way that is hard to diagnose. Generators
take &mut impl RngExt, so tests seed a constant and assert exact output.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 7: Scoring

**Files:**
- Create: `src/tutor/score.rs`
- Modify: `src/tutor/mod.rs` (add the module)

**Interfaces:**
- Consumes: `fingers::{spot, Hand, Finger}` (Task 1), `generate::Batch` (Task 6).
- Produces: `score::{Verdict, Attempt, Summary, Totals, wrap}`. `Attempt::{new, type_char, backspace, pause, verdict, cursor, typed, is_complete, keystrokes, mistakes, elapsed, accuracy, wpm, summarise}`; `wrap(&[char], usize) -> Vec<Range<usize>>`. Tasks 8 and 10 use all of it.

- [ ] **Step 1: Write the failing test**

Create `src/tutor/score.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostlayout::HostLayout;
    use crate::tutor::fixture::reference_keymap;
    use crate::tutor::hint;

    fn batch_for(text: &str) -> Batch {
        let km = reference_keymap();
        let target: Vec<char> = text.chars().collect();
        let paths = target.iter().map(|&c| hint::resolve(&km, HostLayout::Gb, c)).collect();
        Batch { target, paths, note: None }
    }

    /// Backspacing fixes the screen but not the tally: the mistake still happened.
    #[test]
    fn a_correction_does_not_erase_the_mistake() {
        let target: Vec<char> = "as".chars().collect();
        let t0 = Instant::now();
        let mut a = Attempt::new(target.len());
        a.type_char(&target, 'a', t0);
        a.type_char(&target, 'x', t0 + Duration::from_millis(200));
        a.backspace();
        a.type_char(&target, 's', t0 + Duration::from_millis(400));
        assert!(a.is_complete(&target));
        assert_eq!(a.verdict(&target, 1), Verdict::Correct, "the screen shows the correction");
        assert_eq!((a.keystrokes(), a.mistakes()), (3, 1), "backspace is not a keystroke");
        assert!((a.accuracy() - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn verdicts_follow_the_cursor() {
        let target: Vec<char> = "abc".chars().collect();
        let t0 = Instant::now();
        let mut a = Attempt::new(target.len());
        a.type_char(&target, 'a', t0);
        a.type_char(&target, 'z', t0 + Duration::from_millis(100));
        assert_eq!(a.verdict(&target, 0), Verdict::Correct);
        assert_eq!(a.verdict(&target, 1), Verdict::Wrong);
        assert_eq!(a.verdict(&target, 2), Verdict::Cursor);
        assert_eq!(a.cursor(), 2);
    }

    /// Staring at an unfamiliar symbol costs five seconds, not a minute; time spent in another
    /// window costs nothing at all, which is how "only while focused" shows up in the score.
    #[test]
    fn the_clock_caps_idling_and_ignores_time_away() {
        let target: Vec<char> = "abc".chars().collect();
        let t0 = Instant::now();
        let mut a = Attempt::new(target.len());
        a.type_char(&target, 'a', t0);
        assert_eq!(a.elapsed(), Duration::ZERO, "the clock starts when typing starts");
        a.type_char(&target, 'b', t0 + Duration::from_secs(60));
        assert_eq!(a.elapsed(), Duration::from_secs(5));
        a.pause();
        a.type_char(&target, 'c', t0 + Duration::from_secs(600));
        assert_eq!(a.elapsed(), Duration::from_secs(5));
    }

    #[test]
    fn the_summary_blames_the_finger_that_missed() {
        let batch = batch_for("qa");
        let t0 = Instant::now();
        let mut a = Attempt::new(batch.target.len());
        a.type_char(&batch.target, 'x', t0); // should have been q: the left pinky
        a.type_char(&batch.target, 'a', t0 + Duration::from_millis(100));
        let s = a.summarise(&batch);
        assert_eq!(s.mistakes, 1);
        assert_eq!(s.worst_finger, Some((Hand::Left, Finger::Pinky, 1)));
        assert_eq!(s.worst_chars, vec![('q', 1)]);
    }

    #[test]
    fn words_per_minute_counts_correct_characters_in_fives() {
        let target: Vec<char> = "abcdefghij".chars().collect();
        let t0 = Instant::now();
        let mut a = Attempt::new(target.len());
        // Ten correct characters at one every 200 ms: 1.8 s of typing after the first keystroke.
        for (i, c) in target.iter().enumerate() {
            a.type_char(&target, *c, t0 + Duration::from_millis(200 * i as u64));
        }
        assert_eq!(a.elapsed(), Duration::from_millis(1800));
        assert!((a.wpm() - (10.0 / 5.0) / (1.8 / 60.0)).abs() < 0.01, "{}", a.wpm());
    }

    #[test]
    fn lines_break_at_spaces_and_cover_the_whole_target() {
        let target: Vec<char> = "asdf jkl fdsa lkj asdfg".chars().collect();
        let lines = wrap(&target, 10);
        let joined: String = lines.iter().flat_map(|r| target[r.clone()].iter()).collect();
        assert_eq!(joined, "asdf jkl fdsa lkj asdfg", "every character lands on exactly one line");
        assert!(lines.iter().all(|r| r.len() <= 10), "{lines:?}");
        assert_eq!(target[lines[0].clone()].iter().collect::<String>(), "asdf jkl ");
    }

    #[test]
    fn an_unbroken_run_is_cut_at_the_width() {
        let target: Vec<char> = "aaaaaaaaaaaa".chars().collect();
        assert_eq!(wrap(&target, 5), vec![0..5, 5..10, 10..12]);
    }

    #[test]
    fn totals_accumulate_across_batches() {
        let batch = batch_for("as");
        let t0 = Instant::now();
        let mut totals = Totals::default();
        for _ in 0..2 {
            let mut a = Attempt::new(batch.target.len());
            a.type_char(&batch.target, 'a', t0);
            a.type_char(&batch.target, 'x', t0 + Duration::from_millis(500));
            totals.add(&a.summarise(&batch));
        }
        assert_eq!((totals.batches, totals.keystrokes, totals.mistakes), (2, 4, 2));
        assert!((totals.accuracy() - 0.5).abs() < 1e-6);
    }
}
```

Add to `src/tutor/mod.rs`:

```rust
pub mod score;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked tutor::score`
Expected: FAIL to compile — `cannot find type Attempt`.

- [ ] **Step 3: Write the implementation**

Put this above the test module in `src/tutor/score.rs`:

```rust
//! What was typed against what was asked for, and what that scores.

use std::ops::Range;
use std::time::{Duration, Instant};

use crate::tutor::fingers::{self, Finger, Hand};
use crate::tutor::generate::Batch;

/// A gap longer than this isn't counted. Looking up an unfamiliar symbol shouldn't wreck the
/// words-per-minute, and nor should answering the door.
const IDLE_CAP: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Correct,
    Wrong,
    Cursor,
    Pending,
}

#[derive(Debug, Clone, Default)]
pub struct Attempt {
    typed: Vec<char>,
    /// Per target position: was it ever typed wrong? This latches, so backspacing and retyping
    /// fixes the screen but not the tally — the mistake still happened.
    wrong: Vec<bool>,
    keystrokes: u32,
    mistakes: u32,
    elapsed: Duration,
    last: Option<Instant>,
}

impl Attempt {
    pub fn new(len: usize) -> Attempt {
        Attempt { wrong: vec![false; len], ..Attempt::default() }
    }

    pub fn cursor(&self) -> usize {
        self.typed.len()
    }

    pub fn typed(&self) -> &[char] {
        &self.typed
    }

    pub fn keystrokes(&self) -> u32 {
        self.keystrokes
    }

    pub fn mistakes(&self) -> u32 {
        self.mistakes
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn is_complete(&self, target: &[char]) -> bool {
        self.typed.len() >= target.len()
    }

    pub fn type_char(&mut self, target: &[char], c: char, now: Instant) {
        if self.is_complete(target) {
            return;
        }
        if let Some(last) = self.last {
            self.elapsed += now.saturating_duration_since(last).min(IDLE_CAP);
        }
        self.last = Some(now);
        let at = self.typed.len();
        self.keystrokes += 1;
        if target[at] != c {
            self.mistakes += 1;
            self.wrong[at] = true;
        }
        self.typed.push(c);
    }

    /// Backspace isn't a keystroke, by convention, so correcting doesn't dilute accuracy.
    pub fn backspace(&mut self) {
        self.typed.pop();
    }

    /// The window lost focus. The gap until the next keystroke isn't typing time, so don't
    /// measure it at all.
    pub fn pause(&mut self) {
        self.last = None;
    }

    pub fn verdict(&self, target: &[char], at: usize) -> Verdict {
        match at.cmp(&self.typed.len()) {
            std::cmp::Ordering::Less if self.typed[at] == target[at] => Verdict::Correct,
            std::cmp::Ordering::Less => Verdict::Wrong,
            std::cmp::Ordering::Equal => Verdict::Cursor,
            std::cmp::Ordering::Greater => Verdict::Pending,
        }
    }

    pub fn accuracy(&self) -> f32 {
        if self.keystrokes == 0 {
            return 1.0;
        }
        (self.keystrokes - self.mistakes) as f32 / self.keystrokes as f32
    }

    pub fn wpm(&self) -> f32 {
        wpm(self.keystrokes - self.mistakes, self.elapsed)
    }

    pub fn summarise(&self, batch: &Batch) -> Summary {
        let mut fingers_missed: Vec<((Hand, Finger), u32)> = Vec::new();
        let mut chars_missed: Vec<(char, u32)> = Vec::new();
        for (at, _) in self.wrong.iter().enumerate().filter(|&(_, &w)| w) {
            if let Some(spot) = batch.paths.get(at).and_then(Option::as_ref).and_then(|p| fingers::spot(p.key.0, p.key.1)) {
                tally(&mut fingers_missed, (spot.hand, spot.finger));
            }
            if let Some(&c) = batch.target.get(at) {
                tally(&mut chars_missed, c);
            }
        }
        fingers_missed.sort_by_key(|a| std::cmp::Reverse(a.1));
        chars_missed.sort_by_key(|a| std::cmp::Reverse(a.1));
        chars_missed.truncate(3);
        Summary {
            chars: batch.target.len(),
            keystrokes: self.keystrokes,
            mistakes: self.mistakes,
            accuracy: self.accuracy(),
            wpm: self.wpm(),
            elapsed: self.elapsed,
            worst_finger: fingers_missed.first().map(|&((hand, finger), n)| (hand, finger, n)),
            worst_chars: chars_missed,
        }
    }
}

fn tally<T: PartialEq>(counts: &mut Vec<(T, u32)>, key: T) {
    match counts.iter_mut().find(|(k, _)| *k == key) {
        Some((_, n)) => *n += 1,
        None => counts.push((key, 1)),
    }
}

fn wpm(correct: u32, elapsed: Duration) -> f32 {
    let minutes = elapsed.as_secs_f32() / 60.0;
    if minutes <= 0.0 {
        return 0.0;
    }
    (correct as f32 / 5.0) / minutes
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub chars: usize,
    pub keystrokes: u32,
    pub mistakes: u32,
    pub accuracy: f32,
    pub wpm: f32,
    pub elapsed: Duration,
    pub worst_finger: Option<(Hand, Finger, u32)>,
    pub worst_chars: Vec<(char, u32)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub batches: u32,
    pub keystrokes: u32,
    pub mistakes: u32,
    pub elapsed: Duration,
}

impl Totals {
    pub fn add(&mut self, summary: &Summary) {
        self.batches += 1;
        self.keystrokes += summary.keystrokes;
        self.mistakes += summary.mistakes;
        self.elapsed += summary.elapsed;
    }

    pub fn accuracy(&self) -> f32 {
        if self.keystrokes == 0 {
            return 1.0;
        }
        (self.keystrokes - self.mistakes) as f32 / self.keystrokes as f32
    }

    pub fn wpm(&self) -> f32 {
        wpm(self.keystrokes - self.mistakes, self.elapsed)
    }
}

/// Splits `target` into display lines of at most `width` characters, breaking after spaces.
///
/// The ranges index `target`, so the typed line is sliced the same way and the two stay on one
/// character grid. Letting egui wrap them separately would break them at different points,
/// because their contents differ, and they would drift out of alignment.
pub fn wrap(target: &[char], width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut start = 0;
    while start < target.len() {
        if target.len() - start <= width {
            lines.push(start..target.len());
            break;
        }
        let limit = start + width;
        let brk = target[start..limit].iter().rposition(|&c| c == ' ').map_or(limit, |i| start + i + 1);
        lines.push(start..brk);
        start = brk;
    }
    lines
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked tutor::score`
Expected: PASS, 8 tests.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tutor/score.rs src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: score a typing attempt

Per-position verdicts, latched mistakes so a correction fixes the screen but not
the tally, and a clock with two rules: an idle gap counts for at most five
seconds, and time after the window loses focus isn't counted at all. Per-finger
blame reuses the batch's precomputed paths. wrap() does the line breaking so the
target and typed lines stay on one character grid.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 8: The session state machine

**Files:**
- Modify: `src/tutor/mod.rs` (add `Session` and friends above the module declarations)

**Interfaces:**
- Consumes: Tasks 1, 5, 6, 7.
- Produces: `tutor::{Session, Phase, Input, Availability}`. `Session::{new, set_availability, available, is_active, phase, toggle, close, selected, select, start, input, keyboard_changed, hint, hints_on, totals}`. Task 10 drives all of it; Task 9 uses `hint`.

- [ ] **Step 1: Write the failing test**

Add to `src/tutor/mod.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tutor::fixture::reference_keymap;

    fn session() -> (Session, Keymap) {
        let mut s = Session::new();
        s.set_availability(Availability::Ready);
        (s, reference_keymap())
    }

    /// Types a whole batch correctly. The borrow of `s.phase()` has to end before `s.input`,
    /// so the next character is read out in its own step rather than in a `while let`.
    fn type_out(s: &mut Session, km: &Keymap) {
        let mut now = Instant::now();
        loop {
            let next = match s.phase() {
                Phase::Typing { batch, attempt, .. } => batch.target.get(attempt.cursor()).copied(),
                _ => None,
            };
            let Some(c) = next else { return };
            now += Duration::from_millis(100);
            s.input(Input::Char(c), km, HostLayout::Gb, now);
        }
    }

    #[test]
    fn the_tutor_opens_on_the_drill_picker_and_escape_backs_out() {
        let (mut s, km) = session();
        assert!(!s.is_active());
        s.toggle();
        assert!(matches!(s.phase(), Phase::Choosing));
        s.start(0, &km, HostLayout::Gb).unwrap();
        assert!(matches!(s.phase(), Phase::Typing { .. }));
        s.input(Input::Escape, &km, HostLayout::Gb, Instant::now());
        assert!(matches!(s.phase(), Phase::Choosing));
        s.input(Input::Escape, &km, HostLayout::Gb, Instant::now());
        assert!(matches!(s.phase(), Phase::Off));
    }

    #[test]
    fn finishing_a_batch_scores_it_and_enter_starts_another() {
        let (mut s, km) = session();
        s.toggle();
        s.start(0, &km, HostLayout::Gb).unwrap();
        type_out(&mut s, &km);
        let Phase::Done { summary, .. } = s.phase() else { panic!("a finished batch is Done") };
        assert_eq!(summary.mistakes, 0);
        assert_eq!(s.totals().batches, 1);
        s.input(Input::Enter, &km, HostLayout::Gb, Instant::now());
        assert!(matches!(s.phase(), Phase::Typing { .. }), "Enter starts the next batch");
    }

    #[test]
    fn backspace_and_focus_loss_reach_the_attempt() {
        let (mut s, km) = session();
        s.toggle();
        s.start(0, &km, HostLayout::Gb).unwrap();
        let now = Instant::now();
        s.input(Input::Char('!'), &km, HostLayout::Gb, now); // certainly wrong for a home-keys drill
        s.input(Input::Backspace, &km, HostLayout::Gb, now);
        s.input(Input::FocusLost, &km, HostLayout::Gb, now);
        let Phase::Typing { attempt, .. } = s.phase() else { panic!("still typing") };
        assert_eq!(attempt.cursor(), 0);
        assert_eq!(attempt.mistakes(), 1, "the mistake is still counted");
    }

    /// A freshly read keymap invalidates the batch's precomputed paths. This fires on Reload and
    /// when Vial releases the keyboard, which is how a remap reaches the drills.
    #[test]
    fn a_re_read_keymap_abandons_the_batch() {
        let (mut s, km) = session();
        s.toggle();
        s.start(0, &km, HostLayout::Gb).unwrap();
        s.keyboard_changed();
        assert!(matches!(s.phase(), Phase::Choosing));
    }

    #[test]
    fn losing_the_keyboard_closes_the_tutor() {
        let (mut s, km) = session();
        s.toggle();
        s.start(0, &km, HostLayout::Gb).unwrap();
        s.set_availability(Availability::NoKeyboard);
        assert!(matches!(s.phase(), Phase::Off));
        s.toggle();
        assert!(matches!(s.phase(), Phase::Off), "it can't be opened without a keyboard");
    }

    #[test]
    fn hints_point_at_the_next_character_and_can_be_turned_off() {
        let (mut s, km) = session();
        s.toggle();
        s.start(0, &km, HostLayout::Gb).unwrap();
        let Phase::Typing { batch, .. } = s.phase() else { panic!("typing") };
        let expected = batch.paths[0].clone();
        assert_eq!(s.hint().cloned(), expected);
        s.hints_on = false;
        assert_eq!(s.hint(), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --locked tutor::tests`
Expected: FAIL to compile — `cannot find type Session`.

- [ ] **Step 3: Write the implementation**

Replace the contents of `src/tutor/mod.rs` above the test module with:

```rust
//! Typing tutor mode: drills generated from the keyboard's own keymap.

pub mod drills;
pub mod fingers;
pub mod generate;
pub mod hint;
pub mod score;

#[cfg(test)]
pub mod fixture;

use std::time::Instant;

use rand::rngs::StdRng;

use crate::hostlayout::HostLayout;
use crate::keymap::Keymap;
use crate::tutor::drills::DrillId;
use crate::tutor::generate::{Batch, StartError};
use crate::tutor::hint::KeyPath;
use crate::tutor::score::{Attempt, Summary, Totals};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Ready,
    NoKeyboard,
    /// The keyboard's layout doesn't match the built-in finger map.
    LayoutMismatch(String),
}

impl Availability {
    /// Why the tutor can't run, for the disabled button.
    pub fn reason(&self) -> Option<String> {
        match self {
            Availability::Ready => None,
            Availability::NoKeyboard => Some("The typing tutor needs the keyboard picture.".into()),
            Availability::LayoutMismatch(why) => Some(why.clone()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Phase {
    Off,
    Choosing,
    Typing { drill: DrillId, batch: Batch, attempt: Attempt },
    Done { drill: DrillId, batch: Batch, summary: Summary },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Char(char),
    Backspace,
    Enter,
    Escape,
    FocusLost,
}

pub struct Session {
    phase: Phase,
    rng: StdRng,
    available: Availability,
    selected: DrillId,
    totals: Totals,
    /// Why the last attempt to start a drill failed. Recorded by `start` itself, so every route
    /// into it — a picker button, Enter, a restart — reports failure the same way, and no caller
    /// can drop it on the floor.
    start_error: Option<StartError>,
    pub hints_on: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session {
            phase: Phase::Off,
            rng: rand::make_rng::<StdRng>(),
            available: Availability::NoKeyboard,
            selected: 0,
            totals: Totals::default(),
            start_error: None,
            hints_on: true,
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.phase, Phase::Off)
    }

    pub fn available(&self) -> &Availability {
        &self.available
    }

    pub fn totals(&self) -> &Totals {
        &self.totals
    }

    pub fn start_error(&self) -> Option<&StartError> {
        self.start_error.as_ref()
    }

    pub fn selected(&self) -> DrillId {
        self.selected
    }

    pub fn select(&mut self, id: DrillId) {
        self.selected = id;
    }

    /// Set by `App` when the keyboard arrives or goes. Anything but `Ready` closes the tutor:
    /// without the picture there are no colours and no hints, and it isn't worth pretending.
    pub fn set_availability(&mut self, available: Availability) {
        if available != Availability::Ready {
            self.phase = Phase::Off;
        }
        self.available = available;
    }

    pub fn toggle(&mut self) {
        self.phase = match (&self.phase, &self.available) {
            (Phase::Off, Availability::Ready) => Phase::Choosing,
            (Phase::Off, _) => Phase::Off,
            _ => Phase::Off,
        };
    }

    pub fn close(&mut self) {
        self.phase = Phase::Off;
    }

    pub fn start(&mut self, id: DrillId, keymap: &Keymap, host: HostLayout) -> Result<(), StartError> {
        let batch = match generate::batch(drills::drill(id), keymap, host, &mut self.rng) {
            Ok(batch) => batch,
            Err(problem) => {
                self.start_error = Some(problem.clone());
                return Err(problem);
            }
        };
        self.start_error = None;
        let attempt = Attempt::new(batch.target.len());
        self.selected = id;
        self.phase = Phase::Typing { drill: id, batch, attempt };
        Ok(())
    }

    /// A freshly read keymap invalidates a batch's precomputed paths, so the batch is abandoned.
    /// This happens on Reload and when another program releases the keyboard — which is how a
    /// remap in Vial reaches the drills without restarting the app.
    pub fn keyboard_changed(&mut self) {
        if matches!(self.phase, Phase::Typing { .. } | Phase::Done { .. }) {
            self.phase = Phase::Choosing;
        }
    }

    pub fn hint(&self) -> Option<&KeyPath> {
        if !self.hints_on {
            return None;
        }
        let Phase::Typing { batch, attempt, .. } = &self.phase else { return None };
        batch.paths.get(attempt.cursor())?.as_ref()
    }

    pub fn input(&mut self, input: Input, keymap: &Keymap, host: HostLayout, now: Instant) {
        let mut finished: Option<Phase> = None;
        let mut restart: Option<DrillId> = None;
        match (&mut self.phase, input) {
            (Phase::Typing { drill, batch, attempt }, Input::Char(c)) => {
                attempt.type_char(&batch.target, c, now);
                if attempt.is_complete(&batch.target) {
                    let summary = attempt.summarise(batch);
                    self.totals.add(&summary);
                    finished = Some(Phase::Done { drill: *drill, batch: batch.clone(), summary });
                }
            }
            (Phase::Typing { attempt, .. }, Input::Backspace) => attempt.backspace(),
            (Phase::Typing { attempt, .. }, Input::FocusLost) => attempt.pause(),
            (Phase::Done { drill, .. }, Input::Enter) => restart = Some(*drill),
            (Phase::Choosing, Input::Enter) => restart = Some(self.selected),
            (Phase::Typing { .. } | Phase::Done { .. }, Input::Escape) => self.phase = Phase::Choosing,
            (Phase::Choosing, Input::Escape) => self.phase = Phase::Off,
            _ => {}
        }
        if let Some(phase) = finished {
            self.phase = phase;
        }
        if let Some(id) = restart {
            let _ = self.start(id, keymap, host);
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --locked tutor`
Expected: PASS — every tutor test, 6 new ones here.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: the typing tutor session state machine

Off / Choosing / Typing / Done, driven by a small Input enum so nothing here
depends on egui. Holds no keyboard data: callers pass the keymap at the call
site, leaving AppState the single source of truth. A re-read keymap abandons the
batch in progress; losing the keyboard closes the tutor.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 9: Finger colours and hints on the keyboard picture

**Files:**
- Modify: `src/ui/keyboard.rs` (the `show` signature, the per-key fill and stroke)
- Modify: `src/ui/mod.rs:237` (the one existing `keyboard::show` call, inside `App::central`)
- Modify: `src/tutor/mod.rs` (add `colours_on`, `stage`, `again` to `Session` — see Step 1)

**Interfaces:**
- Consumes: `fingers::{spot, Hand, Finger}` (Task 1), `hint::KeyPath` (Task 4).
- Produces: `keyboard::View<'a> { unlock_keys, fingers, hint }` and `keyboard::finger_colour(Hand, Finger) -> Color32`. Task 10 builds the `View`. Also `Session::{colours_on, stage, again}` and `tutor::Stage`, which Task 10 needs.

- [ ] **Step 1: Add the three `Session` members Task 10 needs**

In `src/tutor/mod.rs`, add above `Session`:

```rust
/// Which phase the session is in, without borrowing its contents — so a UI function can match
/// on it and then take `&mut` to act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Off,
    Choosing,
    Typing,
    Done,
}
```

Add `colours_on` to the struct, beside `hints_on`:

```rust
    pub hints_on: bool,
    pub colours_on: bool,
```

Set it in `Session::new`, after `hints_on: true,`:

```rust
            colours_on: true,
```

And add these two methods to `impl Session`, after `is_active`:

```rust
    pub fn stage(&self) -> Stage {
        match self.phase {
            Phase::Off => Stage::Off,
            Phase::Choosing => Stage::Choosing,
            Phase::Typing { .. } => Stage::Typing,
            Phase::Done { .. } => Stage::Done,
        }
    }

    /// Retype the same text: a fresh attempt over the batch just finished.
    pub fn again(&mut self) {
        if let Phase::Done { drill, batch, .. } = &self.phase {
            let attempt = Attempt::new(batch.target.len());
            self.phase = Phase::Typing { drill: *drill, batch: batch.clone(), attempt };
        }
    }
```

- [ ] **Step 2: Write the failing test**

Add to the existing `mod tests` in `src/ui/keyboard.rs`:

```rust
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
```

Also add, to the same test module, a compile-level check that the new signature is what Task 10 expects:

```rust
    #[test]
    fn a_default_view_highlights_nothing() {
        let view = View::default();
        assert!(view.unlock_keys.is_empty());
        assert!(!view.fingers);
        assert!(view.hint.is_none());
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --locked ui::keyboard`
Expected: FAIL to compile — `cannot find function finger_colour`, `cannot find type View`.

- [ ] **Step 4: Write the implementation**

In `src/ui/keyboard.rs`, add to the imports:

```rust
use crate::tutor::fingers::{self, Finger, Hand};
use crate::tutor::hint::KeyPath;
```

Add beside the existing `HELD` and `UNLOCK` constants:

```rust
/// The key to press next, and the keys to hold to reach it.
const HINT: Color32 = Color32::from_rgb(120, 200, 120);
const HINT_HOLD: Color32 = Color32::from_rgb(170, 215, 170);
/// Thicker than the plain hairline, so a finger group reads as a group.
const FINGER_STROKE: f32 = 2.0;

/// What to highlight on the picture besides the keys being pressed.
#[derive(Default)]
pub struct View<'a> {
    pub unlock_keys: &'a [(u8, u8)],
    /// Outline each key in its finger's colour.
    pub fingers: bool,
    pub hint: Option<&'a KeyPath>,
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
```

Change the signature of `show`:

```rust
pub fn show(ui: &mut egui::Ui, state: &AppState, now: Instant, view: View<'_>) {
```

Inside `show`, just before the `for key in &layout.keys` loop, add:

```rust
    let hint_key = view.hint.map(|h| h.key);
    let hint_hold: &[(u8, u8)] = view.hint.map_or(&[], |h| &h.hold);
```

Replace the body of the loop from `let (is_held, is_unlock) = ...` down to the `painter.add(...)` line with:

```rust
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
        let points: Vec<Pos2> = inner.corners().into_iter().map(to_screen).collect();
        painter.add(Shape::convex_polygon(points, fill, stroke));
```

In `src/ui/mod.rs`, update the single call inside `App::central`. `App` has no `tutor` field
until Task 10, so this task passes the defaults and Task 10 fills them in:

```rust
            keyboard::show(ui, &self.state, now, keyboard::View { unlock_keys: self.unlock_highlight(), ..Default::default() });
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --locked`
Expected: PASS, including the existing `keycaps_show_what_the_key_types`.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/ui/keyboard.rs src/ui/mod.rs src/tutor/mod.rs
git commit -m "$(cat <<'MSG'
feat: finger colours and next-key hints on the keyboard picture

Ten colours, one per finger per hand, drawn as the key outline because the fills
are already spoken for by held and unlock. The hinted key and the keys to hold
fill green, below held in precedence, so a hinted key turns blue the moment it
is pressed.

keyboard::show now takes a View struct rather than growing a fifth and sixth
argument.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 10: The tutor panel and the app wiring

**Files:**
- Create: `src/ui/tutor_panel.rs`
- Modify: `src/input/focused.rs` (add `FocusedInput::Command`)
- Modify: `src/ui/mod.rs` (the `tutor` field, `logic` routing, Ctrl+T, the top panel, availability)
- Modify: `src/ui/status.rs` (the tutor button)

**Interfaces:**
- Consumes: everything from Tasks 1–9.
- Produces: the working feature. No further task depends on its interfaces.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `src/input/focused.rs`:

```rust
    /// The tutor's controls. Modifier combinations are left alone, so Ctrl+R and Ctrl+T still
    /// reach the app while a batch is being typed.
    #[test]
    fn bare_backspace_enter_and_escape_are_reported_as_commands() {
        let out = translate(&[key(Key::Backspace, true, false), key(Key::Enter, true, false), key(Key::Escape, true, false)]);
        let commands: Vec<Key> = out
            .iter()
            .filter_map(|i| match i {
                FocusedInput::Command(k) => Some(*k),
                _ => None,
            })
            .collect();
        assert_eq!(commands, vec![Key::Backspace, Key::Enter, Key::Escape]);
        assert_eq!(out.len(), 6, "each one is still tracked as a key press too");
    }

    #[test]
    fn modified_and_released_keys_are_not_commands() {
        let ctrl = Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        };
        let released = key(Key::Escape, false, false);
        let out = translate(&[ctrl, released]);
        assert!(!out.iter().any(|i| matches!(i, FocusedInput::Command(_))));
    }
```

Add to the existing `mod tests` in `src/ui/mod.rs`:

```rust
    /// The tutor is unavailable until the keyboard's layout has been checked against the finger
    /// map, and goes away again when the keyboard does.
    #[test]
    fn the_tutor_follows_the_keyboard() {
        let (mut app, _events, _commands) = app(None);
        assert_eq!(app.tutor.available(), &crate::tutor::Availability::NoKeyboard);
        app.on_device_event(connected(), Instant::now());
        // The 2x3 test keyboard is not a Lily58, so the finger map rejects it by design.
        assert!(matches!(app.tutor.available(), crate::tutor::Availability::LayoutMismatch(_)));
        app.tutor.toggle();
        assert!(!app.tutor.is_active(), "it can't be opened against a layout it doesn't know");
        app.on_device_event(DeviceEvent::Disconnected, Instant::now());
        assert_eq!(app.tutor.available(), &crate::tutor::Availability::NoKeyboard);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --locked focused && cargo test --locked ui::tests`
Expected: FAIL to compile — `no variant named Command`, `no field tutor`.

- [ ] **Step 3: Add the `Command` input**

In `src/input/focused.rs`, add the variant to `FocusedInput`:

```rust
pub enum FocusedInput {
    Key(OsKey),
    Text(String),
    /// A bare Backspace, Enter or Escape: the typing tutor's controls. Modifier combinations are
    /// deliberately excluded, so Ctrl+R and Ctrl+T pass through untouched.
    Command(Key),
    /// The window lost keyboard focus, so releases of keys held now won't arrive.
    FocusLost,
}
```

Replace `translate` — one event can now produce two inputs, so it builds a `Vec` rather than
using `filter_map`:

```rust
pub fn translate(events: &[Event]) -> Vec<FocusedInput> {
    let mut out = Vec::new();
    for event in events {
        match event {
            Event::Key { key, physical_key, pressed, repeat: false, modifiers } => {
                let k: Key = physical_key.unwrap_or(*key);
                out.push(FocusedInput::Key(OsKey {
                    source: OsSource::Focused,
                    usages: egui_key_to_hid(k),
                    pressed: *pressed,
                    name: format!("{k:?}"),
                }));
                if *pressed && *modifiers == Modifiers::NONE && matches!(k, Key::Backspace | Key::Enter | Key::Escape) {
                    out.push(FocusedInput::Command(k));
                }
            }
            Event::Text(t) => out.push(FocusedInput::Text(t.clone())),
            Event::WindowFocused(false) => out.push(FocusedInput::FocusLost),
            _ => {}
        }
    }
    out
}
```

Add `Modifiers` to the imports at the top of the file:

```rust
use eframe::egui::{Event, Key, Modifiers};
```

The test module's own `use eframe::egui::Modifiers;` still compiles — an explicit import
shadows a glob import of the same item — so leave it alone.

- [ ] **Step 4: Write the panel**

Create `src/ui/tutor_panel.rs`:

```rust
//! The typing tutor's panel: the drill picker, the text being typed, and the result.

use std::ops::Range;

use eframe::egui::{self, Color32, FontId, RichText, TextFormat, text::LayoutJob};

use super::App;
use crate::tutor::score::{self, Attempt, Verdict};
use crate::tutor::{Phase, Stage, drills, generate};

/// The same red the status bar uses for errors.
const WRONG: Color32 = Color32::from_rgb(230, 90, 90);
/// Below this the text block is unreadable anyway, and a zero would divide badly.
const MIN_COLUMNS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Target,
    Typed,
}

pub(super) fn show(ui: &mut egui::Ui, app: &mut App) {
    match app.tutor.stage() {
        Stage::Off => {}
        Stage::Choosing => choosing(ui, app),
        Stage::Typing => typing(ui, app),
        Stage::Done => done(ui, app),
    }
}

fn header(ui: &mut egui::Ui, app: &mut App) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Typing tutor").strong());
        ui.separator();
        ui.checkbox(&mut app.tutor.hints_on, "Next-key hints");
        ui.checkbox(&mut app.tutor.colours_on, "Finger colours");
    });
}

fn choosing(ui: &mut egui::Ui, app: &mut App) {
    header(ui, app);
    // What each drill currently resolves to on the live keymap. Retune a key in Vial, close it,
    // and the new character shows up here without restarting.
    let mut rows: Vec<(drills::DrillId, String)> = Vec::new();
    if let Some(keymap) = &app.state.keymap {
        for id in drills::ids_of(drills::Kind::Position) {
            let alpha = generate::alphabet(drills::drill(id), keymap, app.state.host());
            rows.push((id, alpha.focus_chars().into_iter().collect()));
        }
    }
    let mut start = None;
    for (id, chars) in &rows {
        ui.horizontal(|ui| {
            if ui.button(drills::drill(*id).name).clicked() {
                start = Some(*id);
            }
            ui.label(RichText::new(chars.as_str()).monospace().weak());
        });
    }
    ui.horizontal_wrapped(|ui| {
        for id in drills::ids_of(drills::Kind::Programmer) {
            if ui.button(drills::drill(id).name).clicked() {
                start = Some(id);
            }
        }
        ui.separator();
        if ui.button("Close (Esc)").clicked() {
            app.tutor.close();
        }
    });
    if let Some(id) = start {
        app.start_drill(id);
    }
    if let Some(problem) = app.tutor.start_error() {
        ui.colored_label(WRONG, problem.to_string());
    }
}

fn typing(ui: &mut egui::Ui, app: &mut App) {
    let mut stop = false;
    {
        let Phase::Typing { drill, batch, attempt } = app.tutor.phase() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(drills::drill(*drill).name).strong());
            ui.separator();
            ui.label(format!(
                "{:.0}% · {:.0} wpm · {} errors",
                attempt.accuracy() * 100.0,
                attempt.wpm(),
                attempt.mistakes()
            ));
            if let Some(note) = batch.note {
                ui.separator();
                ui.label(RichText::new(note).weak());
            }
            ui.separator();
            if ui.button("Stop (Esc)").clicked() {
                stop = true;
            }
        });
        text_block(ui, &batch.target, attempt);
    }
    if stop {
        app.tutor.close();
    }
}

fn done(ui: &mut egui::Ui, app: &mut App) {
    let (mut again, mut next, mut stop) = (false, false, false);
    {
        let Phase::Done { drill, summary, .. } = app.tutor.phase() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(drills::drill(*drill).name).strong());
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "{:.0}% · {:.0} wpm · {} errors in {} characters",
                    summary.accuracy * 100.0,
                    summary.wpm,
                    summary.mistakes,
                    summary.chars
                ))
                .strong(),
            );
        });
        if let Some((hand, finger, missed)) = summary.worst_finger {
            let worst: String = summary.worst_chars.iter().map(|(c, _)| *c).collect();
            let tail = if worst.is_empty() { String::new() } else { format!(" — missed {worst}") };
            ui.label(format!("weakest: {} {} ({missed}){tail}", hand_name(hand), finger_name(finger)));
        }
        let totals = app.tutor.totals();
        ui.label(
            RichText::new(format!(
                "this session: {} batches · {:.0}% · {:.0} wpm",
                totals.batches,
                totals.accuracy() * 100.0,
                totals.wpm()
            ))
            .weak(),
        );
        ui.horizontal(|ui| {
            again = ui.button("Again (same text)").clicked();
            next = ui.button("Next batch (Enter)").clicked();
            stop = ui.button("Stop (Esc)").clicked();
        });
    }
    if again {
        app.tutor.again();
    } else if next {
        let id = app.tutor.selected();
        app.start_drill(id);
    } else if stop {
        app.tutor.close();
    }
}

fn hand_name(hand: crate::tutor::fingers::Hand) -> &'static str {
    match hand {
        crate::tutor::fingers::Hand::Left => "left",
        crate::tutor::fingers::Hand::Right => "right",
    }
}

fn finger_name(finger: crate::tutor::fingers::Finger) -> &'static str {
    use crate::tutor::fingers::Finger;
    match finger {
        Finger::Pinky => "little finger",
        Finger::Ring => "ring finger",
        Finger::Middle => "middle finger",
        Finger::Index => "index finger",
        Finger::Thumb => "thumb",
    }
}

/// The target and what was typed, one pair of lines per display row.
///
/// The line breaking is ours, not egui's: the two lines differ in content, so letting the
/// layouter wrap them separately would break them at different points and they would drift out
/// of alignment. Recomputed each frame from the available width, so resizing just reflows.
fn text_block(ui: &mut egui::Ui, target: &[char], attempt: &Attempt) {
    let font = FontId::monospace(16.0);
    let advance = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'm')).max(1.0);
    let columns = ((ui.available_width() / advance).floor() as usize).max(MIN_COLUMNS);
    let visuals = ui.visuals().clone();
    for line in score::wrap(target, columns) {
        ui.label(job(target, attempt, line.clone(), &font, &visuals, Row::Target));
        ui.label(job(target, attempt, line, &font, &visuals, Row::Typed));
        ui.add_space(4.0);
    }
}

fn job(target: &[char], attempt: &Attempt, line: Range<usize>, font: &FontId, visuals: &egui::Visuals, row: Row) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY; // the breaking is done above
    let typed = attempt.typed();
    let mut run = String::new();
    let mut run_verdict: Option<Verdict> = None;
    for at in line {
        let verdict = attempt.verdict(target, at);
        if run_verdict != Some(verdict) {
            if let Some(previous) = run_verdict {
                job.append(&run, 0.0, format_for(previous, row, font, visuals));
                run.clear();
            }
            run_verdict = Some(verdict);
        }
        run.push(match row {
            Row::Target => target[at],
            Row::Typed => typed.get(at).copied().unwrap_or(' '),
        });
    }
    if let Some(previous) = run_verdict {
        job.append(&run, 0.0, format_for(previous, row, font, visuals));
    }
    job
}

fn format_for(verdict: Verdict, row: Row, font: &FontId, visuals: &egui::Visuals) -> TextFormat {
    let mut format = TextFormat::simple(font.clone(), visuals.text_color());
    match (verdict, row) {
        (Verdict::Pending, _) => format.color = visuals.weak_text_color(),
        (Verdict::Wrong, _) => format.color = WRONG,
        (Verdict::Cursor, Row::Target) => format.background = visuals.selection.bg_fill,
        (Verdict::Cursor, Row::Typed) => format.color = visuals.weak_text_color(),
        (Verdict::Correct, _) => {}
    }
    format
}
```

- [ ] **Step 5: Wire it into `App`**

In `src/ui/mod.rs`:

Add the module beside the others at the top:

```rust
mod tutor_panel;
```

Add to the imports:

```rust
use crate::tutor::{self, Availability, Session};
```

Add one field to `struct App`, after `show_hints` — the failure reason lives in the session,
so `App` keeps no copy of its own:

```rust
    tutor: Session,
```

Initialise them in `App::with_device`, after `show_hints: false,`:

```rust
            tutor: Session::new(),
```

In `on_device_event`, replace the `DeviceEvent::Connected` arm's `self.state.set_keyboard(layout, keymap);` line with:

```rust
                // Check the layout against the finger map before it moves into the state.
                let availability = match tutor::fingers::validate(&layout) {
                    Ok(()) => Availability::Ready,
                    Err(why) => Availability::LayoutMismatch(why),
                };
                self.state.set_keyboard(layout, keymap);
                self.tutor.set_availability(availability);
                // A freshly read keymap invalidates a batch's paths. This arrives on Reload and
                // when another program releases the keyboard, which is how a remap in Vial
                // reaches the drills without restarting the app.
                self.tutor.keyboard_changed();
```

In the `DeviceEvent::Disconnected` arm, after `self.state.clear_keyboard();`:

```rust
                self.tutor.set_availability(Availability::NoKeyboard);
```

Add these two methods to `impl App`, after `start_unlock`:

```rust
    /// The failure reason is recorded inside the session, so the panel reads it from there
    /// rather than this keeping a second copy that could drift.
    fn start_drill(&mut self, id: drills::DrillId) {
        let Some(keymap) = &self.state.keymap else { return };
        let _ = self.tutor.start(id, keymap, self.state.host());
    }

    fn tutor_input(&mut self, input: tutor::Input, now: Instant) {
        let Some(keymap) = &self.state.keymap else { return };
        self.tutor.input(input, keymap, self.state.host(), now);
    }
```

with `use crate::tutor::drills;` added to the imports.

An unlock can't be cancelled and needs two keys held for ten seconds, so it must not overlap a
drill. In `start_unlock`, before the `send`:

```rust
        self.tutor.close();
```

Replace the body of `logic`'s loop with:

```rust
        for input in focused::translate(&std::mem::take(&mut self.typed)) {
            match input {
                FocusedInput::Key(key) if !self.state.evdev_active => self.state.os_key(&key, now),
                FocusedInput::Key(_) => {} // evdev already reported it
                FocusedInput::Text(text) => {
                    self.state.on_text(&text);
                    if self.tutor.is_active() {
                        // Text can carry several characters at once (IME, dead keys). Space
                        // arrives here too, which is why `status::visible` has to special-case it.
                        for c in text.chars() {
                            self.tutor_input(tutor::Input::Char(c), now);
                        }
                    }
                }
                FocusedInput::Command(key) if self.tutor.is_active() => {
                    let input = match key {
                        egui::Key::Backspace => tutor::Input::Backspace,
                        egui::Key::Enter => tutor::Input::Enter,
                        _ => tutor::Input::Escape,
                    };
                    self.tutor_input(input, now);
                }
                FocusedInput::Command(_) => {}
                FocusedInput::FocusLost => {
                    self.state.release_focused_keys();
                    self.tutor_input(tutor::Input::FocusLost, now);
                }
            }
        }
```

In `ui`, beside the existing Ctrl+R handling:

```rust
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::T)) {
            self.tutor.toggle();
        }
```

And add the panel between the status panel and the central panel — order matters, because egui
panels claim space in the order they are added:

```rust
        egui::Panel::bottom("status").show(ui, |ui| status::show(ui, self, now));
        if self.tutor.is_active() {
            egui::Panel::top("tutor").show(ui, |ui| tutor_panel::show(ui, self));
        }
        egui::CentralPanel::default().show(ui, |ui| self.central(ui, now));
```

Finally, replace the placeholder `keyboard::View` from Task 9 with the real one:

```rust
            keyboard::show(ui, &self.state, now, keyboard::View {
                unlock_keys: self.unlock_highlight(),
                fingers: self.tutor.colours_on && self.tutor.is_active(),
                hint: self.tutor.hint(),
            });
```

- [ ] **Step 6: Add the status-bar button**

In `src/ui/status.rs`, before the existing `Reload` button:

```rust
        ui.separator();
        let blocked = app.tutor.available().reason().or_else(|| {
            matches!(app.unlock, super::Unlock::InProgress { .. }).then(|| "Finish the unlock first.".to_string())
        });
        let label = if app.tutor.is_active() { "Close tutor (Ctrl+T)" } else { "Typing tutor (Ctrl+T)" };
        let button = ui.add_enabled(blocked.is_none(), egui::Button::new(label));
        if button.clicked() {
            app.tutor.toggle();
        }
        if let Some(why) = blocked {
            button.on_hover_text(why);
        }
```

A disabled button reports no clicks, so there is deliberately no click route from here to the
setup window: `NoKeyboard` is explained by the hover text, and the existing
`how to enable more…` link a few widgets along is the way in.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --locked`
Expected: PASS, everything.

Run: `cargo clippy --locked --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo build --release` and start the app with the keyboard plugged in. Check by hand:
- Ctrl+T opens the panel on the drill picker; each position drill shows its characters.
- Starting "Home keys" shows the target and an empty typed line; typing fills it, wrong
  characters go red, backspace works, and the batch ends with a summary.
- Finger colours outline the keys; the two OLED positions stay uncoloured.
- With hints on, the next key is green; on a `{` from the Rust drill, the left thumb (MO(1))
  is pale green at the same time.
- Ctrl+R mid-batch returns to the drill picker rather than leaving a stale batch.

- [ ] **Step 8: Commit**

```bash
git add src/ui/tutor_panel.rs src/ui/mod.rs src/ui/status.rs src/input/focused.rs
git commit -m "$(cat <<'MSG'
feat: the typing tutor panel and its wiring

A top panel with the drill picker, the text being typed and the result; the
keyboard picture keeps the central panel below it. Focused-window text drives
the session, and bare Backspace, Enter and Escape become tutor commands while
modifier combinations pass through, so Ctrl+R and Ctrl+T keep working mid-batch.

The drill picker shows what each position drill resolves to on the live keymap,
so a remap in Vial is visible without restarting.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```

---

### Task 11: Documentation

**Files:**
- Modify: `README.md` (a tutor section)
- Modify: `docs/implementation-notes.md` (the encoder wording, and what this feature taught)
- Modify: `docs/manual-test-checklist.md` (a tutor section)

**Interfaces:** none.

- [ ] **Step 1: README**

Add a section after "Live layer tracking":

```markdown
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
```

- [ ] **Step 2: implementation notes**

In `docs/implementation-notes.md`, under "The keyboard", replace:

> The definition has 60 keys, not 58: the extra two are the encoder push switches (`4,5` and `9,0`).

with:

```markdown
- The definition has 60 keys, not 58. The extra two (`4,5` and `9,0`) are matrix positions the
  firmware supports that a given build need not populate: the PCB takes rotary encoders there,
  and the reference board has OLED screens instead. The keymap still assigns them something
  (`KC_MPLY` and `KC_MUTE`), so they appear on the picture as ordinary keys. The typing tutor
  gives them no finger, so they are the only uncoloured keys while it is open.
```

Add a new section before "Testing lesson":

```markdown
## The typing tutor

- **The finger map is a hardwired table, not derived from geometry.** The first design clustered
  keys by their coordinates. It doesn't work: the columnar stagger is up to 0.5 units and the row
  pitch is 1.0, so row bands genuinely overlap across columns — the home row's `y` values sit
  closer to the inner `[` key at 2.75 than to their own neighbours, and clustering on raw `y`
  merges the home and bottom rows. Stagger-normalising first (`round(y - stagger(column))`) does
  work, but a heuristic that misfires produces subtly wrong colours that are hard to notice,
  where a table is wrong loudly or not at all. `fingers::validate` checks the table against the
  reported layout in both directions.
- **`hostlayout::usages_for` must return a list.** GB maps both `KC_BSLS` (0x31) and `KC_NUHS`
  (0x32) to `#`, and the reference keymap has no `KC_BSLS` anywhere — `#` is `KC_NUHS` on layer 1.
  A single-answer reverse lookup declares `#` untypeable and silently strips every heading from
  the Markdown drill.
- **Layer keys are searched on layer 0 only.** `MO(3)` exists only on layers 1 and 2, so a search
  across every layer finds a key that does nothing from the base layer and produces an impossible
  hint. Restricting the search to layer 0 makes hints correct by construction and treats anything
  deeper as unreachable. Nothing is lost on this keymap: layer 3 is RGB controls and `KC_NO`.
  This is also why there is no tri-layer branch in `hint.rs`.
- **Cheapest-path picking pays for itself, and the tie-break does real work.** `{` has three
  routes on this keymap: Shift plus layer 0's `[` at `(4,0)`, layer 1's dedicated
  `LSFT(KC_LBRC)` at `(8,2)` holding `MO(1)`, and layer 2's `[` plus Shift. The first two both
  cost one hold, so the lower-layer tie-break decides and the hint teaches Shift+`[`. The same
  rule makes `!` resolve to layer 0's Shift+1 rather than layer 1's `LSFT(KC_1)`, which is what
  the Shift drill is for. To teach dedicated layer keys instead, invert the layer term in the
  rank tuple in `hint::resolve` — but that flips `!` too.
- **A batch is abandoned on any `DeviceEvent::Connected`, not just on Reload.** Both
  `DeviceCommand::Reload` and resuming after another program releases the device set
  `Session::loaded = false` and re-read the keymap, so a remap made in Vial arrives without a
  Ctrl+R. Hanging abandonment off `App::reload` would leave a batch scoring against stale paths.
- **The ten-colour palette has not been validated across themes.** The colours in
  `ui::keyboard::finger_colour` are a starting point and need an eyeball check as thin strokes
  against both the light and dark egui themes.
```

- [ ] **Step 3: manual checklist**

Add a section to `docs/manual-test-checklist.md`, after "Live layers (unlocked)":

```markdown
## Typing tutor
- [ ] Ctrl+T opens the panel; every position drill lists the characters it currently resolves to.
- [ ] With the keyboard unplugged, the tutor button is disabled and says why.
- [ ] "Home keys" generates a fresh batch each time; typing fills the second line, wrong
      characters go red, and backspace takes them back without erasing the error count.
- [ ] Finger colours are legible on both the light and dark themes, and the two OLED positions
      are the only uncoloured keys.
- [ ] With hints on, the Rust drill's `{` highlights the right-hand key *and* the left thumb
      (MO(1)) at the same time; pressing the thumb turns it blue.
- [ ] Alt+Tab away mid-batch for ten seconds and come back: the wpm hasn't collapsed.
- [ ] Open Vial mid-batch: the status bar says paused, and the tutor keeps working.
- [ ] Remap a key in Vial, close Vial: the drill picker shows the new character.
- [ ] Ctrl+R mid-batch returns to the drill picker.
- [ ] Unplug the keyboard mid-batch: the panel closes and the button says why.
```

- [ ] **Step 4: Commit**

```bash
git add README.md docs/implementation-notes.md docs/manual-test-checklist.md
git commit -m "$(cat <<'MSG'
docs: the typing tutor

README section, manual checklist steps, and the implementation notes this
feature produced: why the finger map is a table rather than geometry, why the
host-layout reverse lookup returns a list, why layer keys are searched on layer 0
only, and that the ten-colour palette still needs a theme check.

Also corrects the note about the two extra keys in the definition: they are
matrix positions the firmware supports that a build need not populate, not
necessarily encoder push switches.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
MSG
)"
```
