//! The application window: toolbar, spectral view with zoom/pan, overview and status bar.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, Response, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, pos2, vec2,
};

use crate::audio::{self, AudioData};
use crate::colormap::Colormap;
use crate::spectrum::{
    self, FFT_SIZES, Peaks, RenderParams, Spectrogram, WindowKind, freq_to_u, u_to_freq,
};

const AXIS_LEFT: f32 = 64.0;
const AXIS_RIGHT: f32 = 72.0;
const RULER_HEIGHT: f32 = 22.0;
const OVERVIEW_HEIGHT: f32 = 44.0;
const GAP: f32 = 6.0;
const MIN_U_SPAN: f64 = 0.002;
const WAVE_COLOR: Color32 = Color32::from_rgb(70, 220, 130);
const AXIS_TEXT: Color32 = Color32::from_gray(190);
const AUDIO_EXTENSIONS: [&str; 7] = ["mp3", "wav", "flac", "ogg", "m4a", "aac", "mp4"];

/// Settings that require re-running the FFT when changed.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Analysis {
    fft_size: usize,
    window: WindowKind,
    /// `None` = mono mix of all channels.
    channel: Option<usize>,
}

struct Loaded {
    path: PathBuf,
    audio: Arc<AudioData>,
    spec: Arc<Spectrogram>,
    peaks: Arc<Peaks>,
    analysis: Analysis,
}

enum JobResult {
    Loaded {
        path: PathBuf,
        audio: Arc<AudioData>,
        spec: Arc<Spectrogram>,
        peaks: Arc<Peaks>,
    },
    Analyzed {
        spec: Arc<Spectrogram>,
        peaks: Arc<Peaks>,
    },
    Failed(String),
}

struct Job {
    rx: Receiver<JobResult>,
    /// Per mille of the current stage.
    progress: Arc<AtomicU32>,
    /// 0 = decoding, 1 = analysing.
    stage: Arc<AtomicU32>,
    target: Analysis,
    is_load: bool,
}

/// Visible region: time in seconds, frequency as normalized scale position (0 = bottom).
#[derive(Clone, Copy, PartialEq, Debug)]
struct View {
    t0: f64,
    t1: f64,
    u0: f64,
    u1: f64,
}

impl View {
    fn full(duration: f64) -> Self {
        Self {
            t0: 0.0,
            t1: duration,
            u0: 0.0,
            u1: 1.0,
        }
    }

    fn t_span(&self) -> f64 {
        self.t1 - self.t0
    }

    fn u_span(&self) -> f64 {
        self.u1 - self.u0
    }

    /// `factor > 1` zooms in, keeping time `anchor` fixed on screen.
    fn zoom_time(&mut self, factor: f64, anchor: f64) {
        self.t0 = anchor - (anchor - self.t0) / factor;
        self.t1 = anchor + (self.t1 - anchor) / factor;
    }

    fn zoom_freq(&mut self, factor: f64, anchor: f64) {
        self.u0 = anchor - (anchor - self.u0) / factor;
        self.u1 = anchor + (self.u1 - anchor) / factor;
    }

    fn clamp(&mut self, duration: f64, min_t_span: f64) {
        let span = self.t_span().clamp(min_t_span.min(duration), duration);
        self.t0 = self.t0.clamp(0.0, duration - span);
        self.t1 = self.t0 + span;
        let span = self.u_span().clamp(MIN_U_SPAN, 1.0);
        self.u0 = self.u0.clamp(0.0, 1.0 - span);
        self.u1 = self.u0 + span;
    }
}

pub struct AudioPaintApp {
    loaded: Option<Loaded>,
    job: Option<Job>,
    error: Option<String>,

    fft_size: usize,
    window: WindowKind,
    channel: Option<usize>,
    log_scale: bool,
    floor_db: f32,
    ceil_db: f32,
    colormap: Colormap,
    show_waveform: bool,

    view: View,
    texture: Option<TextureHandle>,
    rendered: Option<(RenderParams, usize)>,
    /// Start of a right-drag zoom box, and whether it started in the spectral lane.
    zoom_box: Option<(Pos2, bool)>,
    hover_info: Option<String>,
}

