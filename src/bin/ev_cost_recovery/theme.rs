//! Visuals for both themes.
//!
//! egui's defaults are deliberately low-contrast: in the dark theme ordinary text is `gray(140)`
//! against a `gray(27)` panel, buttons carry no outline at all, and secondary text is dimmed to 60%
//! on top of that. The result reads as a wash of grey. What follows keeps egui's shape and spacing
//! and changes only what separates one thing from another:
//!
//! - **Text is brighter.** Near-white on dark, near-black on light, rather than mid-grey on either.
//! - **Widgets are outlined.** Every button, dropdown and field gets a visible border, so a control
//!   is distinguishable from the panel behind it without having to be hovered first.
//! - **The panel is pushed away from the widgets.** The window background goes darker (or lighter)
//!   while widget fills stay where they are, which widens the gap between the two.
//! - **Secondary text stays secondary but stays legible**, at 78% rather than 60%.

use eframe::egui::{Color32, Stroke, Theme, Ui, Visuals, style::WidgetVisuals};

/// Applies the visuals to both themes, so the app looks deliberate whichever one the system is on.
pub fn apply(ctx: &eframe::egui::Context) {
    ctx.style_mut_of(Theme::Dark, |style| dark(&mut style.visuals));
    ctx.style_mut_of(Theme::Light, |style| light(&mut style.visuals));
}

/// The app's colour, taken from the icon: a teal that carries headings, the selected tab and the
/// lower of the two estimates. One accent, used in few places, is what keeps it from looking busy.
pub fn accent(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(52, 200, 165)
    } else {
        Color32::from_rgb(10, 112, 92)
    }
}

/// The colour of the upper estimate — the breaker-spec bound. Warmer than [`accent`] because it is
/// the ceiling of the bracket, not because anything is wrong.
pub fn ceiling(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(255, 190, 85)
    } else {
        Color32::from_rgb(155, 95, 0)
    }
}

/// Outline, fill and text for one widget state, in one line at each call site.
fn widget(fill: u8, stroke: u8, text: u8, stroke_width: f32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: Color32::from_gray(fill),
        weak_bg_fill: Color32::from_gray(fill),
        bg_stroke: Stroke::new(stroke_width, Color32::from_gray(stroke)),
        fg_stroke: Stroke::new(1.0, Color32::from_gray(text)),
        corner_radius: 3.into(),
        expansion: 0.0,
    }
}

fn dark(v: &mut Visuals) {
    // The window recedes; the widgets keep their fill, so the two no longer read as one surface.
    v.panel_fill = Color32::from_gray(20);
    v.window_fill = Color32::from_gray(26);
    v.extreme_bg_color = Color32::from_gray(12);
    v.faint_bg_color = Color32::from_gray(30);
    v.code_bg_color = Color32::from_gray(34);
    v.window_stroke = Stroke::new(1.0, Color32::from_gray(90));

    //                       fill  outline text  outline width
    v.widgets.noninteractive = widget(20, 85, 225, 1.0); // labels, separators, indent lines
    v.widgets.inactive = widget(62, 105, 235, 1.0); // a button at rest — now outlined
    v.widgets.hovered = widget(80, 175, 255, 1.5);
    v.widgets.active = widget(98, 255, 255, 1.5);
    v.widgets.open = widget(70, 130, 240, 1.0);

    v.selection.bg_fill = Color32::from_rgb(14, 105, 90);
    v.selection.stroke = Stroke::new(1.0, Color32::from_gray(255));

    // Pure red on a dark panel is hard to read; this stays unmistakably an error and legible.
    v.error_fg_color = Color32::from_rgb(255, 105, 95);
    v.warn_fg_color = Color32::from_rgb(255, 175, 60);
    v.hyperlink_color = Color32::from_rgb(52, 200, 165);

    common(v);
}

fn light(v: &mut Visuals) {
    // Mirror image: the panel takes a light grey so that white widgets stand out from it.
    v.panel_fill = Color32::from_gray(233);
    v.window_fill = Color32::from_gray(245);
    v.extreme_bg_color = Color32::from_gray(255);
    v.faint_bg_color = Color32::from_gray(240);
    v.code_bg_color = Color32::from_gray(228);
    v.window_stroke = Stroke::new(1.0, Color32::from_gray(150));

    //                       fill  outline text  outline width
    v.widgets.noninteractive = widget(233, 165, 25, 1.0);
    v.widgets.inactive = widget(252, 145, 20, 1.0);
    v.widgets.hovered = widget(255, 85, 0, 1.5);
    v.widgets.active = widget(215, 0, 0, 1.5);
    v.widgets.open = widget(248, 120, 20, 1.0);

    v.selection.bg_fill = Color32::from_rgb(168, 222, 208);
    v.selection.stroke = Stroke::new(1.0, Color32::from_gray(0));

    v.error_fg_color = Color32::from_rgb(180, 0, 0);
    v.warn_fg_color = Color32::from_rgb(160, 90, 0);
    v.hyperlink_color = Color32::from_rgb(10, 112, 92);

    common(v);
}

fn common(v: &mut Visuals) {
    // Secondary text — the explanatory notes under headings — should read as secondary without
    // becoming unreadable, which is what egui's 60% does over a low-contrast base.
    v.weak_text_alpha = 0.78;
    // A striped table is easier to follow than an unstriped one, and the stripe is now visible.
    v.striped = true;
    v.button_frame = true;
    v.collapsing_header_frame = true;
    v.indent_has_left_vline = true;
}
