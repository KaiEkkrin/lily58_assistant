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
/// How often a `Words` batch spends an item on a focus character the word list can't spell.
/// Every third item: often enough to work through a small set within one batch, rare enough that
/// the text still reads like words. One in two once the set is large, because the Shift drill has
/// 24 such characters — far more than a batch can reach at the slower rate.
const COVER_IN: u32 = 3;
const MANY_UNCOVERED: usize = 8;

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
    // Focus characters the pool can't spell need items of their own, or the picker advertises
    // keys the drill never asks for — which is what "Index reach" did with `[ ] 5 6`.
    let uncovered = if words { uncovered_focus(alpha, &pool, drill) } else { Vec::new() };
    let cover_in = if uncovered.len() > MANY_UNCOVERED { COVER_IN - 1 } else { COVER_IN };
    // A rotation from a random start, not a fresh pick each time: a pick can land on the same
    // character twice and leave another out, and coverage is the whole point of these items.
    let mut turn = if uncovered.is_empty() { 0 } else { rng.random_range(0..uncovered.len()) };
    let mut text = String::new();
    while text.chars().count() < BATCH_CHARS {
        if !text.is_empty() {
            text.push(' ');
        }
        let item = if !words {
            syllable_item(alpha, drill, &focus, rng)
        } else if !uncovered.is_empty() && rng.random_range(0..cover_in) == 0 {
            turn += 1;
            cover_item(&pool, alpha, drill, uncovered[(turn - 1) % uncovered.len()], rng)
        } else {
            word_item(&pool, drill, rng)
        };
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
    let usable: Vec<&'static str> = tokens
        .iter()
        .copied()
        // An empty token would make the batch loop below make no progress — `text.push(' ')`
        // is skipped while `text` is still empty, so an empty first pick would spin forever.
        .filter(|t| !t.is_empty())
        .filter(|t| t.chars().all(|c| hint::resolve(keymap, host, c).is_some()))
        .collect();
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

/// Focus characters no item built from the word pool can contain. `words.txt` is lowercase
/// letters, so for a drill that reaches the digits and symbols this is most of what it
/// advertises. `word_item` capitalises when the drill allows Shift, so the upper case of a
/// pooled letter is in reach and doesn't belong here.
fn uncovered_focus(alpha: &Alphabet, pool: &[&'static str], drill: &Drill) -> Vec<char> {
    let mut spellable: Vec<char> = Vec::new();
    for word in pool {
        for c in word.chars() {
            push_once(&mut spellable, c);
            if drill.shift != Shift::Never {
                for upper in c.to_uppercase() {
                    push_once(&mut spellable, upper);
                }
            }
        }
    }
    alpha.focus_chars().into_iter().filter(|c| !spellable.contains(c)).collect()
}

fn push_once(chars: &mut Vec<char>, c: char) {
    if !chars.contains(&c) {
        chars.push(c);
    }
}

/// One character the word pool can't spell, in the shape it is actually typed: wrapping a word
/// when it is half of a pair this keymap has, otherwise attached to one. `[bank]` is how a
/// bracket arrives in code; a loose `[` between two words is not. The word comes from
/// `word_item`, so a `Shift::Required` drill still gets its capital in every item.
fn cover_item(pool: &[&'static str], alpha: &Alphabet, drill: &Drill, ch: char, rng: &mut impl RngExt) -> String {
    let word = word_item(pool, drill, rng);
    match pair(ch) {
        Some((open, close)) if alpha.chars().any(|c| c == open) && alpha.chars().any(|c| c == close) => {
            format!("{open}{word}{close}")
        }
        // Currency and the like lead; punctuation, closers and digits follow.
        _ if matches!(ch, '$' | '£' | '#' | '@' | '&' | '~' | '%' | '\\') => format!("{ch}{word}"),
        _ => format!("{word}{ch}"),
    }
}

/// The bracket or quote pair this character belongs to, opener first. Wrapping covers both
/// halves at once, which is why the pair matters and not just the character.
fn pair(ch: char) -> Option<(char, char)> {
    match ch {
        '[' | ']' => Some(('[', ']')),
        '(' | ')' => Some(('(', ')')),
        '{' | '}' => Some(('{', '}')),
        '<' | '>' => Some(('<', '>')),
        '"' => Some(('"', '"')),
        '\'' => Some(('\'', '\'')),
        _ => None,
    }
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
    // These two fix-ups can in principle fight — the Shift one below could evict the focus
    // character this one just placed — but across the shipped catalogue they never both fire
    // for the same drill: the one `Shift::Required` drill has `focus: None`. A future drill
    // pairing `focus: Some(_)` with `Shift::Required` would need that interaction resolved
    // first, or it could silently lose the "every item exercises its focus" guarantee.
    if !focus.is_empty() && !out.iter().any(|c| focus.contains(c)) {
        let at = rng.random_range(0..out.len());
        out[at] = focus[rng.random_range(0..focus.len())];
    }
    // See the comment above: this fix-up and the one above it are mutually exclusive in
    // practice, not by construction, so a new drill is what would put that precondition to test.
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

    /// The picker advertises `focus_chars()`, so the drill has to ask for them. Restricted to the
    /// non-letters because that is the class the word list structurally cannot reach — it is
    /// lowercase words, so `[`, `]`, `5` and `6` were advertised by "Index reach" and never
    /// typed, and every digit and symbol by "Shift combinations". An advertised *letter* can be
    /// rare without being unreachable: `z` is in one word of the 296-word pool, and English
    /// frequency deciding how often it comes up is correct, not a defect.
    ///
    /// Over a run of batches rather than one: "Shift combinations" advertises 24 non-letters and
    /// a 110-character batch can't hold them all and still read like text.
    #[test]
    fn every_position_drill_asks_for_the_symbols_it_advertises() {
        let km = reference_keymap();
        let mut rng = seeded();
        let mut complaints: Vec<String> = Vec::new();
        for id in drills::ids_of(drills::Kind::Position) {
            let drill = drills::drill(id);
            let advertised: Vec<char> =
                alphabet(drill, &km, HostLayout::Gb).focus_chars().into_iter().filter(|c| !c.is_alphabetic()).collect();
            let mut seen: Vec<char> = Vec::new();
            for _ in 0..40 {
                let batch = batch(drill, &km, HostLayout::Gb, &mut rng).expect("the reference keymap runs every drill");
                seen.extend(batch.target.iter().copied());
            }
            let missing: String = advertised.iter().copied().filter(|c| !seen.contains(c)).collect();
            if !missing.is_empty() {
                complaints.push(format!("\"{}\" advertises {missing:?} and never asks for it", drill.name));
            }
        }
        assert!(complaints.is_empty(), "{}", complaints.join("; "));
    }

    /// A bracket is typed around something, so the item that covers it is `[word]` and not a
    /// loose `[` sitting between two words. The coverage test above is satisfied either way,
    /// which is why the shape needs its own.
    #[test]
    fn brackets_arrive_wrapped_around_a_word() {
        let km = reference_keymap();
        let mut rng = seeded();
        let mut wrapped = 0;
        for _ in 0..20 {
            let batch = batch(find("Index reach"), &km, HostLayout::Gb, &mut rng).expect("the reference keymap runs this drill");
            let text: String = batch.target.iter().collect();
            for item in text.split(' ').filter(|i| i.contains('[') || i.contains(']')) {
                let inner = item.strip_prefix('[').and_then(|i| i.strip_suffix(']'));
                assert!(inner.is_some_and(|i| !i.is_empty()), "{item:?} is not a word in brackets");
                wrapped += 1;
            }
        }
        assert!(wrapped > 0, "the drill has [ and ] in its alphabet, so some item should use them");
    }

    #[test]
    fn the_home_drill_resolves_to_the_eight_resting_keys() {
        let alpha = alphabet(drills::drill(0), &reference_keymap(), HostLayout::Gb);
        let mut focus = alpha.focus_chars();
        focus.sort_unstable();
        assert_eq!(focus, vec![';', 'a', 'd', 'f', 'j', 'k', 'l', 's']);
        assert!(alpha.chars().any(|c| c == ' '), "space is always available, whatever the drill");
    }

    /// `fingers::BANDS` is indexed positionally, so a swap between two bands (say `Number` and
    /// `Bottom`) would silently make "Number row" drill the bottom-row letters and vice versa —
    /// every other test in this file would still pass, because they don't check which alphabet
    /// comes out, only that a batch stays inside it. This pins the identity down against the
    /// spec's own oracle table.
    #[test]
    fn stretch_up_and_number_row_resolve_to_the_right_bands() {
        let km = reference_keymap();
        let mut stretch_up = alphabet(find("Stretch up"), &km, HostLayout::Gb).focus_chars();
        stretch_up.sort_unstable();
        assert_eq!(stretch_up, vec!['e', 'i', 'o', 'p', 'q', 'r', 'u', 'w']);
        let mut number_row = alphabet(find("Number row"), &km, HostLayout::Gb).focus_chars();
        number_row.sort_unstable();
        assert_eq!(number_row, vec!['0', '1', '2', '3', '4', '7', '8', '9']);
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

    /// Syllables shouldn't hammer one finger: the same key never repeats immediately. Checked
    /// against every `Syllables` drill, not just Home keys — Home's focus equals its include and
    /// it never asks for Shift, so it's the one drill where neither post-loop fix-up in
    /// `syllable_item` (topping up the focus character, topping up a shifted one) ever runs.
    /// Number row, Stretch down and Outer column do exercise those fix-ups.
    #[test]
    fn letter_groups_never_repeat_a_key_back_to_back() {
        let km = reference_keymap();
        for d in DRILLS {
            if d.style != Style::Syllables {
                continue;
            }
            let alpha = alphabet(d, &km, HostLayout::Gb);
            let (text, _) = keys_text(d, &alpha, &mut seeded()).unwrap();
            for item in text.split(' ') {
                let chars: Vec<char> = item.chars().collect();
                for pair in chars.windows(2) {
                    assert_ne!(pair[0], pair[1], "{}: {item:?} repeats a key", d.name);
                }
            }
        }
    }
}
