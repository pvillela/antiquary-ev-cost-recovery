//! The Estimate tab: a workbook and an interval of interest in, the peak-contribution report out.

use crate::state::{EstimateState, WorkingDir, report_sections};
use crate::{theme, widgets};
use eframe::egui;
use egui_extras::DatePickerButton;
use ev_peak_contrib::{EstimateSet, IntervalLength, LEGAL_START_MINUTES, PowerEstimates};

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
                // simply not on offer. See README.md, "Interval of interest boundaries".
                let options = [
                    (IntervalLength::Quarter, "15 minutes", true),
                    (
                        IntervalLength::Hour,
                        "1 hour",
                        IntervalLength::Hour.allowed_from(state.minute),
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

        match &outcome.report.estimates {
            None => {
                ui.label("No charging sessions fall in this interval.");
            }
            Some(estimates) => headline(ui, estimates, !outcome.report.skew_margins.is_empty()),
        }
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

/// The four figures, as a bracket rather than a point.
///
/// `min_overlap <= nominal` on all four figures unconditionally, so the bracket needs no search:
/// its floor is the consumption figure of the lower reading and its ceiling the breaker-spec figure
/// of the nominal one. This is the same rule `report.rs` states in prose, and the two must agree.
fn headline(ui: &mut egui::Ui, estimates: &PowerEstimates, has_margins: bool) {
    let low: &EstimateSet = estimates.min_reading();
    let high: &EstimateSet = &estimates.nominal;

    egui::Grid::new("headline")
        .spacing([28.0, 6.0])
        .num_columns(3)
        .show(ui, |ui| {
            ui.label("");
            ui.label(egui::RichText::new("kW").strong());
            ui.label(egui::RichText::new("kVA").strong());
            ui.end_row();

            // The two rows are the two ends of the bracket, and are coloured as such: one figure
            // is the consumption-based floor, the other the breaker-spec ceiling.
            let floor = theme::accent(ui);
            ui.label("Likely at least");
            figure(ui, low.consumption_based_kw.value, floor);
            figure(ui, low.consumption_based_kva.value, floor);
            ui.end_row();

            let ceiling = theme::ceiling(ui);
            ui.label("Likely at most");
            figure(ui, high.breaker_specs_based_kw.value, ceiling);
            figure(ui, high.breaker_specs_based_kva.value, ceiling);
            ui.end_row();
        });

    ui.add_space(8.0);
    if estimates.min_overlap.is_some() {
        widgets::note(
            ui,
            "More than one reading of the data is defensible here: some group holds two sessions \
             that need not have overlapped each other. The floor above is the minimum-overlap \
             reading; quote the nominal figures under Estimates if only one set is wanted.",
        );
    } else {
        widgets::note(
            ui,
            "From the consumption-based figure to the breaker-spec-based one. See Estimates below \
             for both, and the group each was drawn from.",
        );
    }
    if has_margins {
        ui.add_space(4.0);
        widgets::note(
            ui,
            "A skew margin beside this interval comes out higher — see Skew margins below.",
        );
    }
}

fn figure(ui: &mut egui::Ui, value: f64, color: egui::Color32) {
    ui.label(
        egui::RichText::new(format!("{value:.3}"))
            .monospace()
            .size(22.0)
            .color(color),
    );
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
