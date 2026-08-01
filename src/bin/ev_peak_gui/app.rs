//! The window: the tab bar, the landing screen, and which tab is drawn.

use crate::state::{AppState, Tab};
use crate::{convert, estimate};
use eframe::egui;

pub const APP_NAME: &str = "EV Peak Power Contribution";

#[derive(Default)]
pub struct App {
    state: AppState,
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("tabs").show(root_ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::Convert, "Convert"), (Tab::Estimate, "Estimate")] {
                    // `self.state.tab` is an Option, so on the landing screen neither tab reads as
                    // selected: entering one is a deliberate act, and leaving one is not possible.
                    if ui
                        .selectable_label(self.state.tab == Some(tab), label)
                        .clicked()
                    {
                        self.state.tab = Some(tab);
                    }
                }
            });
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(root_ui, |ui| {
            // One scroll area for the whole tab: a long report scrolls together with the controls
            // that produced it, rather than being clipped into a panel of its own.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.state.tab {
                    None => landing(ui, &mut self.state),
                    Some(Tab::Convert) => {
                        if let Some(workbook) = convert::ui(ui, &mut self.state.convert) {
                            self.state.estimate.select_workbook(workbook);
                            self.state.tab = Some(Tab::Estimate);
                        }
                    }
                    Some(Tab::Estimate) => estimate::ui(ui, &mut self.state.estimate),
                });
        });
    }
}

/// What the window holds before either tab has been chosen.
///
/// The two buttons say what the tabs mean, which two one-word tab labels cannot, and the workflow
/// is stated once here because this is the only moment a first-time user is looking for it.
fn landing(ui: &mut egui::Ui, state: &mut AppState) {
    ui.add_space(28.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new(APP_NAME).size(26.0).strong());
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(egui::RichText::new(
                "Estimates what EV charging contributed to the building's peak power demand, over \
                 the interval a Toronto Hydro bill was charged on.",
            ))
            .wrap(),
        );
        ui.add_space(28.0);

        let width = 420.0;
        if ui
            .add_sized(
                [width, 40.0],
                egui::Button::new("Convert a session report  (CSV to Excel)"),
            )
            .clicked()
        {
            state.tab = Some(Tab::Convert);
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Once a month, when Evolute's session report arrives.").weak(),
        );

        ui.add_space(20.0);
        if ui
            .add_sized(
                [width, 40.0],
                egui::Button::new("Estimate peak contribution"),
            )
            .clicked()
        {
            state.tab = Some(Tab::Estimate);
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("For each interval of interest on the bill, using that workbook.")
                .weak(),
        );
    });
}