impl AudioPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = Self {
            loaded: None,
            job: None,
            error: None,
            fft_size: 2048,
            window: WindowKind::BlackmanHarris,
            channel: None,
            log_scale: false,
            floor_db: -110.0,
            ceil_db: 0.0,
            colormap: Colormap::Spectral,
            show_waveform: true,
            view: View::full(1.0),
            texture: None,
            rendered: None,
            zoom_box: None,
            hover_info: None,
        };
        if let Some(path) = initial_file {
            app.open(path, &cc.egui_ctx);
        }
        app
    }

    fn analysis(&self) -> Analysis {
        Analysis {
            fft_size: self.fft_size,
            window: self.window,
            channel: self.channel,
        }
    }

    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.channel = None;
        let target = self.analysis();
        self.spawn_job(ctx, target, true, move |progress, stage| {
            let audio = Arc::new(audio::decode_file(&path, progress)?);
            stage.store(1, Ordering::Relaxed);
            progress.store(0, Ordering::Relaxed);
            let (spec, peaks) = analyze(&audio, target, progress);
            Ok(JobResult::Loaded {
                path,
                audio,
                spec,
                peaks,
            })
        });
    }

    fn reanalyze(&mut self, ctx: &egui::Context) {
        let Some(loaded) = &self.loaded else { return };
        let audio = loaded.audio.clone();
        let target = self.analysis();
        self.spawn_job(ctx, target, false, move |progress, stage| {
            stage.store(1, Ordering::Relaxed);
            let (spec, peaks) = analyze(&audio, target, progress);
            Ok(JobResult::Analyzed { spec, peaks })
        });
    }

    fn spawn_job(
        &mut self,
        ctx: &egui::Context,
        target: Analysis,
        is_load: bool,
        work: impl FnOnce(&AtomicU32, &AtomicU32) -> Result<JobResult, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        let progress = Arc::new(AtomicU32::new(0));
        let stage = Arc::new(AtomicU32::new(0));
        let (p, s, ctx) = (progress.clone(), stage.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = work(&p, &s).unwrap_or_else(JobResult::Failed);
            // The receiver is gone if a newer job replaced this one.
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.error = None;
        // Dropping the previous job's receiver discards its result.
        self.job = Some(Job {
            rx,
            progress,
            stage,
            target,
            is_load,
        });
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let result = match job.rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                return;
            }
        };
        let target = job.target;
        self.job = None;

        match result {
            JobResult::Loaded {
                path,
                audio,
                spec,
                peaks,
            } => {
                self.view = View::full(audio.duration());
                let name = file_name(&path);
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{name} - Audio Paint"
                )));
                self.loaded = Some(Loaded {
                    path,
                    audio,
                    spec,
                    peaks,
                    analysis: target,
                });
            }
            JobResult::Analyzed { spec, peaks } => {
                if let Some(loaded) = &mut self.loaded {
                    loaded.spec = spec;
                    loaded.peaks = peaks;
                    loaded.analysis = target;
                }
            }
            JobResult::Failed(message) => self.error = Some(message),
        }
    }

    /// Starts a new analysis when FFT size, window or channel differ from the displayed one.
    fn check_reanalysis(&mut self, ctx: &egui::Context) {
        let wanted = self.analysis();
        let Some(loaded) = &self.loaded else { return };
        let pending = self.job.as_ref().map(|j| (j.target, j.is_load));
        let up_to_date = match pending {
            Some((_, true)) => true, // wait for the file load to finish first
            Some((target, false)) => target == wanted,
            None => loaded.analysis == wanted,
        };
        if !up_to_date {
            self.reanalyze(ctx);
        }
    }

    fn pick_file(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio files", &AUDIO_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_file();
        if let Some(path) = picked {
            self.open(path, ctx);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open...").clicked() {
                self.pick_file(ui.ctx());
            }
            ui.separator();

            ui.label("FFT size");
            egui::ComboBox::from_id_salt("FFT size")
                .selected_text(self.fft_size.to_string())
                .show_ui(ui, |ui| {
                    for size in FFT_SIZES {
                        ui.selectable_value(&mut self.fft_size, size, size.to_string());
                    }
                });
            ui.label("Window");
            egui::ComboBox::from_id_salt("Window")
                .selected_text(self.window.name())
                .show_ui(ui, |ui| {
                    for w in WindowKind::ALL {
                        ui.selectable_value(&mut self.window, w, w.name());
                    }
                });

            let num_channels = self.loaded.as_ref().map_or(1, |l| l.audio.channels.len());
            if num_channels > 1 {
                ui.label("Channel");
                egui::ComboBox::from_id_salt("Channel")
                    .selected_text(channel_name(self.channel, num_channels))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.channel, None, "Mix");
                        for c in 0..num_channels {
                            let name = channel_name(Some(c), num_channels);
                            ui.selectable_value(&mut self.channel, Some(c), name);
                        }
                    });
            }
            ui.separator();

            let was_log = self.log_scale;
            ui.selectable_value(&mut self.log_scale, false, "Linear");
            ui.selectable_value(&mut self.log_scale, true, "Log");
            if was_log != self.log_scale {
                self.view.u0 = 0.0;
                self.view.u1 = 1.0;
            }
            ui.label("Colors");
            egui::ComboBox::from_id_salt("Colors")
                .selected_text(self.colormap.name())
                .show_ui(ui, |ui| {
                    for c in Colormap::ALL {
                        ui.selectable_value(&mut self.colormap, c, c.name());
                    }
                });
            ui.separator();

            ui.add(
                egui::Slider::new(&mut self.floor_db, -150.0..=-20.0)
                    .text("Floor dB")
                    .integer(),
            );
            ui.add(
                egui::Slider::new(&mut self.ceil_db, -60.0..=5.0)
                    .text("Ceiling dB")
                    .integer(),
            );
            if self.ceil_db < self.floor_db + 10.0 {
                self.ceil_db = self.floor_db + 10.0;
            }
            ui.separator();

            ui.checkbox(&mut self.show_waveform, "Waveform");
            if ui.button("Zoom to fit").clicked()
                && let Some(l) = &self.loaded
            {
                self.view = View::full(l.audio.duration());
            }
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(job) = &self.job {
                ui.spinner();
                let stage = match job.stage.load(Ordering::Relaxed) {
                    0 => "Decoding",
                    _ => "Analysing",
                };
                let pct = job.progress.load(Ordering::Relaxed) as f32 / 10.0;
                ui.label(format!("{stage}... {pct:.0}%"));
                ui.separator();
            }
            if let Some(err) = &self.error {
                ui.colored_label(Color32::from_rgb(255, 110, 110), err);
                ui.separator();
            }
            if let Some(l) = &self.loaded {
                ui.label(format!(
                    "{}  |  {} Hz  |  {} ch  |  {}  |  FFT {} ({:.1} Hz/bin)",
                    file_name(&l.path),
                    l.audio.sample_rate,
                    l.audio.channels.len(),
                    format_time(l.audio.duration(), 0.001, l.audio.duration() >= 60.0),
                    l.spec.fft_size,
                    l.spec.bin_hz(),
                ));
                ui.separator();
            }
            if let Some(info) = &self.hover_info {
                ui.monospace(info);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(
                    "Wheel: zoom time | Ctrl+wheel: zoom freq | Shift/Alt+wheel: pan | \
                     Drag: pan | Right-drag: zoom box | Double-click: fit",
                );
            });
        });
    }

    fn main_view(&mut self, ui: &mut egui::Ui) {
        let Some(loaded) = &self.loaded else {
            let text = if self.job.is_some() {
                "Loading..."
            } else {
                "Open an audio file (MP3, WAV, FLAC, OGG, M4A) or drop it onto this window"
            };
            ui.centered_and_justified(|ui| ui.heading(text));
            return;
        };
        let spec = loaded.spec.clone();
        let peaks = loaded.peaks.clone();
        let duration = loaded.audio.duration();
        let sample_rate = loaded.audio.sample_rate as f64;
        let nyq = spec.nyquist();

        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, Sense::hover());
        let (left, right) = (full.left() + AXIS_LEFT, full.right() - AXIS_RIGHT);
        if right - left < 50.0 || full.height() < 150.0 {
            return;
        }
        let overview_rect = Rect::from_min_max(
            pos2(left, full.top()),
            pos2(right, full.top() + OVERVIEW_HEIGHT),
        );
        let mut top = overview_rect.bottom() + GAP;
        let bottom = full.bottom() - RULER_HEIGHT;
        let wave_rect = self.show_waveform.then(|| {
            let h = ((bottom - top) * 0.25).clamp(50.0, 160.0);
            let r = Rect::from_min_max(pos2(left, top), pos2(right, top + h));
            top += h + GAP;
            r
        });
        let spec_rect = Rect::from_min_max(pos2(left, top), pos2(right, bottom));

        // --- interaction -------------------------------------------------------------------
        let over_resp = ui.interact(
            overview_rect,
            ui.scope_id().with("overview"),
            Sense::click_and_drag(),
        );
        let spec_resp = ui.interact(
            spec_rect,
            ui.scope_id().with("spec"),
            Sense::click_and_drag(),
        );
        let wave_resp =
            wave_rect.map(|r| ui.interact(r, ui.scope_id().with("wave"), Sense::click_and_drag()));

        self.hover_info = None;
        self.handle_lane(ui, &spec_resp, spec_rect, true, duration, nyq, &spec);
        if let (Some(resp), Some(rect)) = (&wave_resp, wave_rect) {
            self.handle_lane(ui, resp, rect, false, duration, nyq, &spec);
        }
        self.handle_overview(ui, &over_resp, overview_rect, duration);
        self.handle_keys(ui);
        self.view.clamp(duration, 64.0 / sample_rate);

        // --- drawing -----------------------------------------------------------------------
        let painter = ui.painter_at(full);
        self.draw_spectrogram(ui, &painter, spec_rect, &spec);
        self.draw_freq_axis(&painter, spec_rect, nyq);
        self.draw_colorbar(&painter, spec_rect);
        draw_time_ruler(&painter, spec_rect, self.view, duration);
        if let Some(rect) = wave_rect {
            draw_waveform(&painter, rect, self.view, &peaks, sample_rate);
        }
        draw_overview(&painter, overview_rect, self.view, duration, &peaks);

        // Cursor line across the lanes.
        let hover = [Some(&spec_resp), wave_resp.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|r| r.hover_pos());
        if let Some(pos) = hover {
            let lanes_top = wave_rect.map_or(spec_rect.top(), |r| r.top());
            painter.vline(
                pos.x,
                lanes_top..=spec_rect.bottom(),
                Stroke::new(1.0, Color32::from_white_alpha(90)),
            );
        }

        // Zoom box preview.
        if let (Some((start, in_spec)), Some(pos)) =
            (self.zoom_box, ui.input(|i| i.pointer.latest_pos()))
        {
            let lane = if in_spec {
                spec_rect
            } else {
                wave_rect.unwrap_or(spec_rect)
            };
            let mut r = Rect::from_two_pos(start, pos).intersect(lane);
            if !in_spec {
                r = Rect::from_x_y_ranges(r.x_range(), lane.y_range());
            }
            painter.rect_filled(r, 0.0, Color32::from_white_alpha(25));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
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
            if zoom != 1.0 {
                if is_spec {
                    self.view.zoom_freq(zoom as f64, y_to_u(pos.y));
                } else {
                    self.view.zoom_time(zoom as f64, t);
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

        if resp.dragged_by(PointerButton::Primary) {
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

        if resp.double_clicked() {
            self.view = View::full(duration);
        }
    }

    fn handle_overview(&mut self, ui: &egui::Ui, resp: &Response, rect: Rect, duration: f64) {
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
            self.view = View::full(duration);
        }
    }

    fn handle_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let Some(duration) = self.loaded.as_ref().map(|l| l.audio.duration()) else {
            return;
        };
        let center = (self.view.t0 + self.view.t1) / 2.0;
        let span = self.view.t_span();
        ui.input(|i| {
            if i.key_pressed(Key::Home) || i.key_pressed(Key::F) {
                self.view = View::full(duration);
            }
            if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
                self.view.zoom_time(1.5, center);
            }
            if i.key_pressed(Key::Minus) {
                self.view.zoom_time(1.0 / 1.5, center);
            }
            if i.key_pressed(Key::ArrowLeft) {
                self.view.t0 -= span * 0.1;
                self.view.t1 -= span * 0.1;
            }
            if i.key_pressed(Key::ArrowRight) {
                self.view.t0 += span * 0.1;
                self.view.t1 += span * 0.1;
            }
        });
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
            let image = spectrum::render(spec, &key.0);
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
            self.rendered = Some(key);
        }
        if let Some(tex) = &self.texture {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
        }
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, Color32::from_gray(70)),
            StrokeKind::Outside,
        );
    }

    fn draw_freq_axis(&self, painter: &egui::Painter, rect: Rect, nyq: f64) {
        let v = self.view;
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

        let font = FontId::proportional(11.0);
        let mut last_y = f32::INFINITY;
        for f in ticks {
            let y = u_to_y(freq_to_u(f, nyq, self.log_scale));
            if (last_y - y).abs() < 16.0 {
                continue;
            }
            last_y = y;
            let x = rect.left();
            painter.line_segment([pos2(x - 5.0, y), pos2(x, y)], Stroke::new(1.0, AXIS_TEXT));
            painter.hline(
                rect.x_range(),
                y,
                Stroke::new(1.0, Color32::from_white_alpha(14)),
            );
            painter.text(
                pos2(x - 7.0, y),
                Align2::RIGHT_CENTER,
                freq_label(f),
                font.clone(),
                AXIS_TEXT,
            );
        }
        painter.text(
            pos2(rect.left() - AXIS_LEFT + 4.0, rect.top()),
            Align2::LEFT_TOP,
            "Hz",
            FontId::proportional(11.0),
            AXIS_TEXT,
        );
    }

    fn draw_colorbar(&self, painter: &egui::Painter, rect: Rect) {
        let bar = Rect::from_min_size(
            pos2(rect.right() + 10.0, rect.top()),
            vec2(12.0, rect.height()),
        );
        let steps = 96;
        for i in 0..steps {
            let t = 1.0 - (i as f32 + 0.5) / steps as f32;
            let y0 = bar.top() + i as f32 / steps as f32 * bar.height();
            let y1 = bar.top() + (i + 1) as f32 / steps as f32 * bar.height();
            let r = Rect::from_x_y_ranges(bar.x_range(), y0..=y1 + 0.5);
            painter.rect_filled(r, 0.0, self.colormap.color(t));
        }
        painter.rect_stroke(
            bar,
            0.0,
            Stroke::new(1.0, Color32::from_gray(70)),
            StrokeKind::Outside,
        );

        let range = (self.ceil_db - self.floor_db) as f64;
        let step = nice_step(range, (bar.height() / 40.0).max(2.0) as usize);
        let mut db = (self.floor_db as f64 / step).ceil() * step;
        while db <= self.ceil_db as f64 + 1e-9 {
            let y = bar.bottom() - ((db - self.floor_db as f64) / range) as f32 * bar.height();
            painter.line_segment(
                [pos2(bar.right(), y), pos2(bar.right() + 4.0, y)],
                Stroke::new(1.0, AXIS_TEXT),
            );
            painter.text(
                pos2(bar.right() + 6.0, y),
                Align2::LEFT_CENTER,
                format!("{db:.0}"),
                FontId::proportional(11.0),
                AXIS_TEXT,
            );
            db += step;
        }
        painter.text(
            pos2(bar.center().x, bar.top() - 2.0),
            Align2::CENTER_BOTTOM,
            "dB",
            FontId::proportional(11.0),
            AXIS_TEXT,
        );
    }
}

