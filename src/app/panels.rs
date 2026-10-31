//! Menu bar, tool bar, side panel (Files, History, Effects), transport and zoom controls,
//! Levels and Selection/View panels, status bar and dialogs.

use eframe::egui::{
    self, Align2, Color32, FontId, Rect, Response, RichText, Sense, Stroke, pos2, vec2,
};

use super::util::{channel_name, format_bytes, format_time, parse_time};
use super::{AfterSave, AudioPaintApp, EffectDialog, SideTab, Tool, theme};
use crate::audio::SaveFormat;
use crate::colormap::Colormap;
use crate::edit::{EditOp, Effect, PasteMode};
use crate::spectrum::{FFT_SIZES, WindowKind};

const SHORTCUTS: &str = "\
Files      Ctrl+O open, Ctrl+S save as, Ctrl+W close
Edit       Ctrl+Z undo, Ctrl+Y or Ctrl+Shift+Z redo
           Ctrl+X cut, Ctrl+C copy, Ctrl+V paste (insert), Ctrl+Shift+V overwrite paste
           Delete mute the selection, Ctrl+A select all, Esc deselect
Tools      T time selection, E marquee, D lasso, B paintbrush, S spot healing, H hand
           Shift adds to a spectral selection, Alt subtracts; [ and ] brush size
Transport  Space play/stop, Home/End move to start/end
View       Wheel zoom time, Ctrl+wheel zoom frequency (spectral) or amplitude (waveform)
           Shift+wheel scroll time, Alt+wheel scroll frequency, middle-drag scroll
           Right-drag zoom to a box, + / - zoom time, F zoom out full
Selection  Drag in the waveform (or the spectral display with the time tool),
           drag an edge to resize, Shift+click extends, double-click selects all";

/// Icons for the transport and zoom buttons, drawn with shapes.
#[derive(Clone, Copy)]
enum Icon {
    Stop,
    Play,
    Pause,
    ToStart,
    Rewind,
    FastForward,
    ToEnd,
    Loop,
    ZoomInAmp,
    ZoomOutAmp,
    ZoomInTime,
    ZoomOutTime,
    ZoomFull,
    ZoomSelection,
    ToolTime,
    ToolMarquee,
    ToolLasso,
    ToolBrush,
    ToolHeal,
    ToolHand,
    SelectAll,
    Deselect,
    Cut,
    Copy,
    Paste,
}

fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Time => Icon::ToolTime,
        Tool::Marquee => Icon::ToolMarquee,
        Tool::Lasso => Icon::ToolLasso,
        Tool::Brush => Icon::ToolBrush,
        Tool::Heal => Icon::ToolHeal,
        Tool::Hand => Icon::ToolHand,
    }
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, hint: &str, enabled: bool, on: bool) -> Response {
    icon_text_button(ui, icon, "", hint, enabled, on)
}

/// Icon button with an optional text label to the right of the icon.
fn icon_text_button(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    hint: &str,
    enabled: bool,
    on: bool,
) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let text_color = if enabled {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(100)
    };
    let galley = (!text.is_empty()).then(|| {
        ui.painter()
            .layout_no_wrap(text.to_string(), FontId::proportional(13.0), text_color)
    });
    let width = galley.as_ref().map_or(30.0, |g| 28.0 + g.size().x + 8.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), sense);
    let resp = resp.on_hover_text(hint);
    let painter = ui.painter_at(rect);
    let bg = if on {
        theme::ACCENT
    } else if enabled && resp.hovered() {
        Color32::from_gray(70)
    } else {
        Color32::from_gray(48)
    };
    painter.rect_filled(rect.shrink(1.0), 3.0, bg);
    let color = if enabled {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(100)
    };
    match galley {
        Some(galley) => {
            draw_icon(
                &painter,
                pos2(rect.left() + 15.0, rect.center().y),
                icon,
                color,
            );
            let pos = pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0);
            painter.galley(pos, galley, text_color);
        }
        None => draw_icon(&painter, rect.center(), icon, color),
    }
    resp
}

