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
