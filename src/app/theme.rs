//! Colours and widget styling modelled on Adobe Audition's dark interface.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Panel background.
pub const PANEL: Color32 = Color32::from_gray(35);
/// Darker background for menus, the status bar and text fields.
pub const PANEL_DARK: Color32 = Color32::from_gray(26);
/// Accent blue for selected items and the time display.
pub const ACCENT: Color32 = Color32::from_rgb(38, 128, 235);
pub const TEXT: Color32 = Color32::from_gray(205);
pub const TEXT_DIM: Color32 = Color32::from_gray(140);

/// Editor backgrounds and the waveform.
pub const EDITOR_BG: Color32 = Color32::from_rgb(14, 22, 32);
pub const WAVE: Color32 = Color32::from_rgb(60, 215, 120);
pub const WAVE_SELECTED: Color32 = Color32::from_rgb(25, 70, 45);
pub const SELECTION_BG: Color32 = Color32::from_rgb(205, 214, 222);
pub const RULER_BG: Color32 = Color32::from_gray(30);
pub const RULER_TEXT: Color32 = Color32::from_gray(185);
pub const GRID: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);

/// Current-time indicator (playhead).
pub const PLAYHEAD: Color32 = Color32::from_rgb(235, 60, 60);
pub const TIMECODE: Color32 = Color32::from_rgb(70, 165, 245);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = Color32::from_gray(40);
    visuals.extreme_bg_color = PANEL_DARK;
    visuals.faint_bg_color = Color32::from_gray(42);
    visuals.code_bg_color = PANEL_DARK;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.hyperlink_color = Color32::from_rgb(90, 170, 250);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_gray(60));
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.menu_corner_radius = CornerRadius::same(3);

    let radius = CornerRadius::same(2);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(50));
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.noninteractive.corner_radius = radius;
    w.inactive.bg_fill = Color32::from_gray(54);
    w.inactive.weak_bg_fill = Color32::from_gray(54);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.corner_radius = radius;
    w.hovered.bg_fill = Color32::from_gray(70);
    w.hovered.weak_bg_fill = Color32::from_gray(70);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(95));
    w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.hovered.corner_radius = radius;
    w.active.bg_fill = Color32::from_gray(85);
    w.active.weak_bg_fill = Color32::from_gray(85);
    w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    w.active.corner_radius = radius;
    w.open.bg_fill = Color32::from_gray(54);
    w.open.weak_bg_fill = Color32::from_gray(54);
    w.open.corner_radius = radius;
    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(7.0, 3.0);
        style.spacing.menu_margin = egui::Margin::same(4);
        style.interaction.tooltip_delay = 0.3;
    });
}