fn draw_icon(p: &egui::Painter, c: egui::Pos2, icon: Icon, color: Color32) {
    let tri = |pts: [(f32, f32); 3]| {
        egui::Shape::convex_polygon(
            pts.iter().map(|(x, y)| pos2(c.x + x, c.y + y)).collect(),
            color,
            Stroke::NONE,
        )
    };
    let bar = |x0: f32, x1: f32| {
        p.rect_filled(
            Rect::from_min_max(pos2(c.x + x0, c.y - 5.0), pos2(c.x + x1, c.y + 5.0)),
            0.0,
            color,
        )
    };
    let stroke = Stroke::new(1.6, color);
    let magnifier = |sign: Option<bool>| {
        let m = pos2(c.x - 2.0, c.y - 2.0);
        p.circle_stroke(m, 5.0, stroke);
        p.line_segment(
            [pos2(m.x + 3.6, m.y + 3.6), pos2(m.x + 7.5, m.y + 7.5)],
            stroke,
        );
        if let Some(plus) = sign {
            p.line_segment([pos2(m.x - 2.5, m.y), pos2(m.x + 2.5, m.y)], stroke);
            if plus {
                p.line_segment([pos2(m.x, m.y - 2.5), pos2(m.x, m.y + 2.5)], stroke);
            }
        }
        m
    };
    match icon {
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, color);
        }
        Icon::Play => {
            p.add(tri([(-4.0, -6.0), (-4.0, 6.0), (6.0, 0.0)]));
        }
        Icon::Pause => {
            bar(-4.5, -1.5);
            bar(1.5, 4.5);
        }
        Icon::ToStart => {
            bar(-6.0, -4.0);
            p.add(tri([(5.0, -6.0), (5.0, 6.0), (-4.0, 0.0)]));
        }
        Icon::Rewind => {
            p.add(tri([(0.0, -5.0), (0.0, 5.0), (-7.0, 0.0)]));
            p.add(tri([(7.0, -5.0), (7.0, 5.0), (0.0, 0.0)]));
        }
        Icon::FastForward => {
            p.add(tri([(-7.0, -5.0), (-7.0, 5.0), (0.0, 0.0)]));
            p.add(tri([(0.0, -5.0), (0.0, 5.0), (7.0, 0.0)]));
        }
        Icon::ToEnd => {
            p.add(tri([(-5.0, -6.0), (-5.0, 6.0), (4.0, 0.0)]));
            bar(4.0, 6.0);
        }
        Icon::Loop => {
            let r = Rect::from_center_size(c, vec2(14.0, 9.0));
            p.rect_stroke(r, 4.0, stroke, egui::StrokeKind::Middle);
            p.add(tri([(1.0, -7.5), (1.0, -1.5), (5.0, -4.5)]));
        }
        Icon::ZoomInAmp | Icon::ZoomOutAmp => {
            magnifier(Some(matches!(icon, Icon::ZoomInAmp)));
            // vertical arrow: amplitude
            let x = c.x + 9.0;
            p.line_segment(
                [pos2(x, c.y - 7.0), pos2(x, c.y + 7.0)],
                Stroke::new(1.2, color),
            );
            p.add(tri([(7.0, -5.0), (11.0, -5.0), (9.0, -8.0)]));
            p.add(tri([(7.0, 5.0), (11.0, 5.0), (9.0, 8.0)]));
        }
        Icon::ZoomInTime | Icon::ZoomOutTime => {
            magnifier(Some(matches!(icon, Icon::ZoomInTime)));
            // horizontal arrow: time
            let y = c.y + 9.0;
            p.line_segment(
                [pos2(c.x - 9.0, y), pos2(c.x + 9.0, y)],
                Stroke::new(1.2, color),
            );
        }
        Icon::ZoomFull => {
            let m = magnifier(None);
            p.rect_stroke(
                Rect::from_center_size(m, vec2(5.0, 5.0)),
                0.0,
                Stroke::new(1.2, color),
                egui::StrokeKind::Middle,
            );
        }
        Icon::ZoomSelection => {
            let m = magnifier(None);
            p.rect_filled(Rect::from_center_size(m, vec2(4.0, 4.0)), 0.0, color);
        }
        Icon::ToolTime => {
            // I-beam
            p.vline(c.x, c.y - 7.0..=c.y + 7.0, stroke);
            p.hline(c.x - 3.5..=c.x + 3.5, c.y - 7.0, stroke);
            p.hline(c.x - 3.5..=c.x + 3.5, c.y + 7.0, stroke);
        }
        Icon::ToolMarquee | Icon::SelectAll | Icon::Deselect => {
            let r = Rect::from_center_size(c, vec2(16.0, 12.0));
            if matches!(icon, Icon::SelectAll) {
                p.rect_filled(r, 0.0, color.gamma_multiply(0.35));
            }
            let corners = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            p.extend(egui::Shape::dashed_line(
                &corners,
                Stroke::new(1.2, color),
                2.5,
                2.0,
            ));
            if matches!(icon, Icon::Deselect) {
                let s = Stroke::new(1.6, color);
                p.line_segment([pos2(c.x - 3.5, c.y - 3.5), pos2(c.x + 3.5, c.y + 3.5)], s);
                p.line_segment([pos2(c.x + 3.5, c.y - 3.5), pos2(c.x - 3.5, c.y + 3.5)], s);
            }
        }
        Icon::ToolLasso => {
            let loop_points: Vec<egui::Pos2> = (0..=24)
                .map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    pos2(c.x + 7.0 * a.cos(), c.y - 2.0 + 4.5 * a.sin())
                })
                .collect();
            p.add(egui::Shape::line(loop_points, stroke));
            p.add(egui::Shape::line(
                vec![
                    pos2(c.x - 4.0, c.y + 2.0),
                    pos2(c.x - 5.0, c.y + 5.0),
                    pos2(c.x - 2.5, c.y + 7.0),
                ],
                stroke,
            ));
        }
        Icon::ToolBrush | Icon::ToolHeal => {
            p.line_segment(
                [pos2(c.x + 7.0, c.y - 7.0), pos2(c.x - 0.5, c.y + 0.5)],
                Stroke::new(2.4, color),
            );
            p.add(tri([(-1.5, -1.5), (1.5, 1.5), (-6.5, 6.5)]));
            p.circle_filled(pos2(c.x - 1.8, c.y + 1.8), 2.4, color);
            if matches!(icon, Icon::ToolHeal) {
                // small "+" for healing
                let s = Stroke::new(1.6, color);
                p.line_segment([pos2(c.x - 8.0, c.y - 5.0), pos2(c.x - 2.0, c.y - 5.0)], s);
                p.line_segment([pos2(c.x - 5.0, c.y - 8.0), pos2(c.x - 5.0, c.y - 2.0)], s);
            }
        }
        Icon::ToolHand => {
            let palm = Rect::from_min_max(pos2(c.x - 5.0, c.y - 1.0), pos2(c.x + 5.0, c.y + 7.0));
            p.rect_filled(palm, 3.0, color);
            for (i, top) in [-6.0, -8.0, -8.0, -6.5].iter().enumerate() {
                let x = c.x - 4.0 + i as f32 * 2.7;
                p.line_segment(
                    [pos2(x, c.y + 1.0), pos2(x, c.y + top)],
                    Stroke::new(1.8, color),
                );
            }
            p.line_segment(
                [pos2(c.x - 4.5, c.y + 3.0), pos2(c.x - 8.0, c.y - 1.0)],
                Stroke::new(1.8, color),
            );
        }
        Icon::Cut => {
            let s = Stroke::new(1.4, color);
            p.circle_stroke(pos2(c.x - 4.0, c.y + 4.5), 2.6, s);
            p.circle_stroke(pos2(c.x + 4.0, c.y + 4.5), 2.6, s);
            p.line_segment([pos2(c.x - 2.2, c.y + 2.5), pos2(c.x + 4.0, c.y - 7.5)], s);
            p.line_segment([pos2(c.x + 2.2, c.y + 2.5), pos2(c.x - 4.0, c.y - 7.5)], s);
        }
        Icon::Copy => {
            let s = Stroke::new(1.4, color);
            let back = Rect::from_min_size(pos2(c.x - 6.5, c.y - 7.0), vec2(9.0, 11.0));
            let front = back.translate(vec2(4.0, 3.5));
            p.rect_stroke(back, 1.0, s, egui::StrokeKind::Middle);
            p.rect_filled(front, 1.0, Color32::from_gray(48));
            p.rect_stroke(front, 1.0, s, egui::StrokeKind::Middle);
        }
        Icon::Paste => {
            let s = Stroke::new(1.4, color);
            let board = Rect::from_center_size(pos2(c.x, c.y + 1.0), vec2(12.0, 14.0));
            p.rect_stroke(board, 1.5, s, egui::StrokeKind::Middle);
            p.rect_filled(
                Rect::from_center_size(pos2(c.x, board.top()), vec2(6.0, 3.5)),
                1.0,
                color,
            );
            p.hline(c.x - 3.5..=c.x + 3.5, c.y + 1.0, s);
            p.hline(c.x - 3.5..=c.x + 3.5, c.y + 4.5, s);
        }
    }
}

