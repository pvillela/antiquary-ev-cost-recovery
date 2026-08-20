//! The Convert tab: an Evolute session report CSV in, a workbook out.

use crate::state::{ConvertState, WorkingDir};
use crate::widgets;
use eframe::egui;

/// Draws the tab. Returns whether the user asked to move on to the Estimate tab; the workbook
/// itself travels as a handoff on [`ConvertState`], collected wherever the user arrives.
pub fn ui(ui: &mut egui::Ui, state: &mut ConvertState, working: &mut WorkingDir) -> bool {
    widgets::heading(ui, "Convert a session report");
    widgets::note(
        ui,
        "Turns Evolute's monthly session report into a workbook, computing the derived columns and \
         flagging rows that need review. The workbook is written beside the CSV.",
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if ui.button("Select CSV…").clicked()
            && let Some(path) = widgets::dialog(working)
                .add_filter("Session report", &["csv"])
                .pick_file()
        {
            working.remember(&path);
            state.select_csv(path);
        }
        widgets::picked_file(ui, state.csv.as_deref(), "No file chosen");
    });

    ui.add_space(12.0);
    if ui
        .add_enabled(state.csv.is_some(), egui::Button::new("Convert"))
        .clicked()
    {
        state.start();
    }

    if let Some(message) = &state.error {
        ui.add_space(8.0);
        widgets::error_block(ui, message);
    }

    let mut move_on = false;
    if let Some(outcome) = &state.outcome {
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Workbook written").strong());
        ui.add(egui::Label::new(outcome.workbook.display().to_string()).wrap());

        ui.add_space(12.0);
        if outcome.anomalies.is_empty() {
            widgets::note(ui, "No row needed a judgement call.");
        } else {
            ui.label(egui::RichText::new(format!(
                "{} row(s) needed a judgement call",
                outcome.anomalies.len()
            )));
            widgets::note(
                ui,
                "These are recorded in the workbook's Anomalies column and do not stop the \
                 conversion. Row numbers are rows of the CSV, so a record duplicated to resolve a \
                 DST fold appears twice against the one row it came from; the -EDT/-EST suffix on \
                 the session id tells the two apart.",
            );
            ui.add_space(6.0);
            widgets::monospace_block(ui, &outcome.anomalies.join("\n"));
        }

        ui.add_space(12.0);
        if ui.button("Estimate with this workbook").clicked() {
            move_on = true;
        }
    }

    overwrite_prompt(ui, state);
    move_on
}

/// Asked before a workbook is replaced. Re-converting is the one act of this app that can destroy
/// something the estimates were argued from.
fn overwrite_prompt(ui: &mut egui::Ui, state: &mut ConvertState) {
    let Some(target) = state.confirm_overwrite.clone() else {
        return;
    };
    egui::Modal::new(egui::Id::new("confirm_overwrite")).show(ui.ctx(), |ui| {
        ui.set_max_width(460.0);
        ui.heading("Replace existing workbook?");
        ui.add_space(8.0);
        ui.add(egui::Label::new(target.display().to_string()).wrap());
        ui.add_space(8.0);
        widgets::note(
            ui,
            "This file already exists. Converting again overwrites it, and any estimate taken from \
             the old workbook no longer refers to a file you have.",
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Replace").clicked() {
                state.convert();
            }
            if ui.button("Cancel").clicked() {
                state.confirm_overwrite = None;
            }
        });
    });
}
