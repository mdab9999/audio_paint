//! The Editor panel: navigator, per-channel waveform lanes with dB rulers, the spectral
//! frequency display with its Hz ruler, the time ruler, and all mouse interaction.

use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, FontId, PointerButton, Pos2, Rect, Response,
    Sense, Stroke, StrokeKind, TextureOptions, pos2, vec2,
};

use super::util::{format_time, freq_label, nice_step, nice_time_step};
use super::{AudioPaintApp, EDGE_GRAB, Tool, View, theme};
use crate::edit::EditOp;
use crate::selection::{Grid, Selection, Shape, Stamp, rasterize};
use crate::spectrum::{self, Peaks, RenderParams, Spectrogram, freq_to_u, u_to_freq};

const RULER_WIDTH: f32 = 54.0;
const GUTTER: f32 = 22.0;
const TIME_RULER_HEIGHT: f32 = 22.0;
const NAVIGATOR_HEIGHT: f32 = 34.0;
const LANE_GAP: f32 = 2.0;
const SPECTRAL_SELECTION: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
const SELECTION_EDGE: Color32 = Color32::from_rgba_premultiplied(170, 170, 170, 170);
const SPECTRAL_TINT: Color32 = Color32::from_rgb(150, 190, 255);
const SPECTRAL_OUTLINE: Color32 = Color32::from_rgb(235, 242, 255);

/// Maps the on-screen spectrogram image to times and frequencies.
struct ScreenGrid {
    w: usize,
    h: usize,
    view: View,
    nyq: f64,
    log: bool,
}

impl Grid for ScreenGrid {
    fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn t_of_x(&self, x: f64) -> f64 {
        self.view.t0 + x / self.w as f64 * self.view.t_span()
    }
    fn x_of_t(&self, t: f64) -> f64 {
        (t - self.view.t0) / self.view.t_span() * self.w as f64
    }
    fn f_of_y(&self, y: f64) -> f64 {
        u_to_freq(
            self.view.u1 - y / self.h as f64 * self.view.u_span(),
            self.nyq,
            self.log,
        )
    }
    fn y_of_f(&self, f: f64) -> f64 {
        (self.view.u1 - freq_to_u(f, self.nyq, self.log)) / self.view.u_span() * self.h as f64
    }
}

impl AudioPaintApp {
    /// The central Editor panel: file header, editor area and the transport row.
    pub(super) fn editor_panel(&mut self, ui: &mut egui::Ui) {
        self.editor_header(ui);
        egui::Panel::bottom("transport_row").show(ui, |ui| self.transport_row(ui));
        self.main_view(ui);
    }