impl AudioPaintApp {
    // --- menu and tool bar -----------------------------------------------------------------

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let has_audio = self.loaded.is_some();
        let idle = !self.busy();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui
                    .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
                    .clicked()
                {
                    self.pick_files(&ctx);
                }
                if ui
                    .add_enabled(
                        has_audio,
                        egui::Button::new("Close").shortcut_text("Ctrl+W"),
                    )
                    .clicked()
                {
                    self.close_doc(self.active, false, &ctx);
                }
                ui.separator();
                if ui
                    .add_enabled(
                        has_audio && self.save_job.is_none(),
                        egui::Button::new("Save As...").shortcut_text("Ctrl+S"),
                    )
                    .clicked()
                {
                    self.save_dialog = true;
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });

            ui.menu_button("Edit", |ui| {
                self.edit_items(ui, &ctx, true);
            });

            ui.menu_button("Effects", |ui| {
                ui.add_enabled_ui(has_audio && idle, |ui| self.effect_items(ui, &ctx));
            });

            ui.menu_button("View", |ui| {
                let shown = (self.show_waveform, self.show_spectral);
                ui.checkbox(&mut self.show_waveform, "Show Waveform");
                ui.checkbox(&mut self.show_spectral, "Show Spectral Frequency Display");
                if !self.show_waveform && !self.show_spectral {
                    // One of the two displays always stays visible.
                    (self.show_waveform, self.show_spectral) = shown;
                }
                let was_log = self.log_scale;
                ui.checkbox(&mut self.log_scale, "Logarithmic Frequency Scale");
                if was_log != self.log_scale {
                    self.view.u0 = 0.0;
                    self.view.u1 = 1.0;
                }
                ui.separator();
                ui.add_enabled_ui(has_audio, |ui| {
                    if ui
                        .add(egui::Button::new("Zoom In Time").shortcut_text("="))
                        .clicked()
                    {
                        self.zoom_time_center(1.5);
                    }
                    if ui
                        .add(egui::Button::new("Zoom Out Time").shortcut_text("-"))
                        .clicked()
                    {
                        self.zoom_time_center(1.0 / 1.5);
                    }
                    if ui.button("Zoom In Amplitude").clicked() {
                        self.zoom_amp(1.5);
                    }
                    if ui.button("Zoom Out Amplitude").clicked() {
                        self.zoom_amp(1.0 / 1.5);
                    }
                    if ui.button("Zoom to Selection").clicked() {
                        self.zoom_to_selection();
                    }
                    if ui
                        .add(egui::Button::new("Zoom Out Full").shortcut_text("F"))
                        .clicked()
                    {
                        self.zoom_full();
                    }
                });
                ui.separator();
                ui.checkbox(&mut self.follow_playback, "Follow Playback");
                ui.separator();
                if ui.button("Spectral Display Settings...").clicked() {
                    self.settings_window = true;
                }
            });

            ui.menu_button("Window", |ui| {
                ui.checkbox(&mut self.show_side_panel, "Files, History and Effects");
                ui.checkbox(&mut self.show_levels, "Levels");
                ui.checkbox(&mut self.show_selection_view, "Selection/View");
                ui.checkbox(&mut self.show_status_bar, "Status Bar");
            });

            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard Shortcuts").clicked() {
                    self.shortcuts_window = true;
                }
                if ui.button("About Audio Paint").clicked() {
                    self.about_window = true;
                }
            });
        });
    }

    /// Entries of the Edit menu, also used by the editor's right-click menu.
    fn edit_items(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, with_tools: bool) {
        let has_audio = self.loaded.is_some();
        let idle = !self.busy();
        let undo_label = match self.history.can_undo() {
            true => format!("Undo {}", self.history.entries[self.history.pos].0),
            false => "Undo".into(),
        };
        if ui
            .add_enabled(
                self.history.can_undo() && idle,
                egui::Button::new(undo_label).shortcut_text("Ctrl+Z"),
            )
            .clicked()
        {
            self.undo(ctx);
        }
        let redo_label = match self.history.can_redo() {
            true => format!("Redo {}", self.history.entries[self.history.pos + 1].0),
            false => "Redo".into(),
        };
        if ui
            .add_enabled(
                self.history.can_redo() && idle,
                egui::Button::new(redo_label).shortcut_text("Ctrl+Y"),
            )
            .clicked()
        {
            self.redo(ctx);
        }
        ui.separator();
        let can_copy = self.time_range().is_some();
        let can_paste = has_audio && self.clipboard.is_some() && idle;
        if ui
            .add_enabled(
                self.selection.is_some() && idle,
                egui::Button::new("Cut").shortcut_text("Ctrl+X"),
            )
            .on_hover_text("Copy the selected time range and remove it from the file")
            .clicked()
        {
            self.cut(ctx);
        }
        if ui
            .add_enabled(can_copy, egui::Button::new("Copy").shortcut_text("Ctrl+C"))
            .clicked()
        {
            self.copy(ctx);
        }
        if ui
            .add_enabled(
                can_paste,
                egui::Button::new("Paste").shortcut_text("Ctrl+V"),
            )
            .clicked()
        {
            self.paste(ctx, PasteMode::Insert);
        }
        if ui
            .add_enabled(
                can_paste,
                egui::Button::new("Overwrite Paste").shortcut_text("Ctrl+Shift+V"),
            )
            .clicked()
        {
            self.paste(ctx, PasteMode::Overwrite);
        }
        if ui
            .add_enabled(
                self.has_selection() && idle,
                egui::Button::new("Delete (Silence)").shortcut_text("Del"),
            )
            .clicked()
        {
            self.edit_selection(ctx, EditOp::Gain(0.0));
        }
        ui.separator();
        if ui
            .add_enabled(
                has_audio,
                egui::Button::new("Select All").shortcut_text("Ctrl+A"),
            )
            .clicked()
        {
            self.select_all();
        }
        if ui
            .add_enabled(
                self.has_selection(),
                egui::Button::new("Deselect All").shortcut_text("Esc"),
            )
            .clicked()
        {
            self.deselect();
        }
        if !with_tools {
            return;
        }
        ui.separator();
        ui.menu_button("Tools", |ui| {
            for tool in Tool::ALL {
                let text = format!("{} ({:?})", tool.label(), tool.key());
                if ui.radio(self.tool == tool, text).clicked() {
                    self.tool = tool;
                }
            }
        });
    }

    /// Right-click menu of the waveform and the spectral display.
    pub(super) fn editor_context_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let enabled = self.loaded.is_some() && !self.busy();
        self.edit_items(ui, &ctx, false);
        ui.separator();
        ui.menu_button("Effects", |ui| {
            ui.add_enabled_ui(enabled, |ui| self.effect_items(ui, &ctx));
        });
        ui.separator();
        if ui.button("Zoom to Selection").clicked() {
            self.zoom_to_selection();
        }
        if ui
            .add(egui::Button::new("Zoom Out Full").shortcut_text("F"))
            .clicked()
        {
            self.zoom_full();
        }
    }

    /// Entries shared by the Effects menu and the Effects panel.
    fn effect_items(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Amplify...").clicked() {
            self.effect_dialog = Some(EffectDialog::Amplify(self.gain_db));
        }
        if ui.button("Normalize...").clicked() {
            self.effect_dialog = Some(EffectDialog::Normalize(-0.1));
        }
        ui.separator();
        for effect in [
            Effect::FadeIn,
            Effect::FadeOut,
            Effect::Silence,
            Effect::Invert,
            Effect::Reverse,
        ] {
            if ui.button(effect.name()).clicked() {
                self.apply_effect(ctx, effect);
            }
        }
        ui.separator();
        let spectral = self.has_selection();
        if ui
            .add_enabled(spectral, egui::Button::new("Heal Selection"))
            .on_hover_text("Rebuild the selected area from the surrounding audio")
            .clicked()
        {
            self.edit_selection(ctx, EditOp::Heal);
        }
    }

    pub(super) fn tool_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("Waveform").strong().color(theme::ACCENT))
                .on_hover_text("Waveform editor workspace");
            ui.separator();
            for tool in Tool::ALL {
                let on = self.tool == tool;
                let label = tool.short_label();
                if icon_text_button(ui, tool_icon(tool), label, tool.hint(), true, on).clicked() {
                    self.tool = tool;
                }
            }
            ui.separator();
            ui.label(RichText::new("Brush size").color(theme::TEXT_DIM));
            ui.add(egui::Slider::new(&mut self.brush_radius, 2.0..=80.0).integer())
                .on_hover_text("Size of the Paintbrush and Spot Healing brushes ([ and ])");
            ui.separator();

            let ctx = ui.ctx().clone();
            let has_audio = self.loaded.is_some();
            let idle = !self.busy();
            if icon_text_button(
                ui,
                Icon::SelectAll,
                "Select All",
                "Select the whole file (Ctrl+A)",
                has_audio,
                false,
            )
            .clicked()
            {
                self.select_all();
            }
            if icon_text_button(
                ui,
                Icon::Deselect,
                "Deselect",
                "Deselect All (Esc)",
                self.has_selection(),
                false,
            )
            .clicked()
            {
                self.deselect();
            }
            let can_cut = self.selection.is_some() && idle;
            let hint = "Copy the time selection and remove it (Ctrl+X)";
            if icon_text_button(ui, Icon::Cut, "Cut", hint, can_cut, false).clicked() {
                self.cut(&ctx);
            }
            let can_copy = self.time_range().is_some();
            let hint = "Copy the selected time range (Ctrl+C)";
            if icon_text_button(ui, Icon::Copy, "Copy", hint, can_copy, false).clicked() {
                self.copy(&ctx);
            }
            let can_paste = has_audio && self.clipboard.is_some() && idle;
            if icon_text_button(
                ui,
                Icon::Paste,
                "Paste",
                "Paste at the playhead (Ctrl+V)",
                can_paste,
                false,
            )
            .clicked()
            {
                self.paste(&ctx, PasteMode::Insert);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // One of the two displays always stays visible.
                if ui
                    .selectable_label(self.show_spectral, "Spectral")
                    .on_hover_text("Show the spectral frequency display (one display stays on)")
                    .clicked()
                    && (!self.show_spectral || self.show_waveform)
                {
                    self.show_spectral = !self.show_spectral;
                }
                if ui
                    .selectable_label(self.show_waveform, "Waveform")
                    .on_hover_text("Show the waveform (one display stays on)")
                    .clicked()
                    && (!self.show_waveform || self.show_spectral)
                {
                    self.show_waveform = !self.show_waveform;
                }
                ui.label(RichText::new("Display:").color(theme::TEXT_DIM));
            });
        });
    }

    // --- side panel ------------------------------------------------------------------------

    pub(super) fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("files_panel")
            .resizable(true)
            .default_size(220.0)
            .min_size(160.0)
            .show(ui, |ui| self.files_panel(ui));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.side_tab, SideTab::History, "History");
            ui.selectable_value(&mut self.side_tab, SideTab::Effects, "Effects");
        });
        ui.separator();
        match self.side_tab {
            SideTab::History => self.history_panel(ui),
            SideTab::Effects => self.effects_panel(ui),
        }
    }

    fn files_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Files").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.loaded.is_some(), egui::Button::new("Close"))
                    .clicked()
                {
                    self.close_doc(self.active, false, &ctx);
                }
                if ui.button("Open...").clicked() {
                    self.pick_files(&ctx);
                }
            });
        });
        ui.separator();
        let mut switch = None;
        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("files_grid")
                .striped(true)
                .num_columns(4)
                .show(ui, |ui| {
                    for h in ["Name", "Duration", "Rate", "Ch"] {
                        ui.label(RichText::new(h).color(theme::TEXT_DIM));
                    }
                    ui.end_row();
                    for i in 0..self.docs.len() {
                        let Some((name, modified, duration, sr, ch)) = self.doc_info(i) else {
                            continue;
                        };
                        let label = format!("{name}{}", if modified { " *" } else { "" });
                        if ui
                            .selectable_label(i == self.active, label)
                            .on_hover_text("Click to show in the editor")
                            .clicked()
                        {
                            switch = Some(i);
                        }
                        ui.label(format_time(duration, 0.001, true));
                        ui.label(sr.to_string());
                        ui.label(ch.to_string());
                        ui.end_row();
                    }
                });
            if self.docs.is_empty() {
                ui.label(RichText::new("No files open").color(theme::TEXT_DIM));
            }
            if !self.pending_open.is_empty() {
                ui.label(format!(
                    "{} more file(s) waiting to open",
                    self.pending_open.len()
                ));
            }
        });
        if let Some(i) = switch {
            self.switch_to(i, &ctx);
        }
    }

    fn history_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.history.entries.is_empty() {
            ui.label(RichText::new("No history").color(theme::TEXT_DIM));
            return;
        }
        let mut goto = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, (label, _)) in self.history.entries.iter().enumerate() {
                let text = if i > self.history.pos {
                    RichText::new(label).color(theme::TEXT_DIM).italics()
                } else {
                    RichText::new(label)
                };
                if ui
                    .selectable_label(i == self.history.pos, text)
                    .on_hover_text("Click to return to this state")
                    .clicked()
                {
                    goto = Some(i);
                }
            }
        });
        if let Some(i) = goto
            && !self.busy()
        {
            self.goto_history(i, &ctx);
        }
    }

    fn effects_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let target = match (self.time_range(), self.loaded.as_ref()) {
            (Some((a, b)), _) => format!("Selection ({})", format_time(b - a, 0.001, false)),
            (None, Some(_)) => "Entire file".into(),
            (None, None) => "No file".into(),
        };
        ui.label(RichText::new(format!("Applies to: {target}")).color(theme::TEXT_DIM));
        let channels = match (self.channel, &self.loaded) {
            (Some(c), Some(l)) => channel_name(Some(c), l.audio.channels.len()),
            _ => "all channels".into(),
        };
        ui.label(RichText::new(format!("Channels: {channels}")).color(theme::TEXT_DIM));
        ui.separator();
        let enabled = self.loaded.is_some() && !self.busy();
        ui.add_enabled_ui(enabled, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                self.effect_items(ui, &ctx)
            });
        });
    }

    // --- transport row ---------------------------------------------------------------------

    pub(super) fn transport_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let has_audio = self.loaded.is_some();
        let position = self.player.position();
        let playing = self.player.is_playing();
        let paused = self.player.is_paused();
        ui.horizontal(|ui| {
            let cti = position.unwrap_or(self.playhead);
            ui.label(
                RichText::new(format_time(cti, 0.001, true))
                    .font(FontId::monospace(26.0))
                    .color(theme::TIMECODE),
            );
            ui.add_space(12.0);

            if icon_button(ui, Icon::Stop, "Stop (Space)", position.is_some(), false).clicked() {
                self.stop_playback();
            }
            let play_hint = if self.has_selection() {
                "Play the selection (Space)"
            } else {
                "Play from the current time (Space)"
            };
            if icon_button(ui, Icon::Play, play_hint, has_audio, playing).clicked() && !playing {
                self.play();
            }
            if icon_button(ui, Icon::Pause, "Pause", position.is_some(), paused).clicked() {
                self.toggle_pause();
            }
            if icon_button(
                ui,
                Icon::ToStart,
                "Move playhead to previous (Home)",
                has_audio,
                false,
            )
            .clicked()
            {
                let t = self.playhead_stop(false);
                self.seek(t);
            }
            let step = self.view.t_span() * 0.1;
            if icon_button(ui, Icon::Rewind, "Rewind", has_audio, false).clicked() {
                let t = position.unwrap_or(self.playhead) - step;
                self.seek(t);
            }
            if icon_button(ui, Icon::FastForward, "Fast forward", has_audio, false).clicked() {
                let t = position.unwrap_or(self.playhead) + step;
                self.seek(t);
            }
            if icon_button(
                ui,
                Icon::ToEnd,
                "Move playhead to next (End)",
                has_audio,
                false,
            )
            .clicked()
            {
                let t = self.playhead_stop(true);
                self.seek(t);
            }
            if icon_button(ui, Icon::Loop, "Loop playback", true, self.loop_playback).clicked() {
                self.loop_playback = !self.loop_playback;
            }

            ui.add_space(16.0);
            let zooms = [
                (Icon::ZoomInAmp, "Zoom in amplitude"),
                (Icon::ZoomOutAmp, "Zoom out amplitude"),
                (Icon::ZoomInTime, "Zoom in time (=)"),
                (Icon::ZoomOutTime, "Zoom out time (-)"),
                (Icon::ZoomFull, "Zoom out full (F)"),
                (Icon::ZoomSelection, "Zoom to selection"),
            ];
            for (icon, hint) in zooms {
                if icon_button(ui, icon, hint, has_audio, false).clicked() {
                    match icon {
                        Icon::ZoomInAmp => self.zoom_amp(1.5),
                        Icon::ZoomOutAmp => self.zoom_amp(1.0 / 1.5),
                        Icon::ZoomInTime => self.zoom_time_center(1.5),
                        Icon::ZoomOutTime => self.zoom_time_center(1.0 / 1.5),
                        Icon::ZoomFull => self.zoom_full(),
                        _ => self.zoom_to_selection(),
                    }
                }
            }
            ui.add_space(12.0);
            ui.checkbox(&mut self.follow_playback, "Follow")
                .on_hover_text("Scroll the view to keep the playhead visible");
            let _ = &ctx;
        });
    }

    // --- bottom panels ---------------------------------------------------------------------

    pub(super) fn bottom_panels(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let sel_width = if self.show_selection_view { 360.0 } else { 0.0 };
            if self.show_levels {
                let width = (ui.available_width() - sel_width - 12.0).max(200.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Levels").strong());
                    self.levels_meter(ui, width);
                });
            }
            if self.show_selection_view {
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(RichText::new("Selection/View").strong());
                    self.selection_view(ui);
                });
            }
        });
    }

    fn levels_meter(&self, ui: &mut egui::Ui, width: f32) {
        let channels = self.meters.len().max(2);
        let bar_h = 9.0;
        let height = channels as f32 * (bar_h + 3.0) + 16.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let p = ui.painter_at(rect);
        const MIN_DB: f32 = -60.0;
        let x_of = |db: f32| rect.left() + ((db - MIN_DB) / -MIN_DB).clamp(0.0, 1.0) * rect.width();
        for c in 0..channels {
            let y = rect.top() + c as f32 * (bar_h + 3.0);
            let bar = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), bar_h));
            p.rect_filled(bar, 0.0, Color32::from_gray(18));
            let meter = self.meters.get(c);
            if let Some(m) = meter.filter(|m| m.level.is_finite()) {
                // Green up to -18 dB, yellow up to -6 dB, red above.
                for (lo, hi, color) in [
                    (MIN_DB, -18.0, Color32::from_rgb(40, 200, 80)),
                    (-18.0, -6.0, Color32::from_rgb(230, 210, 40)),
                    (-6.0, 0.0, Color32::from_rgb(235, 60, 50)),
                ] {
                    let top = m.level.min(hi);
                    if top > lo {
                        p.rect_filled(
                            Rect::from_x_y_ranges(x_of(lo)..=x_of(top), bar.y_range()),
                            0.0,
                            color,
                        );
                    }
                }
                if m.peak.is_finite() {
                    p.vline(
                        x_of(m.peak),
                        bar.y_range(),
                        Stroke::new(2.0, Color32::WHITE),
                    );
                }
            }
        }
        let scale_y = rect.top() + channels as f32 * (bar_h + 3.0) + 1.0;
        let step = if rect.width() > 600.0 { 3.0 } else { 6.0 };
        let mut db = MIN_DB + step;
        while db <= 0.0 {
            let x = x_of(db);
            p.vline(
                x,
                scale_y..=scale_y + 3.0,
                Stroke::new(1.0, theme::TEXT_DIM),
            );
            // The last label is right-aligned so it stays inside the meter.
            let anchor = if db >= 0.0 {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            };
            p.text(
                pos2(x, scale_y + 3.0),
                anchor,
                format!("{db:.0}"),
                FontId::proportional(10.0),
                theme::TEXT_DIM,
            );
            db += step;
        }
        p.text(
            pos2(rect.left(), scale_y + 3.0),
            Align2::LEFT_TOP,
            "dB",
            FontId::proportional(10.0),
            theme::TEXT_DIM,
        );
    }

    /// Start/End/Duration of the selection and the view; the values can be typed or dragged.
    fn selection_view(&mut self, ui: &mut egui::Ui) {
        let duration = self.duration();
        let max = duration.unwrap_or(0.0);
        let time_field = |ui: &mut egui::Ui, value: &mut f64, speed: f64, enabled: bool| {
            ui.add_enabled(
                enabled,
                egui::DragValue::new(value)
                    .range(0.0..=max)
                    .speed(speed)
                    .custom_formatter(|v, _| format_time(v, 0.001, true))
                    .custom_parser(parse_time),
            )
            .changed()
        };
        let enabled = duration.is_some();
        let speed = self.view.t_span() / 400.0;
        egui::Grid::new("selection_view")
            .num_columns(4)
            .spacing(vec2(10.0, 4.0))
            .show(ui, |ui| {
                ui.label("");
                for h in ["Start", "End", "Duration"] {
                    ui.label(RichText::new(h).color(theme::TEXT_DIM));
                }
                ui.end_row();

                ui.label("Selection");
                let (mut a, mut b) = self.selection.unwrap_or((self.playhead, self.playhead));
                let mut d = b - a;
                let changed_a = time_field(ui, &mut a, speed, enabled);
                let changed_b = time_field(ui, &mut b, speed, enabled);
                let changed_d = time_field(ui, &mut d, speed, enabled);
                if changed_d {
                    b = (a + d).min(max);
                }
                if changed_a || changed_b || changed_d {
                    let (a, b) = (a.min(b), a.max(b));
                    if b - a > 1e-6 {
                        self.selection = Some((a, b));
                    } else {
                        self.selection = None;
                    }
                    self.playhead = a;
                }
                ui.end_row();

                ui.label("View");
                let (mut a, mut b) = (self.view.t0, self.view.t1);
                let mut d = b - a;
                let changed_a = time_field(ui, &mut a, speed, enabled);
                let changed_b = time_field(ui, &mut b, speed, enabled);
                let changed_d = time_field(ui, &mut d, speed, enabled);
                // Like End, Start moves only its own edge of the view.
                if changed_a && a < self.view.t1 {
                    self.view.t0 = a;
                }
                if changed_b && b > self.view.t0 {
                    self.view.t1 = b;
                }
                if changed_d && d > 0.0 {
                    self.view.t1 = self.view.t0 + d;
                }
                ui.end_row();
            });
    }

    pub(super) fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let state = if let Some(job) = &self.job {
                let stage = match job.stage.load(std::sync::atomic::Ordering::Relaxed) {
                    0 => "Opening",
                    2 => "Processing",
                    _ => "Analysing",
                };
                let pct = job.progress.load(std::sync::atomic::Ordering::Relaxed) as f32 / 10.0;
                format!("{stage}... {pct:.0}%")
            } else if self.player.is_paused() {
                "Paused".into()
            } else if self.player.position().is_some() {
                "Playing".into()
            } else {
                "Stopped".into()
            };
            if self.job.is_some() {
                ui.spinner();
            }
            ui.label(RichText::new(state).color(Color32::WHITE));
            if let Some(err) = &self.error {
                ui.separator();
                ui.colored_label(Color32::from_rgb(255, 130, 130), err);
            }
            if let Some(notice) = &self.notice {
                ui.separator();
                ui.colored_label(Color32::from_rgb(170, 235, 170), notice);
            }
            if let Some(info) = &self.hover_info {
                ui.separator();
                ui.monospace(info);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(l) = &self.loaded {
                    let a = &l.audio;
                    let layout = match a.channels.len() {
                        1 => "Mono".to_string(),
                        2 => "Stereo".to_string(),
                        n => format!("{n} channels"),
                    };
                    let bytes = a.num_frames() as u64 * a.channels.len() as u64 * 4;
                    ui.label(format_time(a.duration(), 0.001, true));
                    ui.separator();
                    ui.label(format_bytes(bytes));
                    ui.separator();
                    ui.label(format!("{} Hz • 32-bit (float) • {layout}", a.sample_rate));
                    if let Some(clip) = &self.clipboard {
                        ui.separator();
                        ui.label(format!("Clipboard {:.3} s", clip.duration()));
                    }
                }
            });
        });
    }

    // --- dialogs ---------------------------------------------------------------------------

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.save_dialog_window(ctx);
        self.settings_window(ctx);
        self.effect_dialog_window(ctx);
        self.save_prompt_window(ctx);

        egui::Window::new("Keyboard Shortcuts")
            .open(&mut self.shortcuts_window)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(SHORTCUTS).monospace());
            });
        egui::Window::new("About Audio Paint")
            .open(&mut self.about_window)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.heading(crate::APP_TITLE);
                ui.label("Spectral audio viewer and editor in Rust.");
                ui.label("Decoding: Symphonia · FFT: RustFFT · Playback: rodio · MP3: LAME");
                ui.label("Copyright (c) 2026 Maddin · MIT License");
            });
    }

    fn save_dialog_window(&mut self, ctx: &egui::Context) {
        if !self.save_dialog {
            return;
        }
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Save As")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let mp3_kbps = match self.save_format {
                    SaveFormat::Mp3(k) => k,
                    _ => 320,
                };
                for format in [
                    SaveFormat::Wav16,
                    SaveFormat::Wav24,
                    SaveFormat::WavFloat,
                    SaveFormat::Mp3(mp3_kbps),
                ] {
                    let label = match format {
                        SaveFormat::Mp3(_) => "MP3".to_string(),
                        f => f.name(),
                    };
                    let selected = std::mem::discriminant(&self.save_format)
                        == std::mem::discriminant(&format);
                    if ui.radio(selected, label).clicked() {
                        self.save_format = format;
                    }
                }
                if let SaveFormat::Mp3(kbps) = &mut self.save_format {
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label("Bit rate");
                        egui::ComboBox::from_id_salt("mp3 bitrate")
                            .selected_text(format!("{kbps} kbit/s"))
                            .show_ui(ui, |ui| {
                                for k in SaveFormat::MP3_BITRATES {
                                    ui.selectable_value(kbps, k, format!("{k} kbit/s"));
                                }
                            });
                    });
                }
                ui.weak(match self.save_format {
                    SaveFormat::WavFloat => "Lossless; keeps levels above 0 dBFS.",
                    SaveFormat::Mp3(_) => {
                        "Lossy, small files. Sample rates above 48 kHz become 48 kHz."
                    }
                    _ => "Most compatible. Samples above 0 dBFS are clipped.",
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Save...").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            // Cancelling also cancels a close or exit that was waiting for this save.
            self.save_dialog = false;
            self.cancel_after_save();
        }
        if save {
            self.save_as(ctx);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_window;
        egui::Window::new("Spectral Display Settings")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("spectral_settings")
                    .num_columns(2)
                    .spacing(vec2(12.0, 6.0))
                    .show(ui, |ui| {
                        ui.label("FFT size");
                        egui::ComboBox::from_id_salt("fft_size")
                            .selected_text(self.fft_size.to_string())
                            .show_ui(ui, |ui| {
                                for size in FFT_SIZES {
                                    ui.selectable_value(&mut self.fft_size, size, size.to_string());
                                }
                            });
                        ui.end_row();

                        ui.label("Window");
                        egui::ComboBox::from_id_salt("window_kind")
                            .selected_text(self.window.name())
                            .show_ui(ui, |ui| {
                                for w in WindowKind::ALL {
                                    ui.selectable_value(&mut self.window, w, w.name());
                                }
                            });
                        ui.end_row();

                        let channels = self.loaded.as_ref().map_or(1, |l| l.audio.channels.len());
                        ui.label("Channel");
                        egui::ComboBox::from_id_salt("spectral_channel")
                            .selected_text(channel_name(self.channel, channels))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.channel, None, "Mix");
                                for c in 0..channels {
                                    let name = channel_name(Some(c), channels);
                                    ui.selectable_value(&mut self.channel, Some(c), name);
                                }
                            });
                        ui.end_row();

                        ui.label("Frequency scale");
                        ui.horizontal(|ui| {
                            let was_log = self.log_scale;
                            ui.selectable_value(&mut self.log_scale, true, "Logarithmic");
                            ui.selectable_value(&mut self.log_scale, false, "Linear");
                            if was_log != self.log_scale {
                                self.view.u0 = 0.0;
                                self.view.u1 = 1.0;
                            }
                        });
                        ui.end_row();

                        ui.label("Colors");
                        egui::ComboBox::from_id_salt("colormap")
                            .selected_text(self.colormap.name())
                            .show_ui(ui, |ui| {
                                for c in Colormap::ALL {
                                    ui.selectable_value(&mut self.colormap, c, c.name());
                                }
                            });
                        ui.end_row();

                        ui.label("Floor");
                        ui.add(
                            egui::Slider::new(&mut self.floor_db, -150.0..=-20.0)
                                .suffix(" dB")
                                .integer(),
                        );
                        ui.end_row();
                        ui.label("Ceiling");
                        ui.add(
                            egui::Slider::new(&mut self.ceil_db, -60.0..=5.0)
                                .suffix(" dB")
                                .integer(),
                        );
                        ui.end_row();
                    });
                if self.ceil_db < self.floor_db + 10.0 {
                    self.ceil_db = self.floor_db + 10.0;
                }
                ui.separator();
                // Colour bar for the chosen range.
                let (rect, _) = ui.allocate_exact_size(vec2(260.0, 12.0), Sense::hover());
                let steps = 64;
                for i in 0..steps {
                    let t = i as f32 / (steps - 1) as f32;
                    let x0 = rect.left() + i as f32 / steps as f32 * rect.width();
                    let x1 = rect.left() + (i + 1) as f32 / steps as f32 * rect.width();
                    ui.painter().rect_filled(
                        Rect::from_x_y_ranges(x0..=x1 + 0.5, rect.y_range()),
                        0.0,
                        self.colormap.color(t),
                    );
                }
                ui.horizontal(|ui| {
                    ui.weak(format!("{:.0} dB", self.floor_db));
                    ui.add_space(190.0);
                    ui.weak(format!("{:.0} dB", self.ceil_db));
                });
            });
        self.settings_window = open;
    }

    fn effect_dialog_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.effect_dialog else {
            return;
        };
        let target = match self.time_range() {
            Some((a, b)) => format!("the selection ({})", format_time(b - a, 0.001, false)),
            None => "the entire file".into(),
        };
        let (title, mut value, range, text) = match dialog {
            EffectDialog::Amplify(db) => ("Amplify", db, -40.0..=40.0, "Gain"),
            EffectDialog::Normalize(db) => ("Normalize", db, -40.0..=0.0, "Peak level"),
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("Applies to {target}."));
                ui.horizontal(|ui| {
                    ui.label(text);
                    ui.add(
                        egui::Slider::new(&mut value, range)
                            .suffix(" dB")
                            .fixed_decimals(1),
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        self.effect_dialog = match dialog {
            EffectDialog::Amplify(_) => Some(EffectDialog::Amplify(value)),
            EffectDialog::Normalize(_) => Some(EffectDialog::Normalize(value)),
        };
        if apply {
            let effect = match dialog {
                EffectDialog::Amplify(_) => {
                    self.gain_db = value;
                    Effect::Amplify(value)
                }
                EffectDialog::Normalize(_) => Effect::Normalize(value),
            };
            self.apply_effect(ctx, effect);
        }
        if apply || cancel || !open {
            self.effect_dialog = None;
        }
    }

    /// "Save changes to ... before closing/exiting?" with Save, Don't Save and Cancel.
    fn save_prompt_window(&mut self, ctx: &egui::Context) {
        let Some((index, why)) = self.save_prompt else {
            return;
        };
        let name = self.doc_info(index).map_or_else(String::new, |i| i.0);
        let (title, question) = match why {
            AfterSave::CloseFile => ("Close File", "before closing"),
            AfterSave::Exit => ("Exit Audio Paint", "before exiting"),
        };
        // Saving another file needs the editor to switch to it, which waits for jobs.
        let can_save = (index == self.active || !self.busy()) && self.save_job.is_none();
        let mut answer = None;
        let mut open = true;
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("Save changes to \"{name}\" {question}?"));
                ui.label(
                    RichText::new("Your changes will be lost if you don't save them.")
                        .color(theme::TEXT_DIM),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save As..."))
                        .clicked()
                    {
                        answer = Some(Some(true));
                    }
                    if ui.button("Don't Save").clicked() {
                        answer = Some(Some(false));
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(None);
                    }
                });
            });
        if !open {
            answer = Some(None);
        }
        if let Some(answer) = answer {
            self.answer_save_prompt(answer, ctx);
        }
    }
}
