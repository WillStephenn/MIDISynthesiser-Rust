//! "June's Logue" modern/vintage terracotta theme (port of
//! `resources/synth/ui/junes_logue.css`).
//!
//! The JavaFX stylesheet defined a five-colour palette and applied it to
//! every control class; here the same palette is exposed as constants and
//! applied once to the egui [`egui::Style`].

use eframe::egui;
use egui::Color32;

/* Colour Palette (from the CSS header):
   Black: #0A0908
   Chocolate cosmos: #49111C
   White smoke: #F2F4F3
   Burnt orange: #BA5624
   Orange peel: #FF9F1C
*/

/// Background black (`#0A0908`).
pub const BLACK: Color32 = Color32::from_rgb(0x0A, 0x09, 0x08);
/// Chocolate cosmos section background (`#49111C`).
pub const CHOCOLATE_COSMOS: Color32 = Color32::from_rgb(0x49, 0x11, 0x1C);
/// White smoke text colour (`#F2F4F3`).
pub const WHITE_SMOKE: Color32 = Color32::from_rgb(0xF2, 0xF4, 0xF3);
/// Burnt orange border/accent colour (`#BA5624`).
pub const BURNT_ORANGE: Color32 = Color32::from_rgb(0xBA, 0x56, 0x24);
/// Orange peel highlight colour (`#FF9F1C`).
pub const ORANGE_PEEL: Color32 = Color32::from_rgb(0xFF, 0x9F, 0x1C);

/// Applies the June's Logue theme to the egui context (the equivalent of
/// attaching `junes_logue.css` to the JavaFX scene).
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let visuals = &mut style.visuals;

    visuals.dark_mode = true;
    visuals.override_text_color = Some(WHITE_SMOKE);
    visuals.panel_fill = BLACK;
    visuals.window_fill = CHOCOLATE_COSMOS;
    visuals.window_stroke = egui::Stroke::new(2.0, BURNT_ORANGE);
    visuals.extreme_bg_color = BLACK; // slider track / text-edit background

    // Widget states (sliders, combo boxes, buttons).
    visuals.widgets.noninteractive.bg_fill = BLACK;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BURNT_ORANGE);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, WHITE_SMOKE);

    visuals.widgets.inactive.bg_fill = BURNT_ORANGE; // slider handle
    visuals.widgets.inactive.weak_bg_fill = BLACK; // combo-box body
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(2.0, BURNT_ORANGE);
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, WHITE_SMOKE);

    visuals.widgets.hovered.bg_fill = ORANGE_PEEL;
    visuals.widgets.hovered.weak_bg_fill = BLACK;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(2.0, ORANGE_PEEL);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, ORANGE_PEEL);

    visuals.widgets.active.bg_fill = ORANGE_PEEL;
    visuals.widgets.active.weak_bg_fill = CHOCOLATE_COSMOS;
    visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, ORANGE_PEEL);
    visuals.widgets.active.fg_stroke = egui::Stroke::new(1.5, ORANGE_PEEL);

    visuals.widgets.open.bg_fill = CHOCOLATE_COSMOS;
    visuals.widgets.open.weak_bg_fill = CHOCOLATE_COSMOS;
    visuals.widgets.open.bg_stroke = egui::Stroke::new(2.0, ORANGE_PEEL);

    // Selected items (combo-box menu hover/selection).
    visuals.selection.bg_fill = BURNT_ORANGE;
    visuals.selection.stroke = egui::Stroke::new(1.0, ORANGE_PEEL);

    // Fill the slider track up to the handle with the accent colour.
    visuals.slider_trailing_fill = true;

    style.spacing.slider_width = 200.0;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);

    ctx.set_global_style(style);
}

/// Styles a label like the CSS `.section-header` class (orange, large, bold).
pub fn section_header(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .color(ORANGE_PEEL)
        .size(18.0)
        .strong()
}

/// Styles a label like the CSS `.parameter-label` class (white, small, bold).
pub fn parameter_label(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .color(WHITE_SMOKE)
        .size(12.0)
        .strong()
}

/// Styles a label like the CSS `.value-readout` class (orange, tiny).
pub fn value_readout(text: &str) -> egui::RichText {
    egui::RichText::new(text).color(ORANGE_PEEL).size(10.0)
}