    /// "Editor: <file>" with a menu of the open files, like Audition's editor tab.
    fn editor_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Editor:").color(theme::TEXT_DIM));
            let current = self
                .doc_info(self.active)
                .map(|(name, modified, ..)| format!("{name}{}", if modified { " *" } else { "" }));
            let mut switch = None;
            egui::ComboBox::from_id_salt("editor_file")
                .width(320.0)
                .selected_text(current.clone().unwrap_or_else(|| "(no file)".into()))
                .show_ui(ui, |ui| {
                    for i in 0..self.docs.len() {
                        if let Some((name, modified, ..)) = self.doc_info(i) {
                            let label = format!("{name}{}", if modified { " *" } else { "" });
                            if ui.selectable_label(i == self.active, label).clicked() {
                                switch = Some(i);
                            }
                        }
                    }
                });
            if let Some(i) = switch {
                self.switch_to(i, ui.ctx());
            }
            if current.is_some()
                && ui
                    .small_button("✕")
                    .on_hover_text("Close this file (Ctrl+W)")
                    .clicked()
            {
                self.close_doc(self.active, false, ui.ctx());
            }
        });
    }

    fn main_view(&mut self, ui: &mut egui::Ui) {
        let Some(loaded) = &self.loaded else {
            let text = if self.job.is_some() {
                "Loading..."
            } else {
                "Open an audio file with File > Open (Ctrl+O) or drop it onto this window"
            };
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new(text).size(16.0).color(theme::TEXT_DIM))
            });
            return;
        };
        let spec = loaded.spec.clone();
        let peaks = loaded.peaks.clone();
        let duration = loaded.audio.duration();
        let sample_rate = loaded.audio.sample_rate as f64;
        let nyq = spec.nyquist();

        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, Sense::hover());
        let (left, right) = (full.left() + GUTTER, full.right() - RULER_WIDTH);
        if right - left < 80.0 || full.height() < 160.0 {
            return;
        }
        let navigator = Rect::from_min_max(
            pos2(left, full.top() + 2.0),
            pos2(right, full.top() + 2.0 + NAVIGATOR_HEIGHT),
        );
        let top = navigator.bottom() + 6.0;
        let bottom = full.bottom() - TIME_RULER_HEIGHT;
        let show_spectral = self.show_spectral || !self.show_waveform;
        let (wave_area, spec_rect) = match (self.show_waveform, show_spectral) {
            (true, true) => {
                let split = top + (bottom - top) * 0.42;
                (
                    Some(Rect::from_min_max(pos2(left, top), pos2(right, split))),
                    Some(Rect::from_min_max(
                        pos2(left, split + 4.0),
                        pos2(right, bottom),
                    )),
                )
            }
            (true, false) => (
                Some(Rect::from_min_max(pos2(left, top), pos2(right, bottom))),
                None,
            ),
            _ => (
                None,
                Some(Rect::from_min_max(pos2(left, top), pos2(right, bottom))),
            ),
        };
        // One lane per channel inside the waveform area.
        let lanes: Vec<Rect> = wave_area.map_or_else(Vec::new, |area| {
            let n = peaks.len().max(1) as f32;
            let h = (area.height() - LANE_GAP * (n - 1.0)) / n;
            (0..peaks.len())
                .map(|c| {
                    let y = area.top() + c as f32 * (h + LANE_GAP);
                    Rect::from_min_max(pos2(area.left(), y), pos2(area.right(), y + h))
                })
                .collect()
        });

        // --- interaction -------------------------------------------------------------------
        let nav_resp = ui.interact(
            navigator,
            ui.scope_id().with("navigator"),
            Sense::click_and_drag(),
        );
        let wave_resp =
            wave_area.map(|r| ui.interact(r, ui.scope_id().with("wave"), Sense::click_and_drag()));
        let spec_resp =
            spec_rect.map(|r| ui.interact(r, ui.scope_id().with("spec"), Sense::click_and_drag()));

        self.hover_info = None;
        if let (Some(resp), Some(rect)) = (&spec_resp, spec_rect) {
            self.handle_lane(ui, resp, rect, true, duration, nyq, &spec);
        }
        if let (Some(resp), Some(rect)) = (&wave_resp, wave_area) {
            self.handle_lane(ui, resp, rect, false, duration, nyq, &spec);
        }
        self.handle_navigator(ui, &nav_resp, navigator, duration);

        let play_pos = self.player.position();
        if let Some(p) = play_pos
            && self.follow_playback
            && (p > self.view.t1 || p < self.view.t0)
        {
            // Page forward like Audition: the play cursor restarts at the left edge.
            let span = self.view.t_span();
            self.view.t0 = p;
            self.view.t1 = p + span;
        }
        self.view.clamp(duration, 64.0 / sample_rate);

        // --- drawing -----------------------------------------------------------------------
        let painter = ui.painter_at(full);
        let v = self.view;
        let selection = self.selection;
        for (c, (lane, p)) in lanes.iter().zip(&peaks).enumerate() {
            draw_wave_lane(&painter, *lane, v, p, sample_rate, selection);
            draw_db_ruler(&painter, *lane, v.amp);
            let label = match (peaks.len(), c) {
                (1, _) => "M".to_string(),
                (2, 0) => "L".to_string(),
                (2, _) => "R".to_string(),
                _ => (c + 1).to_string(),
            };
            painter.text(
                pos2(full.left() + GUTTER / 2.0, lane.center().y),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.0),
                theme::TEXT_DIM,
            );
        }
        if let Some(rect) = spec_rect {
            self.draw_spectrogram(ui, &painter, rect, &spec);
            self.draw_freq_ruler(&painter, rect, nyq);
        }
        let ruler = Rect::from_min_max(pos2(left, bottom), pos2(right, full.bottom()));
        draw_time_ruler(&painter, ruler, v, duration);
        draw_navigator(&painter, navigator, v, duration, &peaks);

        // Selection on the spectral display and the navigator (the lanes draw their own).
        let t_to_x = |r: Rect, t: f64| r.left() + ((t - v.t0) / v.t_span()) as f32 * r.width();
        let nav_x = |t: f64| navigator.left() + (t / duration) as f32 * navigator.width();
        if let Some((a, b)) = selection {
            if let Some(r) = spec_rect {
                let (x0, x1) = (t_to_x(r, a).max(r.left()), t_to_x(r, b).min(r.right()));
                if x1 > x0 {
                    painter.rect_filled(
                        Rect::from_x_y_ranges(x0..=x1, r.y_range()),
                        0.0,
                        SPECTRAL_SELECTION,
                    );
                }
                for x in [t_to_x(r, a), t_to_x(r, b)] {
                    if r.x_range().contains(x) {
                        painter.vline(x, r.y_range(), Stroke::new(1.0, SELECTION_EDGE));
                    }
                }
            }
            let nav =
                Rect::from_x_y_ranges(nav_x(a)..=nav_x(b).max(nav_x(a) + 1.0), navigator.y_range());
            painter.rect_filled(nav, 0.0, SPECTRAL_SELECTION);
        }

        // Current-time indicator across all lanes, with a handle on the time ruler.
        let cti = play_pos.unwrap_or(self.playhead);
        let x = t_to_x(ruler, cti);
        if ruler.x_range().contains(x) {
            let stroke = Stroke::new(1.0, theme::PLAYHEAD);
            painter.vline(x, top..=bottom, stroke);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(x - 5.0, ruler.top()),
                    pos2(x + 5.0, ruler.top()),
                    pos2(x, ruler.top() + 7.0),
                ],
                theme::PLAYHEAD,
                Stroke::NONE,
            ));
        }
        painter.vline(
            nav_x(cti),
            navigator.y_range(),
            Stroke::new(1.0, theme::PLAYHEAD),
        );

        // Hover line across the lanes.
        let hover = [spec_resp.as_ref(), wave_resp.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|r| r.hover_pos());
        if let Some(pos) = hover {
            painter.vline(
                pos.x,
                top..=bottom,
                Stroke::new(1.0, Color32::from_white_alpha(70)),
            );
        }

        // Zoom box preview.
        if let (Some((start, in_spec)), Some(pos)) =
            (self.zoom_box, ui.input(|i| i.pointer.latest_pos()))
        {
            let lane = if in_spec { spec_rect } else { wave_area }.unwrap_or(full);
            let mut r = Rect::from_two_pos(start, pos).intersect(lane);
            if !in_spec {
                r = Rect::from_x_y_ranges(r.x_range(), lane.y_range());
            }
            painter.rect_filled(r, 0.0, Color32::from_white_alpha(25));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
        }

        // Brush outline at the pointer.
        if self.tool.uses_brush()
            && let (Some(resp), Some(rect)) = (&spec_resp, spec_rect)
            && let Some(pos) = resp.hover_pos()
        {
            let color = if self.tool == Tool::Heal {
                Color32::from_rgb(255, 200, 120)
            } else {
                Color32::WHITE
            };
            painter.with_clip_rect(rect).circle_stroke(
                pos,
                self.brush_radius,
                Stroke::new(1.0, color),
            );
        }

        self.gain_panel(ui, spec_rect.or(wave_area).unwrap_or(full), wave_area, nyq);
    }

    /// Floating panel next to the selection: gain, mute and heal (like Audition's HUD).
    fn gain_panel(&mut self, ui: &egui::Ui, area: Rect, wave_area: Option<Rect>, nyq: f64) {
        if self.drawing.is_some() || self.job.as_ref().is_some_and(|j| j.is_load) {
            return;
        }
        let v = self.view;
        let t_to_x = |t: f64| area.left() + ((t - v.t0) / v.t_span()) as f32 * area.width();
        let f_to_y = |f: f64| {
            let u = freq_to_u(f.max(1e-3), nyq, self.log_scale);
            area.bottom() - ((u - v.u0) / v.u_span()) as f32 * area.height()
        };
        // Anchor at the top-right corner of the selection, kept inside the editor.
        let anchor = if let Some((_, t1, _, f1)) = self.spectral.bounds() {
            pos2(t_to_x(t1), f_to_y(f1))
        } else if let Some((_, b)) = self.selection {
            pos2(t_to_x(b), wave_area.unwrap_or(area).top())
        } else {
            return;
        };
        let pos = pos2(
            (anchor.x + 8.0).clamp(area.left(), (area.right() - 300.0).max(area.left())),
            anchor
                .y
                .clamp(area.top(), (area.bottom() - 90.0).max(area.top())),
        );
        let busy = self.job.is_some();
        let ctx = ui.ctx().clone();
        let mut action = None;
        egui::Area::new(egui::Id::unique("gain_panel"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Gain");
                        ui.add(
                            egui::Slider::new(&mut self.gain_db, -40.0..=20.0)
                                .suffix(" dB")
                                .fixed_decimals(1),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            if ui.button("Apply gain").clicked() {
                                action = Some(EditOp::Gain(10f32.powf(self.gain_db / 20.0)));
                            }
                            if ui.button("Mute (Del)").clicked() {
                                action = Some(EditOp::Gain(0.0));
                            }
                            if ui
                                .button("Heal")
                                .on_hover_text("Rebuild the area from the surrounding audio")
                                .clicked()
                            {
                                action = Some(EditOp::Heal);
                            }
                        });
                    });
                });
            });
        if let Some(op) = action {
            self.edit_selection(&ctx, op);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_lane(
        &mut self,
        ui: &egui::Ui,
        resp: &Response,
        rect: Rect,
        is_spec: bool,
        duration: f64,
        nyq: f64,
        spec: &Spectrogram,
    ) {
        let v = self.view;
        let x_to_t = |x: f32| v.t0 + ((x - rect.left()) / rect.width()) as f64 * v.t_span();
        let y_to_u = |y: f32| v.u1 - ((y - rect.top()) / rect.height()) as f64 * v.u_span();

        if let Some(pos) = resp.hover_pos() {
            let t = x_to_t(pos.x);
            let long = duration >= 60.0;
            self.hover_info = Some(if is_spec {
                let f = u_to_freq(y_to_u(pos.y), nyq, self.log_scale);
                let level = spec
                    .value_at(t, f)
                    .map_or(String::new(), |db| format!("  {db:6.1} dB"));
                format!(
                    "{}  {:>9}{level}",
                    format_time(t, 0.001, long),
                    format!("{f:.1} Hz"),
                )
            } else {
                format_time(t, 0.0001, long)
            });

            let (scroll, zoom, mods) =
                ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
            // Ctrl+wheel zooms frequency on the spectral display, amplitude on the waveform.
            if zoom != 1.0 {
                if is_spec {
                    self.view.zoom_freq(zoom as f64, y_to_u(pos.y));
                } else {
                    self.zoom_amp(zoom);
                }
            }
            if scroll.y != 0.0 {
                if mods.alt && is_spec {
                    self.view.u0 += scroll.y as f64 / rect.height() as f64 * v.u_span();
                    self.view.u1 += scroll.y as f64 / rect.height() as f64 * v.u_span();
                } else if !mods.alt {
                    self.view.zoom_time((scroll.y as f64 * 0.004).exp(), t);
                }
            }
            if scroll.x != 0.0 {
                let dt = -(scroll.x / rect.width()) as f64 * v.t_span();
                self.view.t0 += dt;
                self.view.t1 += dt;
            }
        }

        // The waveform always selects time; the spectral display follows the active tool.
        let hand = is_spec && self.tool == Tool::Hand;
        if !is_spec || self.tool == Tool::Time {
            self.handle_selection(ui, resp, rect, duration);
        } else if !hand {
            self.handle_spectral_tool(ui, resp, rect, nyq);
        }

        // The hand tool pans with the left button; all lanes pan with the middle button.
        if (hand && resp.dragged_by(PointerButton::Primary))
            || resp.dragged_by(PointerButton::Middle)
        {
            let d = resp.drag_delta();
            let dt = -(d.x / rect.width()) as f64 * v.t_span();
            self.view.t0 += dt;
            self.view.t1 += dt;
            if is_spec {
                let du = (d.y / rect.height()) as f64 * v.u_span();
                self.view.u0 += du;
                self.view.u1 += du;
            }
        }

        if resp.drag_started_by(PointerButton::Secondary)
            && let Some(pos) = resp.interact_pointer_pos()
        {
            self.zoom_box = Some((pos, is_spec));
        }
        if resp.drag_stopped_by(PointerButton::Secondary)
            && let (Some((start, in_spec)), Some(end)) =
                (self.zoom_box.take(), resp.interact_pointer_pos())
            && in_spec == is_spec
        {
            let r = Rect::from_two_pos(start, end).intersect(rect);
            if r.width() > 4.0 {
                self.view.t0 = x_to_t(r.left());
                self.view.t1 = x_to_t(r.right());
            }
            if is_spec && r.height() > 4.0 {
                self.view.u1 = y_to_u(r.top());
                self.view.u0 = y_to_u(r.bottom());
            }
        }

        if hand && resp.double_clicked() {
            self.view = View::full(duration);
        }
    }

    /// Marquee, lasso, paintbrush and spot healing on the spectrum.
    fn handle_spectral_tool(&mut self, ui: &egui::Ui, resp: &Response, rect: Rect, nyq: f64) {
        let v = self.view;
        let log = self.log_scale;
        let u_at = |p: Pos2| v.u1 - ((p.y - rect.top()) / rect.height()) as f64 * v.u_span();
        let tf = |p: Pos2| {
            let t = v.t0 + ((p.x - rect.left()) / rect.width()) as f64 * v.t_span();
            (t, u_to_freq(u_at(p).clamp(0.0, 1.0), nyq, log))
        };
        let radius = self.brush_radius;
        let stamp = |p: Pos2| {
            let (t, f) = tf(p);
            let ru = (radius / rect.height()) as f64 * v.u_span();
            Stamp {
                t,
                f,
                rt: (radius / rect.width()) as f64 * v.t_span(),
                f_lo: u_to_freq(u_at(p) - ru, nyq, log),
                f_hi: u_to_freq(u_at(p) + ru, nyq, log),
            }
        };

        if resp.hovered() {
            ui.ctx().set_cursor_icon(if self.tool.uses_brush() {
                CursorIcon::None
            } else {
                CursorIcon::Crosshair
            });
        }

        let (pressed, released, mods, pointer) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.modifiers,
                i.pointer.latest_pos(),
            )
        });

        if pressed
            && resp.hovered()
            && let Some(p) = pointer
        {
            let add = !mods.alt;
            // The brush accumulates strokes; marquee and lasso replace unless Shift/Alt is held.
            let replace =
                !matches!(self.tool, Tool::Brush | Tool::Heal) && !mods.shift && !mods.alt;
            if replace {
                self.clear_spectral();
            }
            let shape = match self.tool {
                Tool::Marquee => {
                    let (t, f) = tf(p);
                    Shape::rect(t, t, f, f)
                }
                Tool::Lasso => Shape::Lasso(vec![tf(p)]),
                _ => Shape::Brush(vec![stamp(p)]),
            };
            self.drawing = Some((shape, add));
            self.draw_origin = p;
            self.draw_last = p;
            self.overlay_version += 1;
        }

        if let (Some((shape, _)), Some(p)) = (&mut self.drawing, pointer) {
            let p = rect.clamp(p);
            let mut changed = false;
            match shape {
                Shape::Rect { .. } => {
                    let (ta, fa) = tf(self.draw_origin);
                    let (tb, fb) = tf(p);
                    *shape = Shape::rect(ta, tb, fa, fb);
                    changed = p != self.draw_last;
                }
                Shape::Lasso(points) => {
                    if p.distance(self.draw_last) > 2.0 {
                        points.push(tf(p));
                        changed = true;
                    }
                }
                Shape::Brush(stamps) => {
                    let dist = p.distance(self.draw_last);
                    let spacing = (radius / 3.0).max(1.0);
                    if dist >= spacing {
                        let steps = (dist / spacing).ceil() as usize;
                        for s in 1..=steps {
                            stamps.push(stamp(self.draw_last.lerp(p, s as f32 / steps as f32)));
                        }
                        changed = true;
                    }
                }
            }
            if changed {
                self.draw_last = p;
                self.overlay_version += 1;
            }
        }

        if released && let Some((shape, add)) = self.drawing.take() {
            self.overlay_version += 1;
            if self.tool == Tool::Heal {
                let mut region = Selection::default();
                region.push(shape, true);
                if !region.is_empty() {
                    self.run_edit(ui.ctx(), "Spot Healing Brush".into(), EditOp::Heal, region);
                }
            } else {
                self.spectral.push(shape, add);
            }
        }
    }

    /// Time selection: drag selects a time range, dragging near an edge resizes it,
    /// Shift+click extends it, a plain click deselects and moves the playhead,
    /// double-click selects everything.
    fn handle_selection(&mut self, ui: &egui::Ui, resp: &Response, rect: Rect, duration: f64) {
        let v = self.view;
        let x_to_t = |x: f32| {
            (v.t0 + ((x - rect.left()) / rect.width()) as f64 * v.t_span()).clamp(0.0, duration)
        };
        let t_to_x = |t: f64| rect.left() + ((t - v.t0) / v.t_span()) as f32 * rect.width();
        let near_edge = |x: f32| {
            self.selection.and_then(|(a, b)| {
                if (x - t_to_x(a)).abs() <= EDGE_GRAB {
                    Some(b) // grabbing the start keeps the end fixed
                } else if (x - t_to_x(b)).abs() <= EDGE_GRAB {
                    Some(a)
                } else {
                    None
                }
            })
        };

        if let Some(pos) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(if near_edge(pos.x).is_some() {
                CursorIcon::ResizeHorizontal
            } else {
                CursorIcon::Text
            });
        }

        let shift = ui.input(|i| i.modifiers.shift);
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let t = x_to_t(origin.x);
            self.selection_anchor = Some(match (near_edge(origin.x), self.selection) {
                (Some(fixed), _) => fixed,
                // Shift+drag extends from the farther selection edge.
                (None, Some((a, b))) if shift => {
                    if (t - a).abs() > (t - b).abs() {
                        a
                    } else {
                        b
                    }
                }
                _ => t,
            });
        }
        if resp.dragged_by(PointerButton::Primary)
            && let (Some(anchor), Some(pos)) = (self.selection_anchor, resp.interact_pointer_pos())
        {
            let t = x_to_t(pos.x);
            self.selection = Some((anchor.min(t), anchor.max(t)));
            self.playhead = anchor.min(t);
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.selection_anchor = None;
            if let Some((a, b)) = self.selection
                && t_to_x(b) - t_to_x(a) < 2.0
            {
                self.selection = None;
            }
        }

        if resp.clicked_by(PointerButton::Primary)
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let t = x_to_t(pos.x);
            if shift {
                let (a, b) = self.selection.unwrap_or((self.playhead, self.playhead));
                let anchor = if (t - a).abs() > (t - b).abs() { a } else { b };
                self.selection = Some((anchor.min(t), anchor.max(t)));
                self.playhead = anchor.min(t);
            } else {
                self.selection = None;
                if !self.spectral.is_empty() {
                    self.clear_spectral();
                }
                self.playhead = t;
                // Clicking during playback continues from the new position.
                if self.player.is_playing() {
                    self.play_range(t, duration);
                }
            }
        }
        if resp.double_clicked_by(PointerButton::Primary) {
            self.select_all();
        }
    }

    /// Zoom navigator: click or drag to move the view, wheel to zoom, double-click to fit.
    fn handle_navigator(&mut self, ui: &egui::Ui, resp: &Response, rect: Rect, duration: f64) {
        let x_to_t = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * duration;
        if (resp.clicked() || resp.dragged_by(PointerButton::Primary))
            && let Some(pos) = resp.interact_pointer_pos()
        {
            let half = self.view.t_span() / 2.0;
            let t = x_to_t(pos.x);
            self.view.t0 = t - half;
            self.view.t1 = t + half;
        }
        if let Some(pos) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.view
                    .zoom_time((scroll as f64 * 0.004).exp(), x_to_t(pos.x));
            }
        }
        if resp.double_clicked() {
            self.zoom_full();
        }
    }

    fn draw_spectrogram(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        spec: &Arc<Spectrogram>,
    ) {
        let ppp = ui.ctx().pixels_per_point();
        let params = RenderParams {
            t0: self.view.t0,
            t1: self.view.t1,
            u0: self.view.u0,
            u1: self.view.u1,
            width: (rect.width() * ppp).round().max(1.0) as usize,
            height: (rect.height() * ppp).round().max(1.0) as usize,
            log_scale: self.log_scale,
            floor_db: self.floor_db,
            ceil_db: self.ceil_db,
            colormap: self.colormap,
        };
        let key = (params, Arc::as_ptr(spec) as usize);
        if self.rendered.as_ref() != Some(&key) {
            self.base_image = Some(spectrum::render(spec, &key.0));
            self.rendered = Some(key);
        }

        // Composite the spectral selection (and the shape being drawn) over the base image.
        let overlay_key = (self.rendered.clone(), self.overlay_version);
        if self.overlay_key.as_ref() != Some(&overlay_key)
            && let Some(base) = &self.base_image
        {
            let shapes: Vec<&(Shape, bool)> = self
                .spectral
                .shapes
                .iter()
                .chain(self.drawing.as_ref())
                .collect();
            let image = if shapes.is_empty() {
                base.clone()
            } else {
                let grid = ScreenGrid {
                    w: base.size[0],
                    h: base.size[1],
                    view: self.view,
                    nyq: spec.nyquist(),
                    log: self.log_scale,
                };
                composite_selection(base, &rasterize(shapes, &grid))
            };
            match &mut self.texture {
                Some(tex) => tex.set(image, TextureOptions::NEAREST),
                None => {
                    self.texture = Some(ui.ctx().load_texture(
                        "spectrogram",
                        image,
                        TextureOptions::NEAREST,
                    ));
                }
            }
            self.overlay_key = Some(overlay_key);
        }
        if let Some(tex) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
        }
    }

    /// Hz ruler on the right of the spectral display.
    fn draw_freq_ruler(&self, painter: &egui::Painter, rect: Rect, nyq: f64) {
        let v = self.view;
        let ruler = Rect::from_min_size(rect.right_top(), vec2(RULER_WIDTH, rect.height()));
        painter.rect_filled(ruler, 0.0, theme::RULER_BG);
        let u_to_y = |u: f64| rect.bottom() - ((u - v.u0) / v.u_span()) as f32 * rect.height();
        let f_lo = u_to_freq(v.u0, nyq, self.log_scale);
        let f_hi = u_to_freq(v.u1, nyq, self.log_scale);

        let mut ticks: Vec<f64> = Vec::new();
        if self.log_scale {
            for exp in 0..6 {
                for m in [1.0, 2.0, 5.0] {
                    let f = m * 10f64.powi(exp);
                    if f >= f_lo && f <= f_hi {
                        ticks.push(f);
                    }
                }
            }
        }
        if ticks.len() < 3 {
            let target = (rect.height() / 40.0).max(2.0) as usize;
            let step = nice_step(f_hi - f_lo, target);
            let mut f = (f_lo / step).ceil() * step;
            ticks.clear();
            while f <= f_hi {
                ticks.push(f);
                f += step;
            }
        }

        let mut last_y = f32::INFINITY;
        for f in ticks {
            let y = u_to_y(freq_to_u(f, nyq, self.log_scale));
            if (last_y - y).abs() < 16.0 || y < rect.top() + 6.0 || y > rect.bottom() - 8.0 {
                continue;
            }
            last_y = y;
            painter.line_segment(
                [pos2(ruler.left(), y), pos2(ruler.left() + 5.0, y)],
                Stroke::new(1.0, theme::RULER_TEXT),
            );
            painter.hline(rect.x_range(), y, Stroke::new(1.0, theme::GRID));
            painter.text(
                pos2(ruler.left() + 8.0, y),
                Align2::LEFT_CENTER,
                freq_label(f),
                FontId::proportional(11.0),
                theme::RULER_TEXT,
            );
        }
        painter.text(
            pos2(ruler.right() - 4.0, rect.top() + 1.0),
            Align2::RIGHT_TOP,
            "Hz",
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
    }
}