impl eframe::App for AudioPaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job(&ctx);

        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open(path, &ctx);
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| self.main_view(ui));

        self.check_reanalysis(&ctx);
    }
}

fn analyze(audio: &AudioData, a: Analysis, progress: &AtomicU32) -> (Arc<Spectrogram>, Arc<Peaks>) {
    let channel = a.channel.filter(|&c| c < audio.channels.len());
    let samples = audio.channel_or_mix(channel);
    let spec = Spectrogram::analyze(&samples, audio.sample_rate, a.fft_size, a.window, progress);
    (Arc::new(spec), Arc::new(Peaks::new(samples)))
}

fn draw_time_ruler(painter: &egui::Painter, rect: Rect, v: View, duration: f64) {
    let t_to_x = |t: f64| rect.left() + ((t - v.t0) / v.t_span()) as f32 * rect.width();
    let step = nice_time_step(v.t_span(), (rect.width() / 90.0).max(2.0) as usize);
    let long = duration >= 60.0;
    let mut t = (v.t0 / step).ceil() * step;
    while t <= v.t1 {
        let x = t_to_x(t);
        let y = rect.bottom();
        painter.line_segment([pos2(x, y), pos2(x, y + 5.0)], Stroke::new(1.0, AXIS_TEXT));
        painter.text(
            pos2(x, y + 6.0),
            Align2::CENTER_TOP,
            format_time(t, step, long),
            FontId::proportional(11.0),
            AXIS_TEXT,
        );
        t += step;
    }
}

