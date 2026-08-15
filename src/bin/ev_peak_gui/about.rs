//! The About window: what this program is, and the terms the code inside it arrives under.
//!
//! The notices are embedded rather than shipped beside the binary because the app is downloaded
//! as a single file. A copy that travels in the archive can be deleted; a copy inside the
//! executable cannot, and the licences of the crates linked into it require that their notices
//! reach whoever holds the program.

use crate::app::APP_NAME;
use crate::theme;
use eframe::egui;
use std::sync::LazyLock;

/// Put here by `build.rs`: the generated notices in a release build, a short note saying they were
/// not generated in any other. Reaching through `OUT_DIR` is what lets the file be absent, since
/// `include_str!` cannot fall back on a missing path.
const NOTICES: &str = include_str!(concat!(env!("OUT_DIR"), "/third-party-notices.md"));

/// Split once rather than per frame. The text runs to a few thousand lines, and the scroll area
/// needs the count before it can decide which of them to draw.
static NOTICE_LINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| NOTICES.lines().collect());

/// Draws the window when `open`, and clears it when the user dismisses it.
pub fn window(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }

    let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
        // Wide enough for the licence texts, which are written to about 80 columns and would
        // otherwise be wrapped into an unreadable shape.
        ui.set_width(720.0);

        ui.label(
            egui::RichText::new(APP_NAME)
                .size(20.0)
                .strong()
                .color(theme::accent(ui)),
        );
        ui.label(egui::RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).weak());
        ui.add_space(8.0);
        ui.label("Copyright 2026 Paulo Villela");
        ui.add(
            egui::Label::new(
                "Licensed under either of the Apache License, Version 2.0 or the MIT license, \
                 at your option. The full texts are in LICENSE-APACHE and LICENSE-MIT.",
            )
            .wrap(),
        );

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Third-party notices").strong());
        ui.add_space(4.0);

        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .auto_shrink([false, false])
            // Only the visible rows are laid out. Handing the whole text to one label costs a
            // full layout pass over a quarter of a megabyte on every frame the window is open.
            .show_rows(ui, row_height, NOTICE_LINES.len(), |ui, rows| {
                for line in &NOTICE_LINES[rows] {
                    ui.add(
                        egui::Label::new(egui::RichText::new(*line).monospace())
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            });

        ui.add_space(8.0);
        ui.vertical_centered(|ui| ui.button("Close").clicked())
            .inner
    });

    // Escape and a click on the backdrop close it too, which is what `should_close` reports.
    if modal.inner || modal.should_close() {
        *open = false;
    }
}
