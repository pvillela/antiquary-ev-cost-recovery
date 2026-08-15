//! The window: the tab bar, the landing screen, and which tab is drawn.

use crate::state::{AppState, Tab};
use crate::{about, convert, estimate, theme};
use eframe::egui;

pub const APP_NAME: &str = "EV Peak Power Contribution";

/// The window and taskbar icon, and the mark shown on the landing screen.
///
/// Falling back to an empty icon is right if it ever fails to decode: an app without an icon is a
/// great deal better than an app that will not start.
pub fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .unwrap_or_default()
}

#[derive(Default)]
pub struct App {
    state: AppState,
    /// Uploaded on first use rather than at startup, so nothing is paid for it until the landing
    /// screen is actually drawn.
    logo: Option<egui::TextureHandle>,
    /// Window chrome rather than workflow state, so it sits here and not in `AppState`: opening
    /// the About window is not a step in either tab's work and must not survive as one.
    about_open: bool,
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The tab strip sits on its own surface, a shade off the panel below it and closed by a
        // line in the app's colour, so the chrome is visibly chrome.
        let bar = {
            let visuals = root_ui.visuals();
            egui::Frame::new()
                .fill(visuals.window_fill)
                .inner_margin(egui::Margin::symmetric(8, 6))
        };
        egui::Panel::top("tabs")
            .frame(bar)
            .show_separator_line(false)
            .show(root_ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, label) in [(Tab::Convert, "Convert"), (Tab::Estimate, "Estimate")] {
                        // `self.state.tab` is an Option, so on the landing screen neither tab reads
                        // as selected: entering one is a deliberate act, and leaving one is not
                        // possible.
                        let selected = self.state.tab == Some(tab);
                        let text = egui::RichText::new(label).strong();
                        let text = if selected {
                            text.color(theme::accent(ui))
                        } else {
                            text
                        };
                        if ui.selectable_label(selected, text).clicked() {
                            self.state.tab = Some(tab);
                        }
                    }
                    // Pushed to the far end, away from the tabs: About is not a third place to
                    // work, and reads as chrome only if it does not sit in their row.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("About").clicked() {
                            self.about_open = true;
                        }
                    });
                });
                let rect = ui.max_rect();
                let y = rect.bottom() + 6.0;
                ui.painter()
                    .hline(rect.x_range(), y, egui::Stroke::new(2.0, theme::accent(ui)));
            });

        egui::CentralPanel::default().show(root_ui, |ui| {
            // One scroll area for the whole tab: a long report scrolls together with the controls
            // that produced it, rather than being clipped into a panel of its own.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.state.tab {
                    None => {
                        let logo = self
                            .logo
                            .get_or_insert_with(|| {
                                let icon = icon();
                                let size = [icon.width as usize, icon.height as usize];
                                let image =
                                    egui::ColorImage::from_rgba_unmultiplied(size, &icon.rgba);
                                ui.ctx()
                                    .load_texture("logo", image, egui::TextureOptions::LINEAR)
                            })
                            .clone();
                        landing(ui, &mut self.state, &logo)
                    }
                    Some(Tab::Convert) => {
                        if convert::ui(ui, &mut self.state.convert, &mut self.state.working_dir) {
                            self.state.tab = Some(Tab::Estimate);
                        }
                    }
                    Some(Tab::Estimate) => {
                        // A conversion hands its workbook on when the user arrives here, by the
                        // button at the foot of the Convert tab or by clicking the tab itself.
                        // Collected in one place so the two routes cannot behave differently —
                        // which is exactly how they came to differ before.
                        if let Some(workbook) = self.state.convert.take_handoff() {
                            self.state.working_dir.remember(&workbook);
                            self.state.estimate.adopt_workbook(workbook);
                        }
                        estimate::ui(ui, &mut self.state.estimate, &mut self.state.working_dir)
                    }
                });
        });

        // After the panels, so it is drawn over whichever tab is open.
        about::window(root_ui.ctx(), &mut self.about_open);
    }
}

/// What the window holds before either tab has been chosen.
///
/// The two buttons say what the tabs mean, which two one-word tab labels cannot, and the workflow
/// is stated once here because this is the only moment a first-time user is looking for it.
fn landing(ui: &mut egui::Ui, state: &mut AppState, logo: &egui::TextureHandle) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.add(egui::Image::new(logo).fit_to_exact_size(egui::vec2(72.0, 72.0)));
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(APP_NAME)
                .size(26.0)
                .strong()
                .color(theme::accent(ui)),
        );
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