fn draw_waveform(painter: &egui::Painter, rect: Rect, v: View, peaks: &Peaks, sample_rate: f64) {
    painter.rect_filled(rect, 0.0, Color32::from_gray(14));
    let mid = rect.center().y;
    let half = rect.height() / 2.0 * 0.95;
    let amp_to_y = |s: f32| mid - s.clamp(-1.0, 1.0) * half;
    painter.hline(
        rect.x_range(),
        mid,
        Stroke::new(1.0, Color32::from_gray(60)),
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
                    Stroke::new(1.0, WAVE_COLOR),
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
                let x = rect.left() + ((t - v.t0) / v.t_span()) as f32 * rect.width();
                pos2(x, amp_to_y(peaks.samples[i]))
            })
            .collect();
        let clipped = painter.with_clip_rect(rect);
        if samples_per_px < 0.1 {
            for p in &points {
                clipped.circle_filled(*p, 2.0, WAVE_COLOR);
            }
        }
        clipped.line(points, Stroke::new(1.5, WAVE_COLOR));
    }

    for (s, label, anchor) in [
        (1.0, "1.0", Align2::RIGHT_TOP),
        (0.0, "0", Align2::RIGHT_CENTER),
        (-1.0, "-1.0", Align2::RIGHT_BOTTOM),
    ] {
        painter.text(
            pos2(rect.left() - 7.0, amp_to_y(s)),
            anchor,
            label,
            FontId::proportional(11.0),
            AXIS_TEXT,
        );
    }
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(70)),
        StrokeKind::Outside,
    );
}

