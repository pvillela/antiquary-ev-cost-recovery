//! The GUI for the two tools in this crate: converting an Evolute session report to a workbook,
//! and estimating the EV contribution to peak demand over an interval of interest.
//!
//! The same figures as `ev_peak_cli`, computed by the same library code, and saved reports are
//! byte-for-byte what that command prints. What differs is only the asking: the interval is chosen
//! from controls that offer nothing off-spec, rather than typed and then checked.

// A GUI has no use for a console, and on Windows one would open behind the window. Kept in debug
// builds, where a panic message on stderr is worth more than the tidiness.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod convert;
mod estimate;
mod state;
mod widgets;

/// The window and taskbar icon. Falling back to an empty icon is right if it ever fails to
/// decode: an app without an icon is a great deal better than an app that will not start.
fn icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .unwrap_or_default()
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(app::APP_NAME)
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([560.0, 420.0])
            .with_icon(icon()),
        ..Default::default()
    };

    eframe::run_native(
        app::APP_NAME,
        options,
        Box::new(|cc| {
            // egui follows the system light/dark setting on its own; all that is wanted here is
            // slightly larger text than the default, on top of whatever the display's scaling is.
            cc.egui_ctx.set_zoom_factor(1.15);
            Ok(Box::<app::App>::default())
        }),
    )
}
