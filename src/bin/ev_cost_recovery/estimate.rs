//! The Estimate tab: a workbook and an interval of interest in, the peak-contribution report out.

use crate::state::{EstimateState, WorkingDir, report_sections};
use crate::{theme, widgets};
use eframe::egui;
use egui_extras::DatePickerButton;
use ev_cost_recovery::session::{
    Bracket, IntervalEstimates, IoiLength, LEGAL_START_MINUTES, Segment,
};
use ev_cost_recovery::time::time_zone;
use jiff::Zoned;

pub fn ui(ui: &mut egui::Ui, state: &mut EstimateState, working: &mut WorkingDir) {
    widgets::heading(ui, "Estimate peak contribution");
    widgets::note(
        ui,
        "Estimates what EV charging contributed to the building's peak demand over one interval of \
         interest, taken from Toronto Hydro's metering data.",
    );
    ui.add_space(12.0);

    workbook_row(ui, state, working);
    ui.add_space(12.0);
    interval_controls(ui, state);

    ui.add_space(12.0);
    if ui
        .add_enabled(state.can_estimate(), egui::Button::new("Estimate"))
        .clicked()
    {
        state.run();
    }

    if let Some(message) = &state.error {
        ui.add_space(8.0);
        widgets::error_block(ui, message);
    }

    if state.outcome.is_some() {
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        results(ui, state, working);
    }
}

fn workbook_row(ui: &mut egui::Ui, state: &mut EstimateState, working: &mut WorkingDir) {
    ui.horizontal(|ui| {
        if ui.button("Select workbook…").clicked()
            && let Some(path) = widgets::dialog(working)
                .add_filter("Session report workbook", &["xlsx"])
                .pick_file()
        {
            working.remember(&path);
            state.select_workbook(path);
        }
        widgets::picked_file(
            ui,
            state.workbook.as_ref().map(|w| w.path.as_path()),
            "No workbook chosen",
        );
    });

    // What the workbook covers is worth saying plainly: it is both a check that the right month
    // was opened and the reason the date picker starts where it does.
    if let Some(workbook) = &state.workbook {
        match workbook.covers {
            Some((first, last)) => widgets::note(ui, &format!("Covers {first} to {last}")),
            None => widgets::note(ui, "This workbook holds no sessions."),
        }
        // A picker that fills itself in should account for itself.
        if state.carried_over {
            widgets::note(ui, "Carried over from the conversion you just ran.");
        }
    }
}

fn interval_controls(ui: &mut egui::Ui, state: &mut EstimateState) {
    let enabled = state.workbook.is_some();
    ui.add_enabled_ui(enabled, |ui| {
        egui::Grid::new("interval_grid")
            .spacing([12.0, 10.0])
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Date");
                let mut date = state.date;
                if ui
                    .add(DatePickerButton::new(&mut date).id_salt("interval_date"))
                    .changed()
                    || date != state.date
                {
                    state.set_date(date);
                }
                ui.end_row();

                ui.label("Start");
                ui.horizontal(|ui| {
                    hour_picker(ui, state);
                    ui.label(":");
                    let minutes: Vec<(i8, String, bool)> = LEGAL_START_MINUTES
                        .iter()
                        .map(|&m| (m, format!(":{m:02}"), true))
                        .collect();
                    let options: Vec<(i8, &str, bool)> = minutes
                        .iter()
                        .map(|(m, label, on)| (*m, label.as_str(), *on))
                        .collect();
                    if let Some(minute) = widgets::choice_row(ui, state.minute, &options) {
                        state.set_minute(minute);
                    }
                });
                ui.end_row();

                ui.label("Length");
                // An hour-long interval is legal only from HH:00, so off the hour the button is
                // simply not on offer. See docs/session/README.md, "Interval of interest boundaries".
                let options = [
                    (IoiLength::FifteenMinutes, "15 minutes", true),
                    (
                        IoiLength::Hour,
                        "1 hour",
                        IoiLength::Hour.allowed_from(state.minute),
                    ),
                ];
                if let Some(length) = widgets::choice_row(ui, state.length, &options) {
                    state.set_length(length);
                }
                ui.end_row();
            });

        if state.needs_designator() {
            ui.add_space(10.0);
            fold_question(ui, state);
        }
    });
}

fn hour_picker(ui: &mut egui::Ui, state: &mut EstimateState) {
    let mut chosen = None;
    egui::ComboBox::from_id_salt("interval_hour")
        .selected_text(format!("{:02}", state.hour))
        .width(64.0)
        .show_ui(ui, |ui| {
            for entry in &state.hours {
                let label = format!("{:02}", entry.hour);
                if ui
                    .selectable_label(entry.hour == state.hour, label)
                    .clicked()
                {
                    chosen = Some(entry.hour);
                }
            }
        });
    if let Some(hour) = chosen {
        state.set_hour(hour);
    }
}