fn draw_overview(painter: &egui::Painter, rect: Rect, v: View, duration: f64, peaks: &Peaks) {
    painter.rect_filled(rect, 2.0, Color32::from_gray(24));
    let n = peaks.samples.len() as f64;
    let mid = rect.center().y;
    let half = rect.height() / 2.0 * 0.9;
    let mut x = 0.0;
    while x < rect.width() {
        let a = (x as f64 / rect.width() as f64 * n) as usize;
        let b = ((x + 1.0) as f64 / rect.width() as f64 * n) as usize;
        if let Some((lo, hi)) = peaks.range(a, b.max(a + 1)) {
            let px = rect.left() + x + 0.5;
            painter.line_segment(
                [
                    pos2(px, mid - hi.clamp(-1.0, 1.0) * half),
                    pos2(px, mid - lo.clamp(-1.0, 1.0) * half + 1.0),
                ],
                Stroke::new(1.0, WAVE_COLOR.gamma_multiply(0.6)),
            );
        }
        x += 1.0;
    }

    let x0 = rect.left() + (v.t0 / duration) as f32 * rect.width();
    let x1 = rect.left() + (v.t1 / duration) as f32 * rect.width();
    let shade = Color32::from_black_alpha(140);
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

fn nice_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let n = raw / mag;
    let s = if n < 1.5 {
        1.0
    } else if n < 3.5 {
        2.0
    } else if n < 7.5 {
        5.0
    } else {
        10.0
    };
    s * mag
}

