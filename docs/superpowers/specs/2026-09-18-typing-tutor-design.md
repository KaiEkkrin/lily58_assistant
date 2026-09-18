# Typing Tutor Mode — Design

Date: 2026-09-18
Status: Approved (design); not yet implemented

## Purpose

A typing-tutor mode inside the Lily58 Assistant. It generates drill text at
runtime, takes over the keyboard while the assistant's own window has focus,
shows what was typed against what was asked for, and scores it. Alongside the
drill, the keyboard picture gains finger-group colouring and an optional
next-key hint that points at the key to press — including the Shift or layer
key needed to reach it.

The v1 design anticipated this: "It is designed to grow: a typing-tutor mode and
similar features come later as additional consumers of the same state and event
stream." This is that feature, and it stays within that shape — it consumes the
existing layout, keymap and focused-input machinery and adds no new device
access.

## Key non-feature: still strictly read-only

The tutor sends nothing to the keyboard. It reads the layout and keymap that
`AppState` already holds and the keystrokes that `raw_input_hook` already
captures. `hid::guard::ReadOnlyGuard` is untouched.

## Scope

In scope:

- Modal activation from the status bar and Ctrl+T; active only while the
  assistant's window has focus.
- Twelve drills: six position-driven, one Shift drill, five programmer token
  drills.
- Runtime-generated batches of words or syllables, drawn from the live keymap.
- Free-run typing with backspace, per-character verdicts, accuracy, wpm and
  per-finger error attribution.
- Ten-colour finger-group outlines on the keyboard picture.
- Optional next-key hints, including Shift and layer keys.

Out of scope, deliberately:

- Saved history, personal bests, progress over time.
- Adaptive drills that over-sample weak keys.
- Anything about the OLED screens, or hiding the two unpopulated matrix
  positions from the main picture (a pre-existing cosmetic wart; file it
  separately).
- Working without a keyboard connected. The tutor is unavailable then.

## Decisions taken, and why

| Decision | Choice | Reason |
|---|---|---|
| Drill definitions | Position-driven, resolved through the live keymap | "Stretch up" means the keys above home on *this* keymap, and remapping follows |
| Finger map | Hardwired table, matrix position keyed, with a validation guard | A heuristic is subtly wrong and hard to notice; a table is wrong loudly or not at all |
| Item shapes | Per-drill: words or syllables | Letter drills earn real words; symbol drills cannot have them |
| Mistakes | Free-run; backspace allowed; the original error still counts | Closest to typing real text, and shows the mistyped line |
| Session shape | Fixed batch, then a result | Makes the assessment concrete and the scoring testable |
| Persistence | None | YAGNI; keeps the app's "writes nothing" character |
| No keyboard | Tutor unavailable, with the reason | One code path; no degraded mode to maintain |
| Screen layout | Tutor panel on top, keyboard picture below | Reading order matches where the eye goes; both stay visible |
| Finger cue | Coloured per-key outline | Key fills are already taken by held (blue) and unlock (orange) |
| Next-key hint | Yes, including layer keys, with a toggle | The thing a generic tutor cannot do on this keyboard |
| Randomness | `rand` crate | Small PRNGs are subtle; a slightly wrong one feels off in a way that is hard to diagnose |

## Architecture

```
src/tutor/
  mod.rs          Session: the state machine; the only type ui/ touches
  fingers.rs      hardwired (row, col) -> Spot; validate(&Layout)
  drills.rs       the catalogue
  generate.rs     alphabet resolution and batch construction
  words.txt       ~800 common lowercase words, include_str!
  hint.rs         char -> KeyPath
  score.rs        Attempt, Summary, and the display line splitter
  fixture.rs      #[cfg(test)] probe-output parser for the real keymap
src/ui/tutor.rs   the top panel
```

Nothing under `src/tutor/` depends on egui. `Session` accepts a plain `Input`
enum and plain `char`s; `ui/mod.rs` translates. The whole tutor — generation,
scoring, hints, finger map — is therefore testable without a display.

### Ownership

`App` owns `tutor: Session`, as a sibling of `state`. `AppState`'s job is
merging the three input tiers; a scoring engine does not belong there.

