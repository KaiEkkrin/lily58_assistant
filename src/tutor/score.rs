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