/// Like [`nice_step`] but prefers minute-friendly steps above 10 seconds.
fn nice_time_step(span: f64, target: usize) -> f64 {
    let raw = span / target.max(1) as f64;
    if raw > 10.0 {
        for s in [15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0] {
            if s >= raw {
                return s;
            }
        }
        return 3600.0 * (raw / 3600.0).ceil();
    }
    nice_step(span, target)
}

/// Formats seconds with as many decimals as `step` needs; `long` uses m:ss.
fn format_time(t: f64, step: f64, long: bool) -> String {
    let decimals = (-step.log10().floor()).clamp(0.0, 6.0) as u32;
    let scale = 10i64.pow(decimals);
    let total = (t * scale as f64).round() as i64;
    let (whole, frac) = (total / scale, total % scale);
    let frac = if decimals > 0 {
        format!(".{:0w$}", frac, w = decimals as usize)
    } else {
        String::new()
    };
    if long {
        format!("{}:{:02}{frac}", whole / 60, whole % 60)
    } else {
        format!("{whole}{frac} s")
    }
}

fn freq_label(f: f64) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if (k - k.round()).abs() < 1e-6 {
            format!("{}k", k.round())
        } else {
            format!("{k:.1}k")
        }
    } else {
        format!("{f:.0}")
    }
}

fn channel_name(channel: Option<usize>, count: usize) -> String {
    match (channel, count) {
        (None, _) => "Mix".into(),
        (Some(0), 2) => "Left".into(),
        (Some(1), 2) => "Right".into(),
        (Some(c), _) => format!("Ch {}", c + 1),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}
