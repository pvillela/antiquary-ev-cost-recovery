//! Small pieces of chrome the two tabs share.

use eframe::egui;

/// The colour a failure is shown in, in whichever theme is on.
fn error_color(ui: &egui::Ui) -> egui::Color32 {
    ui.visuals().error_fg_color
}

/// A failure, shown where the action that failed was taken.
///
/// The text is the library's own message, unaltered, so that trouble reported from the app and
/// trouble reported from the command line can be compared word for word.
pub fn error_block(ui: &mut egui::Ui, message: &str) {
    let color = error_color(ui);
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.colored_label(color, "⚠");
                ui.add(egui::Label::new(egui::RichText::new(message).color(color)).wrap());
            });
        });
}

/// A note that is not a failure: what was written, what was found, what is not there.
pub fn note(ui: &mut egui::Ui, message: &str) {
    ui.add(egui::Label::new(egui::RichText::new(message).weak()).wrap());
}

/// Report text, shown exactly as the command line prints it.
pub fn monospace_block(ui: &mut egui::Ui, text: &str) {
    // Labels are selectable, so the text can still be picked up by hand. `Extend` keeps the
    // report's own wrapping — it is written to a fixed width and re-wrapping would break its
    // tables.
    ui.add(
        egui::Label::new(egui::RichText::new(text).monospace())
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}

/// A row of mutually exclusive choices, drawn as buttons rather than radio dots.
///
/// Returns the choice made this frame, if any. A choice that is not `enabled` is drawn greyed:
/// an interval of one hour from `HH:15` is not an error to be explained, it is simply not on
/// offer.
pub fn choice_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    current: T,
    options: &[(T, &str, bool)],
) -> Option<T> {
    let mut chosen = None;
    ui.horizontal(|ui| {
        for &(value, label, enabled) in options {
            let selected = value == current;
            let response = ui.add_enabled(enabled, egui::Button::selectable(selected, label));
            if response.clicked() && !selected {
                chosen = Some(value);
            }
        }
    });
    chosen
}