/// The question the clocks ask once a year, asked plainly and only then.
///
/// The hour occurs twice, so a figure quoted without saying which one is a figure for an hour
/// nobody asked about. The Estimate button waits for the answer.
fn fold_question(ui: &mut egui::Ui, state: &mut EstimateState) {
    let color = ui.visuals().warn_fg_color;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.label(egui::RichText::new(format!(
                "The clocks go back that night, so {:02}:{:02} happens twice, an hour apart. \
                 Which one is meant?",
                state.hour, state.minute
            )));
            ui.add_space(6.0);
            let options = [
                (Some("EDT"), "EDT — the first pass, before the change", true),
                (Some("EST"), "EST — the second, after the change", true),
            ];
            ui.vertical(|ui| {
                for (value, label, _) in options {
                    if ui.radio(state.designator == value, label).clicked() {
                        state.set_designator(value.expect("both options name an offset"));
                    }
                }
            });
        });
}

// --------------------------------------------------------------------------------------------
// Results

fn results(ui: &mut egui::Ui, state: &mut EstimateState, working: &mut WorkingDir) {
    if state.outcome.is_none() {
        return;
    }
    {
        let outcome = state.outcome.as_ref().expect("just checked");
        ui.label(
            egui::RichText::new(&outcome.heading)
                .heading()
                .size(20.0)
                .color(theme::accent(ui)),
        );
        if let Some(workbook) = &state.workbook {
            widgets::note(ui, &workbook.name());
        }
        ui.add_space(10.0);

        headline(ui, &outcome.report);
    }

    ui.add_space(14.0);
    export_row(ui, state, working);
    ui.add_space(10.0);

    let sections = report_sections(&state.outcome.as_ref().expect("just checked").text);
    for section in &sections {
        egui::CollapsingHeader::new(&section.title)
            .default_open(true)
            .show(ui, |ui| widgets::monospace_block(ui, &section.body));
    }
}

/// The four figures, each a bracket rather than a point.
///
/// Two derivations times two units, which is the shape of the Estimates table in the report below,
/// so the headline and the table cannot tell different stories. Each row also names the segment
/// its figures were drawn from: the two derivations peak on the same segment in most reports
/// but need not, and a headline that hid that would be quoting two different windows as one.
fn headline(ui: &mut egui::Ui, report: &IntervalEstimates) {
    let (energy_seg, energy) = &report.energy_based_seg_estimate;
    let (count_seg, count) = &report.count_based_seg_estimate;

    egui::Grid::new("headline")
        .spacing([28.0, 6.0])
        .num_columns(4)
        .show(ui, |ui| {
            ui.label("");
            ui.label(egui::RichText::new("kW").strong());
            ui.label(egui::RichText::new("kVA").strong());
            ui.label(egui::RichText::new("Segment").strong());
            ui.end_row();

            // Coloured by derivation, not by high and low: with every figure now a bracket, the
            // two rows are two readings of the same window rather than the ends of one range.
            let energy_colour = theme::accent(ui);
            ui.label("Energy-based");
            figure(ui, energy.energy_based_kw, energy_colour);
            figure(ui, energy.energy_based_kva, energy_colour);
            ui.label(segment_label(energy_seg));
            ui.end_row();

            let count_colour = theme::ceiling(ui);
            ui.label("Count-based");
            figure(ui, count.count_based_kw, count_colour);
            figure(ui, count.count_based_kva, count_colour);
            ui.label(segment_label(count_seg));
            ui.end_row();
        });

    ui.add_space(8.0);
    widgets::note(
        ui,
        "Each figure runs from what the reported session times least support to what they most \
         support; the times are stated only to the minute. The peak is a 15-minute average \
         whatever interval length was asked for. See Estimates below, and Segments for the \
         segment each figure came from.",
    );
}

/// One bracketed figure. Both ends always, never a midpoint — the same rule the report follows,
/// and for the same reason: a single number would state a precision the source does not have.
fn figure(ui: &mut egui::Ui, value: Bracket<f64>, color: egui::Color32) {
    ui.label(
        egui::RichText::new(format!("{:.3}-{:.3}", value.min, value.max))
            .monospace()
            .size(18.0)
            .color(color),
    );
}

/// The segment a figure was drawn from, as local clock time — the same name the report's
/// `Segment` column uses, so the two can be read against each other.
fn segment_label(segment: &Segment) -> String {
    Zoned::new(segment.start(), time_zone())
        .strftime("%H:%M")
        .to_string()
}

fn export_row(ui: &mut egui::Ui, state: &mut EstimateState, working: &mut WorkingDir) {
    let Some(outcome) = &state.outcome else {
        return;
    };
    let text = outcome.text.clone();
    let default_name = state.default_save_name();

    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(text.clone());
        }
        if ui.button("Save…").clicked() {
            // The saved file is byte-for-byte what the command line prints, so a report kept from
            // the app and one piped from the terminal are the same document.
            if let Some(path) = widgets::dialog(working)
                .set_file_name(&default_name)
                .add_filter("Report", &["md"])
                .save_file()
            {
                // Remembered whether or not the write succeeds: it is where the user just chose to
                // be either way.
                working.remember(&path);
                if let Err(e) = std::fs::write(&path, &text) {
                    state.error = Some(format!("{}: {e}", path.display()));
                }
            }
        }
        widgets::note(
            ui,
            "The full report, exactly as the command line prints it.",
        );
    });
}