`Session` holds **no keyboard data** — no `Layout`, no `Keymap`, no clone of
either. `AppState` stays the single source of truth and callers pass references
at the call site:

```rust
Session::start(&mut self, drill: DrillId, keymap: &Keymap, host: HostLayout)
    -> Result<(), StartError>
```

### Precomputed paths

Next-key paths are resolved once per batch, never per frame:

```rust
struct Batch {
    target: Vec<char>,
    paths: Vec<Option<KeyPath>>,   // parallel to target
    note: Option<&'static str>,    // e.g. the syllable fallback
}
```

Drawing a hint is then an index lookup. `docs/implementation-notes.md` already
lists per-frame linear scans as a wart worth not repeating (#10).

A fresh keymap invalidates a batch's precomputed `paths`, so **any
`DeviceEvent::Connected` abandons the batch in progress** and returns to drill
selection. Keying off `Connected` rather than off `App::reload` matters, because
the worker re-reads the keymap on two paths, not one:

- `DeviceCommand::Reload` sets `Session::loaded = false` (`device.rs:172`);
- resuming after another program released the device does the same
  (`device.rs:317`), precisely because "the other program may have locked,
  unlocked, or changed the keymap while it held the device".

Both then emit `Connected` with a freshly read layout and keymap; the test at
`device.rs:634` asserts `["Resumed", "Connected", "Locked"]`. So editing the
keymap in Vial and closing Vial refreshes the assistant with no Ctrl+R, and the
tutor's drills follow the edit on the next batch. Hanging abandonment off
`App::reload` would have missed that path and left a batch scoring against stale
paths.

### Data flow

```
DeviceEvent::Connected -> AppState::set_keyboard -> fingers::validate(&layout)
                                                      -> Session.available

Ctrl+T / button        -> Session::start(drill, &keymap, host)
                            -> generate::batch(..., &mut rng) -> Batch

focused keystroke      -> App::logic -> Session::input(Input)
                            -> Attempt mutates; Summary on completion

frame                  -> ui::tutor::show(top panel, &Session)
                       -> keyboard::show(ui, &state, now, View { .. })
```

## The finger map

```rust
enum Hand   { Left, Right }
enum Finger { Pinky, Ring, Middle, Index, Thumb }
enum Band   { Number, Top, Home, Bottom, Thumb }
enum Reach  { Normal, Outward, Inward }
struct Spot { hand: Hand, finger: Finger, band: Band, reach: Reach }

fn spot(row: u8, col: u8) -> Option<Spot>
```

Colours key off `finger` alone, so a pinky and its outward stretch share a
colour. Drills select on `band` and `reach`.

| matrix rows | meaning |
|---|---|
| 0-3 | left half: `Number`, `Top`, `Home`, `Bottom` |
| 5-8 | right half: the same four bands |
| 4 | `col0` left inner-bottom (`Bottom`/`Inward`) - `col1..4` left thumbs - `col5` no spot |
| 9 | `col5` right inner-bottom (`Bottom`/`Inward`) - `col1..4` right thumbs - `col0` no spot |

Column to finger, for rows 0-3 and 5-8:

| left col | 5 | 4 | 3 | 2 | 1 | 0 |
|---|---|---|---|---|---|---|
| | Pinky / Outward | Pinky | Ring | Middle | Index | Index / Inward |

| right col | 0 | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| | Pinky / Outward | Pinky | Ring | Middle | Index | Index / Inward |

Confirmed against the real keymap: layer 0 row 2 is
`KC_G KC_F KC_D KC_S KC_A KC_LCTL` (cols 0 to 5), and row 4 is
`KC_LBRC KC_SPC MO(1) KC_LGUI KC_LALT KC_MPLY`.

`(4,5)` and `(9,0)` are matrix positions the firmware supports that this build
does not populate — the OLED screens sit there, and the keymap assigns them
`KC_MPLY` and `KC_MUTE`. They get no spot, so the tutor never colours them and
never asks for them. With colouring on they are the only two uncoloured keys,
which happens to read correctly.

`(4,0)` and `(9,5)` are the `[` and `]` keys between the halves, right of G/B
and left of H/N. Their `y` centre lands exactly midway between the home and
bottom rows; they are assigned to `Bottom` with `Inward` reach, which is a
judgement call and a one-line change if it feels wrong in use.

### Validation guard

```rust
fn validate(layout: &Layout) -> Result<(), String>
```

Checks the matrix is 10x6 and that the table's positions and the layout's keys
match exactly in both directions. Arithmetic check: 58 spots plus 2 no-spots
equals the 60 keys the definition reports. On mismatch the tutor is unavailable
with the reason shown, rather than colouring keys wrongly.

## Drills

```rust
struct Group { band: Option<Band>, reach: Option<Reach> }   // conjunctive

enum Source {
    Keys { include: &'static [Group], focus: Option<Group> },
    Tokens(&'static [&'static str]),
}

struct Drill {
    id: DrillId,
    name: &'static str,
    source: Source,
    shift: Shift,   // Never | Allowed | Required
    style: Style,   // Words | Syllables
}
```

The alphabet is drawn from everything in `include`; an item is kept only if it
uses at least one character from `focus`. So "stretch up" mixes the top row with
home keys already known, while guaranteeing every item exercises the top row.
`Group` is conjunctive so that `{band: Home, reach: Normal}` means the eight
home keys, with G, H and the modifiers excluded rather than argued about.

| drill | include | focus | shift | style |
|---|---|---|---|---|
| Home keys | `{Home, Normal}` | `{Home, Normal}` | Never | Syllables |
| Stretch up | `{Home, Normal}`, `{Top, Normal}` | `{Top, Normal}` | Never | Words |
| Stretch down | `{Home, Normal}`, `{Bottom, Normal}` | `{Bottom, Normal}` | Never | Words |
| Number row | `{Home, Normal}`, `{Number, Normal}` | `{Number, Normal}` | Never | Syllables |
| Outer column | `{Home, Normal}`, `{_, Outward}` | `{_, Outward}` | Allowed | Syllables |
| Index reach | `{Home, Normal}`, `{_, Inward}` | `{_, Inward}` | Never | Words |
| Shift combinations | Home/Top/Bottom/Number, all `Normal` | none | Required | Words |
| Markdown, HTML, Rust, TypeScript, Elixir | `Tokens(..)` | — | Allowed | — |

The Shift drill has no `focus`: `shift: Required` already guarantees every item
contains a shifted character, which is the point of the drill.

"Stretch left/right" became **Outer column** and **Index reach**, because
lateral reach on a split board is per hand — outward for the pinky, inward for
the index — so left and right name nothing.

### Resolved alphabets on the reference keymap

Useful as a test oracle. Home keys are added to every position drill via
`include`, and space is always present.

| drill | focus characters |
|---|---|
| Home keys | `a s d f j k l ;` |
| Stretch up | `q w e r u i o p` |
| Stretch down | `z x c v m , . /` |
| Number row | `1 2 3 4 7 8 9 0` |
| Outer column | `` ` `` `-` `'` (plus `¬ _ @` shifted) |
| Index reach | `5 t g b [ 6 y h n ]` |

**The Outer column drill is right-hand only, and permanently so.** The left
outer column is `KC_ESC`, `KC_TAB`, `KC_LCTL`, `KC_LSFT` — no characters at all,
and the keycaps are legended for those functions, so it will not be remapped.
The generator drops unprintable keys without a special case, so this needs no
handling; the drill is simply a right-pinky drill and the panel says so. It
remains position-driven rather than hardcoded, so it would pick up a character
if one ever appeared there.

## Generation

### Alphabet resolution

Take the positions matching any `include` group; resolve each through the keymap
and host layout to the character it emits on layer 0; drop those that emit
nothing. For `shift: Allowed` or `Required`, add the shifted character where it
differs. **Space is always added**, so words have separators and the thumbs are
drilled for free with a hint pointing at the right thumb key.

### Items

**Words.** Filter `words.txt` to words spellable from the unshifted alphabet,
then keep those satisfying `focus`. For `shift: Required` apply a capitalisation
pattern per word (leading cap, all caps, or one interior cap); for `Allowed`, to
about a fifth of them.

**Syllables.** 2-6 characters from the alphabet, weighted toward 3-4, with two
shaping rules: never the same key twice running, and prefer alternating hands
about 60% of the time. Must satisfy `focus`, and for `shift: Required` must
contain at least one shifted character.

**Tokens.** Sample the drill's list, filtered to tokens whose every character
resolves through `hint`. An unusual keymap thins the Rust drill rather than
asking for a `|` that cannot be typed.

**Batch size.** Accumulate space-separated items until the target string reaches
110 characters. One constant to tune, and it gives every drill comparable effort
regardless of item length.

### Honest fallback

If a `Words` drill's filtered pool comes out under a dozen words, generation
falls back to syllables and sets `Batch.note`, which the panel shows. Silently
serving gibberish where words were promised reads as a bug.

### Randomness

`Session` owns a `StdRng` seeded from the OS at construction. Every generator
function takes `&mut impl Rng`, so tests seed a constant and assert exact
output — the only way the shaping rules above are testable.

`rand` goes into `Cargo.toml`; `getrandom` arrives transitively through the
default `sys_rng` feature and needs no direct entry.

## Hints and reachability

```rust
struct KeyPath {
    key: (u8, u8),        // the key that emits the character
    hold: Vec<(u8, u8)>,  // Shift and/or a layer key, held first
}
```

Resolution for a character `c`:

1. **Reverse the host layout.** New
   `hostlayout::usages_for(c) -> Vec<(u8, bool)>` returns *every* usage and
   shift state that produces `c`, ascending. This must be a list, not a single
   answer: GB maps both `KC_BSLS` (0x31) and `KC_NUHS` (0x32) to `#`, the
   reference keymap has no `KC_BSLS` anywhere, and `#` lives on `KC_NUHS` at
   layer 1 `(2,5)`. Returning only the first candidate would declare `#`
   untypeable and silently strip every heading from the Markdown drill.
2. **Enumerate every key that emits it**, over all layers: positions where
   `keycodes::tap_basic(code)` is one of the candidate usages. Build each
   candidate's `hold` list — a Shift key if Shift is needed and the code does
   not already carry it (`keycodes::adds_shift`, which is how `KC_EXLM` needs no
   separate Shift), plus a layer key if the layer is not 0.
3. **Pick the cheapest**: fewest holds, tie-broken by lower layer, then matrix
   order. Enumerating rather than reusing `Keymap::find_position` is what makes
   a preference possible at all. On the reference keymap `{` is
   `LSFT(KC_LBRC)` on layer 1 at `(8,2)` — one hold, the left thumb — against
   Shift plus layer 2's `[`, which is two. The tie-break matters too: `!` is
   `LSFT(KC_1)` on both layer 0 and layer 1, so equal holds and the lower layer
   wins, teaching Shift+1.
4. **Layer keys are searched on layer 0 only.** A key decoding to `Momentary` or
   `LayerTap` for the target layer is preferred, then `LayerMod`,
   `OneShotLayer`, `Toggle`, `TapToggle`, `To`. Searching all layers would be
   wrong: `MO(3)` exists only on layers 1 and 2, so a naive search would find a
   layer-3 key that cannot be pressed from the base layer and emit an impossible
   hint. Restricting to layer 0 makes hints correct by construction and treats
   deeper layers as unreachable. Nothing is lost on this keymap — layer 3 is RGB
   controls and `KC_NO`.
5. **Shift keys**: prefer one on the opposite hand to the target key, via
   `fingers::spot`. Ordinary typing advice, one comparison.

Any step failing means the character is unreachable and resolution returns
`None`. `Keymap::find_position` is left alone for the OS-key-inference job it
was written for.

**The tri-layer branch is deliberately absent.** `config.tri_layer` affects
`LayerTracker`, not the tutor. Step 4's layer-0-only rule makes tri-layer
characters unreachable, which is correct here and fails closed if a future
keymap puts characters on layer 3.

## Scoring

```rust
struct Attempt {
    typed: Vec<char>,      // cursor is typed.len()
    wrong: Vec<bool>,      // per target position: was it EVER typed wrong?
    keystrokes: u32,
    mistakes: u32,
    elapsed: Duration,
    last: Option<Instant>,
}
```

`wrong[i]` latches. Backspacing and retyping fixes the screen but not the tally,
which is how "the original error still counts" holds without storing history.

Verdicts fall out of the cursor: below it, correct or wrong by comparison; at
it, the cursor; above it, pending. The batch completes when
`typed.len() == target.len()`, including when that last character is wrong —
free-run means it ends and the summary shows the damage.

**The clock has two rules.** `IDLE_CAP` is 5 seconds:

```
on keystroke:   if let Some(l) = last { elapsed += (now - l).min(IDLE_CAP) }
                last = Some(now)
on focus lost:  last = None
```

The first keystroke contributes no time, so the clock starts when typing starts,
not when the batch appears. Staring at an unfamiliar symbol for a minute costs
5 seconds. Alt-tabbing away costs nothing, because `last = None` means the gap
across the absence is never measured — which is how "only takes over while the
window is in focus" shows up in the scoring, not just in the input routing.

Backspaces are not counted as keystrokes, per convention, so they do not dilute
accuracy.

```
accuracy = (keystrokes - mistakes) / keystrokes
wpm      = ((keystrokes - mistakes) / 5) / elapsed_minutes
```

Per-finger blame reuses work already done: each latched `wrong[i]` indexes
`batch.paths[i]`, whose `key` goes through `fingers::spot`.

```rust
struct Summary {
    chars: usize, keystrokes: u32, mistakes: u32,
    accuracy: f32, wpm: f32, elapsed: Duration,
    worst_finger: Option<(Hand, Finger, u32)>,
    worst_chars: Vec<(char, u32)>,   // top few
}
```

## The session state machine

```rust
enum Phase {
    Off,
    Choosing,
    Typing { drill: DrillId, batch: Batch, attempt: Attempt },
    Done   { drill: DrillId, batch: Batch, summary: Summary },
}

enum Input { Char(char), Backspace, Enter, Escape, FocusLost }
```

| input | `Typing` | `Done` | `Choosing` |
|---|---|---|---|
| `Char(c)` | type it; on completion go to `Done` and fold into session totals | — | — |
| `Backspace` | un-type | — | — |
| `Enter` | — | next batch, same drill | start the selected drill |
| `Escape` | to `Choosing` | to `Choosing` | to `Off` |
| `FocusLost` | `last = None` | — | — |

`Session` also carries running totals across batches (`batches`, `keystrokes`,
`mistakes`, `elapsed`), `hints_on: bool`, and:

```rust
enum Availability { Ready, NoKeyboard, LayoutMismatch(String) }
```

`Availability` is set by `App`, not computed inside `Session`: `set_keyboard`
runs `fingers::validate` and stores `Ready` or `LayoutMismatch`,
`clear_keyboard` stores `NoKeyboard`. `Session` only reads it.

### Entering and leaving

Ctrl+T (or the status-bar button) moves `Off` to `Choosing`, and any active
phase back to `Off`, abandoning a batch in progress. Escape steps out one level
at a time, per the table above.

### Input translation

`Input::Char` comes **only** from `egui::Event::Text`; `Backspace`, `Enter` and
`Escape` come only from key events whose modifiers are empty. So Ctrl+R and
Ctrl+T pass through the tutor untouched — egui emits no `Text` for a Ctrl
combination, and the modifier check rejects the key event — and Reload and the
mode toggle keep working mid-batch.

Because `focused::operates_widgets` already strips Tab, Space and Enter before
egui sees them (#2), typing a space never presses a button and Enter is the
tutor's to use.

`Event::Text` may carry several characters (IME, dead keys); iterate its chars.

## The UI

### Activation

A `Typing tutor (Ctrl+T)` button beside `Reload` in the status bar, plus the
hotkey consumed in `ui()` the way Ctrl+R already is. When `Availability` is not
`Ready` the button is disabled and carries the reason; `NoKeyboard` also offers
the existing "how to fix…" link into the hints window, so keyboard access is
explained in one place rather than two.

### The top panel

`egui::Panel::top("tutor")`, shown only when `Phase != Off`, so the plain
assistant is unchanged when the tutor is closed.

- **Choosing** — drill buttons in two labelled rows, positions and programmer; a
  `Show next-key hints` checkbox; `Close`. Each position drill shows the
  characters its `focus` currently resolves to on the live keymap, e.g.
  `Outer column   ` `` ` `` `- ' ¬ _ @`. This is cheap (the alphabet resolution
  already exists) and it makes the keymap-reactivity visible: retune a key in
  Vial, close it, and the drill list shows the new character without a
  restart.
- **Typing** — drill name, the text block, a live `accuracy · wpm · errors`
  line, `Stop`.
- **Done** — the summary block, then `Again` / `Next batch (Enter)` / `Stop`.

### The text block

Target and typed lines have different content, so letting egui wrap them would
break them at different points and they would drift out of alignment. The line
breaking is therefore ours:

```
chars_per_line = max(8, floor(available_width / monospace_glyph_advance))
split the target at word boundaries into lines of at most that
render each line as a (target, typed) pair, wrapping disabled
```

Stateless, recomputed each frame from the available width — cheap for 110
characters, and resizing just reflows. The floor of 8 keeps a degenerately
narrow window from producing empty or zero-width lines. Both lines of a pair sit on the same
character grid, so they stay aligned by construction. The splitter is a pure
function in `score.rs`, not in `ui/`, so it is unit-testable.

Each line is one `LayoutJob` with consecutive same-verdict characters coalesced
into runs. Colours: pending `weak_text_color`, correct `text_color`, wrong the
same red the status bar uses for errors, and the cursor as a background
highlight on the target line.

### Finger colours

Drawn as the key's outline stroke, thicker than the current 1px hairline, with
`Finger` mapping to colour. **Ten distinct colours** — four fingers and a thumb
on each hand. A mirrored five would be easier on the eye but would not say which
hand a key belongs to, and on a split board that is half the information.

No legend: the colours group keys, and position already says which finger.

The particular hue per finger is arbitrary, but the mapping must be a fixed
table rather than derived from an index, so that adjusting one colour after the
theme check does not shuffle the others.

The palette needs an eyeball check as thin strokes against both the light and
dark egui themes before it is final. Implementation should start from a
mid-saturation categorical set and adjust; this is the fiddly part of the
feature and should not be assumed done because it compiles.

Colours show whenever the tutor is active, including while choosing a drill,
with a toggle in the panel. They never show when it is closed.

### Next-key hints

`path.key` gets a green fill and `path.hold` keys a paler green, with precedence
**held > unlock > hint > default**. A hinted key turns blue the moment it is
actually pressed, so the hint and the existing press feedback compose instead of
fighting over the same pixels. Shift and layer keys light up in the pale shade,
which is how the chord for `{` teaches itself.

### `keyboard::show`

```rust
pub struct View<'a> {
    pub unlock_keys: &'a [(u8, u8)],
    pub fingers: bool,
    pub hint: Option<&'a KeyPath>,
}
```

`fingers` is a bool because the map is a pure function of position. This
replaces the trailing `unlock_keys` argument rather than adding a fifth and
sixth parameter. The picture keeps the central panel and simply gets less
height; `unit` already derives from available size, so it scales down unchanged.

## Edge cases

| situation | behaviour |
|---|---|
| keyboard disconnects mid-batch | `Session` to `Off`, batch abandoned, button disabled with the reason. Without the picture there are no colours and no hints, and the tutor is not worth pretending about |
| another program takes the keyboard (`Paused`) | **tutor keeps working.** `DeviceEvent::Paused` leaves `layout` and `keymap` intact, so drills, colours and hints all still resolve; only live key highlighting stops |
| Reload (Ctrl+R) | abandons the batch, returns to `Choosing` |
| unlock in progress | the tutor cannot be opened, and starting an unlock closes it. An unlock needs two keys held for ten seconds and cannot be cancelled |
| focus lost mid-batch | clock stops; `release_focused_keys` already handles held keys |
| `Event::Text` with several characters | iterate its chars |
| drill alphabet too small | `Session::start` returns `Err`; the panel names the drill and why |
| word pool too thin | syllable fallback with `Batch.note` shown |

## Testing

Everything under `src/tutor/` is egui-free and pure. `cargo test` must keep
passing with no keyboard and no display.

### Reference keymap fixture

`tests/fixtures/lily58-keymap.txt` holds the four layer blocks from
`lily58-assistant --probe` on the reference board, **with the header lines
stripped** (they carry the device path and keyboard uid, which the tests do not
need).

`tutor::fixture` (test-only) parses it into a `Keymap`: `KC_<NAME>` via a
reverse scan over `keycodes::basic_name(0..=0xFF)`, `LSFT(..)` as `0x0200 |
inner`, `MO(n)` as `0x5220 + n`, plus `KC_TRNS`, `KC_NO` and raw `0x....`. About
40 lines, and it keeps the fixture human-readable and diffable against a future
probe.

This matters: hint and generation tests are only worth much against a realistic
keymap, and `hid::fake::SMALL_KEYMAP` is a 2x3 toy.

### What gets tested

- **`fingers.rs`** — a table test over the real definition, parsed from
  `tests/fixtures/lily58-definition.json` the way `layout.rs` already does.
  `validate` passes; 58 spots plus 2 no-spots equals 60; landmarks `(2,4)` left
  pinky home, `(2,0)` left index home inward, `(7,5)` right index home inward,
  `(4,1..=4)` left thumbs, `(4,0)` left index bottom inward, `(4,5)` and `(9,0)`
  no spot. A negative test that a mismatched layout fails `validate`.
- **`hostlayout::usages_for`** — a round-trip property over every usage and both
  shift states: whatever `char_for` produces must resolve back to *that
  character* (not necessarily the same usage; the keypad duplicates make that a
  deliberately weaker claim). Named cases: `#` yields both 0x31 and 0x32, `/`
  yields `KC_SLSH` before the keypad, `£` yields Shift+3, `A` yields Shift+`KC_A`.
- **`hint.rs`** against the fixture keymap — `{` resolves to `(8,2)` on layer 1
  holding `(4,2)`; `!` resolves to layer 0 Shift+1 rather than layer 1; the
  Shift key chosen is on the opposite hand; a character only on layer 3 is
  unreachable; space resolves to `(4,1)`.
- **`generate.rs`** — seeded `StdRng`, exact expected batch strings, plus
  invariants over many seeds: every character is in the drill's alphabet, every
  item satisfies `focus`, `shift: Required` batches contain shifted characters,
  token drills emit only reachable tokens, the thin-pool fallback triggers and
  sets `note`.
- **`score.rs`** — `Instant`s injected, matching how `now` is already threaded
  through this codebase. Latching (backspace-and-retype does not clear `wrong`),
  the idle cap, and that a focus loss costs no time. Plus the line splitter:
  given a width, pairs align and no line exceeds it.
- **`mod.rs`** — a transition table test over `Phase` and `Input`.
- **`ui/tutor.rs`** — kept thin, because the pure parts live elsewhere.

### Manual checklist additions

`docs/manual-test-checklist.md` gains: colours legible on light and dark themes;
hints point at the right keys for a layered symbol (`{`, `#`, `|`); alt-tabbing
mid-batch does not wreck the wpm; the tutor survives `Paused` with Vial open;
the two unpopulated positions stay uncoloured.

## Documentation

- **README** — a tutor section: what it does, that it reads only keys typed into
  its own window, and the drill list.
- **implementation-notes.md** — correct "the extra two are the encoder push
  switches" to say *matrix positions the firmware supports that a given build
  may not populate*, since the reference board has OLED screens there; record
  that the finger map is a hardwired table with a validation guard and why
  geometry was dropped (the stagger is half the row pitch, so raw `y` clustering
  merges the home and bottom rows, and a misfiring heuristic is hard to notice);
  record that `usages_for` must return a list because GB maps two usages to `#`;
  record that layer keys are searched on layer 0 only because `MO(3)` exists
  only on layers 1 and 2; note the ten-colour palette needs a theme check.

## Deliberate omissions

- Saved history, personal bests, adaptive drills.
- Tri-layer handling in `hint.rs` (see "Hints and reachability").
- Hiding the two unpopulated matrix positions from the main keyboard picture.
- Any change to the device worker, the HID guard, or the input tiers.
