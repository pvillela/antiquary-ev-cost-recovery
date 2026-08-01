//! The Convert tab: an Evolute session report CSV in, a workbook out.

use crate::state::ConvertState;
use crate::widgets;
use eframe::egui;
use std::path::PathBuf;

/// Draws the tab. Returns the workbook the user asked to carry over to the Estimate tab, if any:
/// the app offers the handoff and the user takes it, rather than being moved.
pub fn ui(ui: &mut egui::Ui, state: &mut ConvertState) -> Option<PathBuf> {
    widgets::heading(ui, "Convert a session report");
    widgets::note(
        ui,
        "Turns Evolute's monthly session report into a workbook, computing the derived columns and \
         flagging rows that need review. The workbook is written beside the CSV.",
    );
    ui.add_space(12.0);

    ui.horizontal(|ui| {
        if ui.button("Select CSV…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Session report", &["csv"])
                .pick_file()
        {
            state.select_csv(path);
        }
        match &state.csv {
            Some(path) => ui.label(path.display().to_string()),
            None => ui.weak("No file chosen"),
        };
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

    let mut carry_over = None;
    if let Some(outcome) = &state.outcome {
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Workbook written").strong());
        ui.label(outcome.workbook.display().to_string());

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
                 conversion. Row numbers are workbook rows, so a record duplicated to resolve a \
                 DST fold appears twice, once per copy.",
            );
            ui.add_space(6.0);
            widgets::monospace_block(ui, &outcome.anomalies.join("\n"));
        }

        ui.add_space(12.0);
        if ui.button("Estimate with this workbook").clicked() {
            carry_over = Some(outcome.workbook.clone());
        }
    }

    overwrite_prompt(ui, state);
    carry_over
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
