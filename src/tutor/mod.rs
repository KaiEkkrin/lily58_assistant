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
        let batch = generate::batch(drills::drill(id), keymap, host, &mut self.rng)?;
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