/// Tints the selected pixels and outlines the selection edge (2 px wide).
fn composite_selection(base: &ColorImage, mask: &[f32]) -> ColorImage {
    let [w, h] = base.size;
    let mut out = base.clone();
    let inside = |x: isize, y: isize| {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && mask[y as usize * w + x as usize] > 0.5
    };
    for y in 0..h as isize {
        for x in 0..w as isize {
            if !inside(x, y) {
                continue;
            }
            let edge = [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-2, 0),
                (2, 0),
                (0, -2),
                (0, 2),
            ]
            .iter()
            .any(|(dx, dy)| !inside(x + dx, y + dy));
            let px = &mut out.pixels[y as usize * w + x as usize];
            *px = if edge {
                SPECTRAL_OUTLINE
            } else {
                px.lerp_to_gamma(SPECTRAL_TINT, 0.3)
            };
        }
    }
    out
}

fn draw_time_ruler(painter: &egui::Painter, rect: Rect, v: View, duration: f64) {
    painter.rect_filled(rect, 0.0, theme::RULER_BG);
    let t_to_x = |t: f64| rect.left() + ((t - v.t0) / v.t_span()) as f32 * rect.width();
    let step = nice_time_step(v.t_span(), (rect.width() / 90.0).max(2.0) as usize);
    let long = duration >= 60.0;

    // Minor ticks in between.
    let minor = step / 5.0;
    let mut t = (v.t0 / minor).ceil() * minor;
    while t <= v.t1 {
        let x = t_to_x(t);
        painter.vline(
            x,
            rect.top()..=rect.top() + 4.0,
            Stroke::new(1.0, theme::TEXT_DIM),
        );
        t += minor;
    }
    let mut t = (v.t0 / step).ceil() * step;
    while t <= v.t1 {
        let x = t_to_x(t);
        painter.vline(
            x,
            rect.top()..=rect.top() + 8.0,
            Stroke::new(1.0, theme::RULER_TEXT),
        );
        // Leave room for the label at the right end.
        if x > rect.right() - 40.0 {
            break;
        }
        painter.text(
            pos2(x + 3.0, rect.top() + 8.0),
            Align2::LEFT_TOP,
            format_time(t, step, long),
            FontId::proportional(11.0),
            theme::RULER_TEXT,
        );
        t += step;
    }
    painter.text(
        pos2(rect.right() + 8.0, rect.top() + 8.0),
        Align2::LEFT_TOP,
        "hms",
        FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
}

/// One channel of the waveform. The selected range gets Audition's light background.
fn draw_wave_lane(
    painter: &egui::Painter,
    rect: Rect,
    v: View,
    peaks: &Peaks,
    sample_rate: f64,
    selection: Option<(f64, f64)>,
) {
    painter.rect_filled(rect, 0.0, theme::EDITOR_BG);
    let t_to_x = |t: f64| rect.left() + ((t - v.t0) / v.t_span()) as f32 * rect.width();
    let sel_x = selection.map(|(a, b)| (t_to_x(a).max(rect.left()), t_to_x(b).min(rect.right())));
    if let Some((x0, x1)) = sel_x
        && x1 > x0
    {
        painter.rect_filled(
            Rect::from_x_y_ranges(x0..=x1, rect.y_range()),
            0.0,
            theme::SELECTION_BG,
        );
    }
    let in_sel = |x: f32| sel_x.is_some_and(|(x0, x1)| x >= x0 && x <= x1);
    let color = |x: f32| {
        if in_sel(x) {
            theme::WAVE_SELECTED
        } else {
            theme::WAVE
        }
    };

    let mid = rect.center().y;
    let half = rect.height() / 2.0 * 0.95;
    let amp_to_y = |s: f32| mid - (s * v.amp).clamp(-1.0, 1.0) * half;
    painter.hline(
        rect.x_range(),
        mid,
        Stroke::new(1.0, Color32::from_gray(70)),
    );

    let samples_per_px = v.t_span() * sample_rate / rect.width() as f64;
    if samples_per_px >= 1.0 {
        let mut x = 0.0;
        while x < rect.width() {
            let a = ((v.t0 + x as f64 / rect.width() as f64 * v.t_span()) * sample_rate) as usize;
            let b = (a as f64 + samples_per_px).ceil() as usize;
            if let Some((lo, hi)) = peaks.range(a, b.max(a + 1)) {
                let px = rect.left() + x + 0.5;
                painter.line_segment(
                    [pos2(px, amp_to_y(hi)), pos2(px, amp_to_y(lo) + 1.0)],
                    Stroke::new(1.0, color(px)),
                );
            }
            x += 1.0;
        }
    } else {
        let first = (v.t0 * sample_rate).floor().max(0.0) as usize;
        let last = ((v.t1 * sample_rate).ceil() as usize + 1).min(peaks.samples.len());
        let points: Vec<Pos2> = (first..last)
            .map(|i| {
                let t = i as f64 / sample_rate;
                pos2(t_to_x(t), amp_to_y(peaks.samples[i]))
            })
            .collect();
        let clipped = painter.with_clip_rect(rect);
        for pair in points.windows(2) {
            clipped.line_segment([pair[0], pair[1]], Stroke::new(1.5, color(pair[0].x)));
        }
        if samples_per_px < 0.1 {
            for p in &points {
                clipped.circle_filled(*p, 2.0, color(p.x));
            }
        }
    }
}

/// dB ruler on the right of a waveform lane (mirrored around the centre line).
fn draw_db_ruler(painter: &egui::Painter, lane: Rect, amp: f32) {
    let ruler = Rect::from_min_size(lane.right_top(), vec2(RULER_WIDTH, lane.height()));
    painter.rect_filled(ruler, 0.0, theme::RULER_BG);
    let mid = lane.center().y;
    let half = lane.height() / 2.0 * 0.95;
    let font = FontId::proportional(10.5);
    let mut last_dy = f32::INFINITY;
    for db in [
        0.0f32, -1.0, -2.0, -3.0, -4.5, -6.0, -9.0, -12.0, -15.0, -18.0, -21.0, -24.0, -30.0,
        -36.0, -42.0, -48.0, -54.0, -60.0, -72.0, -84.0, -96.0,
    ] {
        let dy = 10f32.powf(db / 20.0) * amp * half;
        if dy > half + 0.5 || last_dy - dy < 13.0 {
            continue;
        }
        if dy < 9.0 {
            break;
        }
        last_dy = dy;
        for y in [mid - dy, mid + dy] {
            painter.line_segment(
                [pos2(ruler.left(), y), pos2(ruler.left() + 4.0, y)],
                Stroke::new(1.0, theme::RULER_TEXT),
            );
            // Labels at the very edge would collide with the next lane's ruler.
            if y < lane.top() + 6.0 || y > lane.bottom() - 6.0 {
                continue;
            }
            painter.text(
                pos2(ruler.left() + 7.0, y),
                Align2::LEFT_CENTER,
                format!("{db}"),
                font.clone(),
                theme::RULER_TEXT,
            );
        }
    }
    painter.text(
        pos2(ruler.left() + 7.0, mid),
        Align2::LEFT_CENTER,
        "-∞",
        font,
        theme::RULER_TEXT,
    );
    painter.text(
        pos2(ruler.right() - 4.0, lane.top() + 1.0),
        Align2::RIGHT_TOP,
        "dB",
        FontId::proportional(10.5),
        theme::TEXT_DIM,
    );
}

/// Zoom navigator: the whole file (all channels overlaid) with the visible range framed.
fn draw_navigator(
    painter: &egui::Painter,
    rect: Rect,
    v: View,
    duration: f64,
    peaks: &[Arc<Peaks>],
) {
    painter.rect_filled(rect, 2.0, theme::EDITOR_BG);
    let mid = rect.center().y;
    let half = rect.height() / 2.0 * 0.9;
    let mut x = 0.0;
    while x < rect.width() {
        let mut range: Option<(f32, f32)> = None;
        for p in peaks {
            let n = p.samples.len() as f64;
            let a = (x as f64 / rect.width() as f64 * n) as usize;
            let b = ((x + 1.0) as f64 / rect.width() as f64 * n) as usize;
            if let Some((lo, hi)) = p.range(a, b.max(a + 1)) {
                range = Some(range.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
            }
        }
        if let Some((lo, hi)) = range {
            let px = rect.left() + x + 0.5;
            painter.line_segment(
                [
                    pos2(px, mid - hi.clamp(-1.0, 1.0) * half),
                    pos2(px, mid - lo.clamp(-1.0, 1.0) * half + 1.0),
                ],
                Stroke::new(1.0, theme::WAVE.gamma_multiply(0.6)),
            );
        }
        x += 1.0;
    }

    let x0 = rect.left() + (v.t0 / duration) as f32 * rect.width();
    let x1 = rect.left() + (v.t1 / duration) as f32 * rect.width();
    let shade = Color32::from_black_alpha(150);
    painter.rect_filled(
        Rect::from_x_y_ranges(rect.left()..=x0, rect.y_range()),
        0.0,
        shade,
    );
    painter.rect_filled(
        Rect::from_x_y_ranges(x1..=rect.right(), rect.y_range()),
        0.0,
        shade,
    );
    let window = Rect::from_x_y_ranges(x0..=x1.max(x0 + 2.0), rect.y_range());
    painter.rect_stroke(
        window,
        0.0,
        Stroke::new(1.5, Color32::from_gray(220)),
        StrokeKind::Inside,
    );
}
