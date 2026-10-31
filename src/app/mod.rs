//! Application state and logic, laid out like Adobe Audition's Waveform editor.
//!
//! The UI is split into `panels` (menu, tool bar, side and bottom panels, dialogs) and
//! `editor` (navigator, waveform lanes, spectral display, rulers and their tools).

mod editor;
mod panels;
mod theme;
mod util;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{self, ColorImage, Key, Modifiers, Pos2, TextureHandle};

use crate::audio::{self, AudioData, SaveFormat};
use crate::colormap::Colormap;
use crate::edit::{self, EditOp, Effect, PasteMode};
use crate::player::Player;
use crate::selection::{Selection, Shape};
use crate::spectrum::{Peaks, RenderParams, Spectrogram, WindowKind};

use util::file_name;

const MIN_U_SPAN: f64 = 0.002;
const MAX_HISTORY: usize = 30;
/// Distance in points within which a drag grabs a selection edge.
const EDGE_GRAB: f32 = 6.0;
const AUDIO_EXTENSIONS: [&str; 7] = ["mp3", "wav", "flac", "ogg", "m4a", "aac", "mp4"];
/// Highest waveform amplitude zoom factor.
const MAX_AMP_ZOOM: f32 = 64.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Time,
    Marquee,
    Lasso,
    Brush,
    Heal,
    Hand,
}

impl Tool {
    const ALL: [Self; 6] = [
        Self::Time,
        Self::Marquee,
        Self::Lasso,
        Self::Brush,
        Self::Heal,
        Self::Hand,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Time => "Time Selection",
            Self::Marquee => "Marquee",
            Self::Lasso => "Lasso",
            Self::Brush => "Paintbrush",
            Self::Heal => "Spot Healing",
            Self::Hand => "Hand",
        }
    }

    fn key(self) -> Key {
        match self {
            Self::Time => Key::T,
            Self::Marquee => Key::E,
            Self::Lasso => Key::D,
            Self::Brush => Key::B,
            Self::Heal => Key::S,
            Self::Hand => Key::H,
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Time => "Time Selection tool (T): drag to select a time range",
            Self::Marquee => "Marquee Selection tool (E): select a rectangular block",
            Self::Lasso => "Lasso Selection tool (D): select a freehand shape",
            Self::Brush => "Paintbrush Selection tool (B): paint a selection; Alt erases",
            Self::Heal => "Spot Healing Brush tool (S): paint over a blemish to repair it",
            Self::Hand => "Hand tool (H): drag to scroll the view",
        }
    }

    fn uses_brush(self) -> bool {
        matches!(self, Self::Brush | Self::Heal)
    }
}

/// Settings that require re-running the FFT when changed.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Analysis {
    fft_size: usize,
    window: WindowKind,
    /// Channel shown in the spectral display; `None` = mono mix.
    channel: Option<usize>,
}

struct Loaded {
    path: PathBuf,
    audio: Arc<AudioData>,
    spec: Arc<Spectrogram>,
    /// One per channel, for the waveform lanes.
    peaks: Vec<Arc<Peaks>>,
    analysis: Analysis,
}

/// Edit history of one file, like Audition's History panel.
#[derive(Default)]
struct History {
    entries: Vec<(String, Arc<AudioData>)>,
    pos: usize,
    /// Entry that matches the file on disk (or the last save).
    saved: Option<usize>,
}

impl History {
    fn new(audio: Arc<AudioData>) -> Self {
        Self {
            entries: vec![("Open".into(), audio)],
            pos: 0,
            saved: Some(0),
        }
    }

    fn push(&mut self, label: String, audio: Arc<AudioData>) {
        if self.saved.is_some_and(|s| s > self.pos) {
            self.saved = None; // that state is discarded below
        }
        self.entries.truncate(self.pos + 1);
        self.entries.push((label, audio));
        self.pos += 1;
        if self.entries.len() > MAX_HISTORY {
            self.entries.remove(0);
            self.pos -= 1;
            self.saved = self.saved.and_then(|s| s.checked_sub(1));
        }
    }

    fn modified(&self) -> bool {
        !self.entries.is_empty() && self.saved != Some(self.pos)
    }

    fn can_undo(&self) -> bool {
        self.pos > 0
    }

    fn can_redo(&self) -> bool {
        self.pos + 1 < self.entries.len()
    }
}

enum JobResult {
    Loaded {
        path: PathBuf,
        audio: Arc<AudioData>,
        spec: Arc<Spectrogram>,
        peaks: Vec<Arc<Peaks>>,
    },
    Analyzed {
        spec: Arc<Spectrogram>,
        peaks: Vec<Arc<Peaks>>,
    },
    Edited {
        audio: Arc<AudioData>,
        spec: Arc<Spectrogram>,
        peaks: Vec<Arc<Peaks>>,
        /// Time range to select afterwards (e.g. the pasted audio).
        select: Option<(f64, f64)>,
        label: String,
    },
    Failed(String),
}

struct Job {
    rx: Receiver<JobResult>,
    /// Per mille of the current stage.
    progress: Arc<AtomicU32>,
    /// 0 = decoding, 1 = analysing, 2 = processing.
    stage: Arc<AtomicU32>,
    target: Analysis,
    is_load: bool,
}

/// Visible region: time in seconds, frequency as normalized scale position (0 = bottom),
/// and the waveform amplitude zoom.
#[derive(Clone, Copy, PartialEq, Debug)]
struct View {
    t0: f64,
    t1: f64,
    u0: f64,
    u1: f64,
    amp: f32,
}

impl View {
    fn full(duration: f64) -> Self {
        Self {
            t0: 0.0,
            t1: duration,
            u0: 0.0,
            u1: 1.0,
            amp: 1.0,
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
        self.amp = self.amp.clamp(1.0, MAX_AMP_ZOOM);
    }
}

/// An open file that is not the one shown in the editor.
struct DocState {
    loaded: Loaded,
    history: History,
    view: View,
    selection: Option<(f64, f64)>,
    playhead: f64,
    spectral: Selection,
    channel: Option<usize>,
}

/// Tabs of the lower side panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SideTab {
    History,
    Effects,
}

/// Effects with a parameter dialog.
#[derive(Clone, Copy, PartialEq)]
enum EffectDialog {
    Amplify(f32),
    Normalize(f32),
}

/// Level meter state per channel, in dBFS.
#[derive(Clone, Copy)]
struct Meter {
    level: f32,
    peak: f32,
    peak_time: f64,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            level: f32::NEG_INFINITY,
            peak: f32::NEG_INFINITY,
            peak_time: 0.0,
        }
    }
}

pub struct AudioPaintApp {
    // --- the file shown in the editor -----------------------------------------------------
    loaded: Option<Loaded>,
    history: History,
    /// All open files in Files-panel order; the slot `active` is `None` (it lives in the
    /// fields above).
    docs: Vec<Option<DocState>>,
    active: usize,
    pending_open: VecDeque<PathBuf>,
    job: Option<Job>,
    error: Option<String>,
    notice: Option<String>,

    // --- spectral display settings ---------------------------------------------------------
    fft_size: usize,
    window: WindowKind,
    channel: Option<usize>,
    log_scale: bool,
    floor_db: f32,
    ceil_db: f32,
    colormap: Colormap,
    show_waveform: bool,
    show_spectral: bool,

    // --- view and rendering ----------------------------------------------------------------
    view: View,
    texture: Option<TextureHandle>,
    rendered: Option<(RenderParams, usize)>,
    /// The spectrogram without the selection overlay.
    base_image: Option<ColorImage>,
    overlay_key: Option<(Option<(RenderParams, usize)>, u64)>,
    /// Start of a right-drag zoom box, and whether it started in the spectral display.
    zoom_box: Option<(Pos2, bool)>,
    hover_info: Option<String>,

    // --- selection and playback ------------------------------------------------------------
    /// Selected time range in seconds (start < end).
    selection: Option<(f64, f64)>,
    /// Fixed end of the selection while it is being dragged.
    selection_anchor: Option<f64>,
    /// Current-time indicator: playback without a selection starts here.
    playhead: f64,
    player: Player,
    /// Range being played, kept for loop playback.
    playing: Option<(f64, f64)>,
    follow_playback: bool,
    loop_playback: bool,
    meters: Vec<Meter>,
    meter_time: f64,

    // --- spectral tools and edits ----------------------------------------------------------
    tool: Tool,
    spectral: Selection,
    /// Bumped whenever `spectral` or `drawing` changes, to refresh the overlay.
    overlay_version: u64,
    /// Shape being drawn with the current tool, and whether it adds to the selection.
    drawing: Option<(Shape, bool)>,
    draw_origin: Pos2,
    draw_last: Pos2,
    /// Brush radius in points.
    brush_radius: f32,
    gain_db: f32,
    /// Audio copied with Copy; kept when another file is opened.
    clipboard: Option<Arc<AudioData>>,

    // --- panels and dialogs ----------------------------------------------------------------
    show_side_panel: bool,
    side_tab: SideTab,
    show_levels: bool,
    show_selection_view: bool,
    show_status_bar: bool,
    settings_window: bool,
    shortcuts_window: bool,
    about_window: bool,
    effect_dialog: Option<EffectDialog>,
    /// Index of a modified file waiting for "close anyway?".
    close_confirm: Option<usize>,
    save_dialog: bool,
    save_format: SaveFormat,
    save_job: Option<Receiver<Result<PathBuf, String>>>,
}

impl AudioPaintApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_files: Vec<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut app = Self {
            loaded: None,
            history: History::default(),
            docs: Vec::new(),
            active: 0,
            pending_open: VecDeque::new(),
            job: None,
            error: None,
            notice: None,
            fft_size: 4096,
            window: WindowKind::BlackmanHarris,
            channel: None,
            log_scale: true,
            floor_db: -110.0,
            ceil_db: 0.0,
            colormap: Colormap::Spectral,
            show_waveform: true,
            show_spectral: true,
            view: View::full(1.0),
            texture: None,
            rendered: None,
            base_image: None,
            overlay_key: None,
            zoom_box: None,
            hover_info: None,
            selection: None,
            selection_anchor: None,
            playhead: 0.0,
            player: Player::default(),
            playing: None,
            follow_playback: true,
            loop_playback: false,
            meters: Vec::new(),
            meter_time: 0.0,
            tool: Tool::Time,
            spectral: Selection::default(),
            overlay_version: 0,
            drawing: None,
            draw_origin: Pos2::ZERO,
            draw_last: Pos2::ZERO,
            brush_radius: 12.0,
            gain_db: -12.0,
            clipboard: None,
            show_side_panel: true,
            side_tab: SideTab::History,
            show_levels: true,
            show_selection_view: true,
            show_status_bar: true,
            settings_window: false,
            shortcuts_window: false,
            about_window: false,
            effect_dialog: None,
            close_confirm: None,
            save_dialog: false,
            save_format: SaveFormat::Wav16,
            save_job: None,
        };
        for path in initial_files {
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

    fn duration(&self) -> Option<f64> {
        self.loaded.as_ref().map(|l| l.audio.duration())
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    // --- files -----------------------------------------------------------------------------

    /// Opens a file in a new editor tab (queued while another file is loading).
    fn open(&mut self, path: PathBuf, ctx: &egui::Context) {
        if self.job.as_ref().is_some_and(|j| j.is_load) {
            self.pending_open.push_back(path);
            return;
        }
        let target = Analysis {
            channel: None,
            ..self.analysis()
        };
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

    fn pick_files(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio files", &AUDIO_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_files();
        for path in picked.unwrap_or_default() {
            self.open(path, ctx);
        }
    }

    /// Moves the shown file into `docs`, returning its state.
    fn take_active(&mut self) -> Option<DocState> {
        let loaded = self.loaded.take()?;
        self.stop_playback();
        self.drawing = None;
        self.selection_anchor = None;
        Some(DocState {
            loaded,
            history: std::mem::take(&mut self.history),
            view: self.view,
            selection: self.selection.take(),
            playhead: self.playhead,
            spectral: std::mem::take(&mut self.spectral),
            channel: self.channel,
        })
    }

    fn restore(&mut self, doc: DocState, ctx: &egui::Context) {
        self.channel = doc.channel;
        self.loaded = Some(doc.loaded);
        self.history = doc.history;
        self.view = doc.view;
        self.selection = doc.selection;
        self.playhead = doc.playhead;
        self.spectral = doc.spectral;
        self.overlay_version += 1;
        self.meters.clear();
        self.update_title(ctx);
    }

    fn switch_to(&mut self, index: usize, ctx: &egui::Context) {
        if index == self.active || index >= self.docs.len() || self.busy() {
            return;
        }
        if let Some(current) = self.take_active() {
            self.docs[self.active] = Some(current);
        }
        if let Some(doc) = self.docs[index].take() {
            self.active = index;
            self.restore(doc, ctx);
        }
    }

    /// Closes a file; asks first when it has unsaved changes (unless `force`).
    fn close_doc(&mut self, index: usize, force: bool, ctx: &egui::Context) {
        if index >= self.docs.len() || (index == self.active && self.busy()) {
            return;
        }
        let modified = if index == self.active {
            self.history.modified()
        } else {
            self.docs[index]
                .as_ref()
                .is_some_and(|d| d.history.modified())
        };
        if modified && !force {
            self.close_confirm = Some(index);
            return;
        }
        if index == self.active {
            self.take_active();
            self.history = History::default();
            self.docs.remove(index);
            if !self.docs.is_empty() {
                self.active = index.min(self.docs.len() - 1);
                if let Some(doc) = self.docs[self.active].take() {
                    self.restore(doc, ctx);
                }
            } else {
                self.active = 0;
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(crate::APP_TITLE.into()));
            }
        } else {
            self.docs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
        }
    }

    fn update_title(&self, ctx: &egui::Context) {
        let title = match &self.loaded {
            Some(l) => format!("{} - {}", file_name(&l.path), crate::APP_TITLE),
            None => crate::APP_TITLE.into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    /// (name, modified, duration, sample rate, channels) of the file in `docs[index]`.
    fn doc_info(&self, index: usize) -> Option<(String, bool, f64, u32, usize)> {
        let (loaded, history) = if index == self.active {
            (self.loaded.as_ref()?, &self.history)
        } else {
            let d = self.docs.get(index)?.as_ref()?;
            (&d.loaded, &d.history)
        };
        Some((
            file_name(&loaded.path),
            history.modified(),
            loaded.audio.duration(),
            loaded.audio.sample_rate,
            loaded.audio.channels.len(),
        ))
    }

    // --- background jobs -------------------------------------------------------------------

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
                if let Some(current) = self.take_active() {
                    self.docs[self.active] = Some(current);
                }
                self.docs.push(None);
                self.active = self.docs.len() - 1;
                self.view = View::full(audio.duration());
                self.selection = None;
                self.playhead = 0.0;
                self.spectral.clear();
                self.overlay_version += 1;
                self.channel = target.channel;
                self.meters.clear();
                self.notice = None;
                self.history = History::new(audio.clone());
                self.loaded = Some(Loaded {
                    path,
                    audio,
                    spec,
                    peaks,
                    analysis: target,
                });
                self.update_title(ctx);
                if let Some(next) = self.pending_open.pop_front() {
                    self.open(next, ctx);
                }
            }
            JobResult::Analyzed { spec, peaks } => {
                if let Some(loaded) = &mut self.loaded {
                    loaded.spec = spec;
                    loaded.peaks = peaks;
                    loaded.analysis = target;
                }
            }
            JobResult::Edited {
                audio,
                spec,
                peaks,
                select,
                label,
            } => {
                if let Some((a, b)) = select {
                    self.selection = Some((a, b));
                    self.playhead = a;
                    self.clear_spectral();
                }
                if let Some(loaded) = &mut self.loaded {
                    self.history.push(label, audio.clone());
                    loaded.audio = audio;
                    loaded.spec = spec;
                    loaded.peaks = peaks;
                    loaded.analysis = target;
                }
            }
            JobResult::Failed(message) => {
                self.error = Some(message);
                if let Some(next) = self.pending_open.pop_front() {
                    self.open(next, ctx);
                }
            }
        }
    }

    /// Starts a new analysis when FFT size, window or channel differ from the displayed one.
    fn check_reanalysis(&mut self, ctx: &egui::Context) {
        if let Some(l) = &self.loaded
            && self.channel.is_some_and(|c| c >= l.audio.channels.len())
        {
            self.channel = None;
        }
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

    // --- edits and history -----------------------------------------------------------------

    /// Runs `work` on the current audio in the background, then re-analyses; the result
    /// becomes a History entry named `label`. `work` may return a time range to select.
    fn run_audio_job(
        &mut self,
        ctx: &egui::Context,
        label: String,
        work: impl FnOnce(&AudioData) -> Result<(AudioData, Option<(f64, f64)>), String>
        + Send
        + 'static,
    ) {
        if self.job.as_ref().is_some_and(|j| j.is_load) {
            return;
        }
        let Some(loaded) = &self.loaded else { return };
        let audio = loaded.audio.clone();
        let target = self.analysis();
        self.stop_playback();
        self.notice = None;
        self.spawn_job(ctx, target, false, move |progress, stage| {
            stage.store(2, Ordering::Relaxed);
            let (edited, select) = work(&audio)?;
            let edited = Arc::new(edited);
            stage.store(1, Ordering::Relaxed);
            progress.store(0, Ordering::Relaxed);
            let (spec, peaks) = analyze(&edited, target, progress);
            Ok(JobResult::Edited {
                audio: edited,
                spec,
                peaks,
                select,
                label,
            })
        });
    }

    /// Channels that edits apply to: the one shown in the spectral display, or all.
    fn edit_channels(&self) -> Vec<usize> {
        match (self.channel, &self.loaded) {
            (Some(c), _) => vec![c],
            (None, Some(l)) => (0..l.audio.channels.len()).collect(),
            (None, None) => Vec::new(),
        }
    }

    fn run_edit(&mut self, ctx: &egui::Context, label: String, op: EditOp, region: Selection) {
        let channels = self.edit_channels();
        self.run_audio_job(ctx, label, move |audio| {
            Ok((edit::apply(audio, &region, op, &channels), None))
        });
    }

    /// The area a spectral edit applies to: the spectral selection, or else the time
    /// selection across all frequencies.
    fn edit_region(&self) -> Option<Selection> {
        if !self.spectral.is_empty() {
            return Some(self.spectral.clone());
        }
        let (a, b) = self.selection?;
        let nyq = self.loaded.as_ref()?.audio.sample_rate as f64 / 2.0;
        let mut region = Selection::default();
        region.push(Shape::rect(a, b, -1.0, nyq * 1.1), true);
        Some(region)
    }

    fn edit_selection(&mut self, ctx: &egui::Context, op: EditOp) {
        let Some(region) = self.edit_region() else {
            return;
        };
        let spectral = !self.spectral.is_empty();
        let label = match op {
            EditOp::Gain(0.0) if spectral => "Delete (spectral)".to_string(),
            EditOp::Gain(0.0) => "Silence".to_string(),
            EditOp::Gain(g) => format!("Gain {:+.1} dB", 20.0 * g.log10()),
            EditOp::Heal => "Heal".to_string(),
        };
        self.run_edit(ctx, label, op, region);
    }

    /// The time range for time-domain actions: the time selection, or the spectral
    /// selection's extent.
    fn time_range(&self) -> Option<(f64, f64)> {
        let spectral = self.spectral.bounds().map(|(a, b, _, _)| (a.max(0.0), b));
        self.selection.or(spectral).filter(|(a, b)| b > a)
    }

    /// Applies an effect to the selected time range, or to the whole file.
    fn apply_effect(&mut self, ctx: &egui::Context, effect: Effect) {
        if self.loaded.is_none() {
            return;
        }
        let range = self.time_range();
        let channels = self.edit_channels();
        self.run_audio_job(ctx, effect.name(), move |audio| {
            Ok((edit::apply_effect(audio, range, &channels, effect), range))
        });
    }

    fn copy(&mut self, ctx: &egui::Context) {
        let (Some(loaded), Some((a, b))) = (&self.loaded, self.time_range()) else {
            return;
        };
        let clip = edit::extract(&loaded.audio, a, b);
        let secs = clip.duration();
        self.clipboard = Some(Arc::new(clip));
        // Put a note on the system clipboard too: Ctrl+V only reaches the app when the
        // system clipboard is not empty.
        ctx.copy_text(format!("Audio Paint clip ({secs:.3} s)"));
        self.notice = Some(format!("Copied {secs:.3} s"));
    }

    /// Pastes the clipboard at the playhead and selects the pasted audio.
    fn paste(&mut self, ctx: &egui::Context, mode: PasteMode) {
        let Some(clip) = self.clipboard.clone() else {
            return;
        };
        let at = self.playhead;
        let label = match mode {
            PasteMode::Insert => "Paste",
            PasteMode::Overwrite => "Overwrite Paste",
        };
        self.run_audio_job(ctx, label.into(), move |audio| {
            let (pasted, range) = edit::paste(audio, &clip, at, mode)?;
            Ok((pasted, Some(range)))
        });
    }

    /// Restores the History entry `pos` (undo/redo jump).
    fn goto_history(&mut self, pos: usize, ctx: &egui::Context) {
        if pos >= self.history.entries.len()
            || pos == self.history.pos
            || self.job.as_ref().is_some_and(|j| j.is_load)
        {
            return;
        }
        let Some(loaded) = &mut self.loaded else {
            return;
        };
        self.history.pos = pos;
        loaded.audio = self.history.entries[pos].1.clone();
        let duration = loaded.audio.duration();
        self.stop_playback();
        if let Some((a, b)) = self.selection {
            self.selection = (a < duration).then_some((a, b.min(duration)));
        }
        self.playhead = self.playhead.min(duration);
        self.reanalyze(ctx);
    }

    fn undo(&mut self, ctx: &egui::Context) {
        if self.history.can_undo() {
            self.goto_history(self.history.pos - 1, ctx);
        }
    }

    fn redo(&mut self, ctx: &egui::Context) {
        if self.history.can_redo() {
            self.goto_history(self.history.pos + 1, ctx);
        }
    }

    // --- saving ----------------------------------------------------------------------------

    /// Asks for a file name, then encodes and writes the file in the background.
    fn save_as(&mut self, ctx: &egui::Context) {
        let Some(loaded) = &self.loaded else { return };
        let format = self.save_format;
        let ext = format.extension();
        let stem = loaded
            .path
            .file_stem()
            .map_or("audio".into(), |s| s.to_string_lossy().into_owned());
        let picked = rfd::FileDialog::new()
            .add_filter(format.name(), &[ext])
            .set_file_name(format!("{stem}_edited.{ext}"))
            .save_file();
        let Some(mut path) = picked else { return };
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            != Some(ext.into())
        {
            path.set_extension(ext);
        }
        let audio = loaded.audio.clone();
        let saved_pos = self.history.pos;
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = audio::save(&path, &audio, format).map(|()| path);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        // Marked as saved now; a failed save clears the mark again in `poll_save`.
        self.history.saved = Some(saved_pos);
        self.save_job = Some(rx);
        self.save_dialog = false;
        self.notice = Some(format!("Saving as {}...", format.name()));
        self.error = None;
    }

    fn poll_save(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.save_job else { return };
        match rx.try_recv() {
            Ok(Ok(path)) => {
                self.notice = Some(format!("Saved {}", file_name(&path)));
                self.save_job = None;
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.notice = None;
                self.history.saved = None;
                self.save_job = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => self.save_job = None,
        }
    }

    // --- selection -------------------------------------------------------------------------

    fn clear_spectral(&mut self) {
        self.spectral.clear();
        self.drawing = None;
        self.overlay_version += 1;
    }

    fn select_all(&mut self) {
        if let Some(duration) = self.duration() {
            self.selection = Some((0.0, duration));
            self.playhead = 0.0;
        }
    }

    /// Clears both the time and the spectral selection.
    fn deselect(&mut self) {
        if let Some((a, _)) = self.selection.take() {
            self.playhead = a;
        }
        self.selection_anchor = None;
        self.clear_spectral();
    }

    fn has_selection(&self) -> bool {
        self.selection.is_some() || !self.spectral.is_empty()
    }

    // --- playback --------------------------------------------------------------------------

    fn play_range(&mut self, t0: f64, t1: f64) {
        let Some(loaded) = &self.loaded else { return };
        match self.player.play(&loaded.audio, t0, t1) {
            Ok(()) => self.playing = Some((t0, t1)),
            Err(e) => self.error = Some(e),
        }
    }

    /// Plays the selection, or from the playhead to the end when nothing is selected.
    fn play(&mut self) {
        let Some(duration) = self.duration() else {
            return;
        };
        if self.player.is_paused() {
            self.player.set_paused(false);
            return;
        }
        match self.time_range() {
            Some((a, b)) => self.play_range(a, b),
            None => {
                // Restart from the beginning when the playhead sits at the very end.
                let start = if self.playhead >= duration - 1e-3 {
                    0.0
                } else {
                    self.playhead
                };
                self.play_range(start, duration);
            }
        }
    }

    fn stop_playback(&mut self) {
        self.player.stop();
        self.playing = None;
    }

    fn toggle_pause(&mut self) {
        if self.player.position().is_some() {
            let paused = self.player.is_paused();
            self.player.set_paused(!paused);
        }
    }

    /// Space: play or stop.
    fn toggle_play(&mut self) {
        if self.player.position().is_some() {
            self.stop_playback();
        } else {
            self.play();
        }
    }

    /// Moves the playhead; continues playback from there when playing.
    fn seek(&mut self, t: f64) {
        let Some(duration) = self.duration() else {
            return;
        };
        self.playhead = t.clamp(0.0, duration);
        if self.player.is_playing() {
            self.play_range(self.playhead, duration);
        }
    }

    /// Previous/next stop for the playhead: file start, selection edges, file end.
    fn playhead_stop(&self, forward: bool) -> f64 {
        let duration = self.duration().unwrap_or(0.0);
        let mut stops = vec![0.0, duration];
        if let Some((a, b)) = self.selection {
            stops.extend([a, b]);
        }
        let current = self.player.position().unwrap_or(self.playhead);
        let candidates = stops.into_iter();
        if forward {
            candidates
                .filter(|&t| t > current + 1e-6)
                .fold(duration, f64::min)
        } else {
            candidates
                .filter(|&t| t < current - 1e-6)
                .fold(0.0, f64::max)
        }
    }

    /// Restarts the loop, or forgets the range when playback reached its end.
    fn update_playback(&mut self) {
        if let Some((a, b)) = self.playing
            && self.player.position().is_none()
        {
            if self.loop_playback {
                self.play_range(a, b);
            } else {
                self.playing = None;
            }
        }
    }

    /// Peak level meters of the audio around the play position, with falling bars and
    /// 1.5 s peak hold.
    fn update_meters(&mut self, now: f64) {
        let dt = (now - self.meter_time).clamp(0.0, 0.5) as f32;
        self.meter_time = now;
        let Some(loaded) = &self.loaded else {
            self.meters.clear();
            return;
        };
        let channels = loaded.audio.channels.len();
        self.meters.resize(channels, Meter::default());
        let pos = self.player.position().filter(|_| self.player.is_playing());
        let sr = loaded.audio.sample_rate as f64;
        for (c, meter) in self.meters.iter_mut().enumerate() {
            let level = pos.map_or(f32::NEG_INFINITY, |p| {
                let b = ((p * sr) as usize).min(loaded.audio.num_frames());
                let a = b.saturating_sub((0.05 * sr) as usize);
                let peak = loaded.audio.channels[c][a..b]
                    .iter()
                    .fold(0.0f32, |m, v| m.max(v.abs()));
                20.0 * peak.max(1e-9).log10()
            });
            // Fast attack, 24 dB/s fall.
            meter.level = level.max(meter.level - 24.0 * dt);
            if level >= meter.peak || now - meter.peak_time > 1.5 {
                meter.peak = level.max(meter.level);
                meter.peak_time = now;
            }
        }
    }

    // --- zoom ------------------------------------------------------------------------------

    fn zoom_time_center(&mut self, factor: f64) {
        let center = (self.view.t0 + self.view.t1) / 2.0;
        self.view.zoom_time(factor, center);
    }

    fn zoom_amp(&mut self, factor: f32) {
        self.view.amp = (self.view.amp * factor).clamp(1.0, MAX_AMP_ZOOM);
    }

    fn zoom_full(&mut self) {
        if let Some(d) = self.duration() {
            self.view = View::full(d);
        }
    }

    fn zoom_to_selection(&mut self) {
        if let Some((a, b)) = self.time_range() {
            self.view.t0 = a;
            self.view.t1 = b;
        }
        if let (Some((_, _, f0, f1)), Some(l)) = (self.spectral.bounds(), &self.loaded) {
            let nyq = l.spec.nyquist();
            let u = |f: f64| crate::spectrum::freq_to_u(f.max(1e-3), nyq, self.log_scale);
            self.view.u0 = u(f0).max(0.0);
            self.view.u1 = u(f1).min(1.0);
        }
    }

    // --- keyboard --------------------------------------------------------------------------

    /// Runs before the panels so shortcuts (e.g. Space) are consumed before a focused
    /// button could also react to them.
    fn handle_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Shift is ignored unless asked for, so the Shift variants must be consumed first.
        // Ctrl+C and Ctrl+V arrive as clipboard events, not as key presses.
        let (open, save, close, paste_overwrite, redo) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)
                    || i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::V),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                    || i.consume_key(Modifiers::COMMAND, Key::Y),
            )
        });
        if open {
            self.pick_files(ctx);
        }
        let Some(duration) = self.duration() else {
            return;
        };
        if save && self.save_job.is_none() {
            self.save_dialog = true;
        }
        if close {
            self.close_doc(self.active, false, ctx);
            return;
        }
        if redo {
            self.redo(ctx);
        }
        let (copy, paste_insert) = ctx.input(|i| {
            let has = |f: fn(&egui::Event) -> bool| i.events.iter().any(f);
            (
                has(|e| matches!(e, egui::Event::Copy)),
                has(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))),
            )
        });
        if copy {
            self.copy(ctx);
        }
        if paste_insert && self.clipboard.is_some() {
            self.paste(ctx, PasteMode::Insert);
        }
        if paste_overwrite && self.clipboard.is_some() {
            self.paste(ctx, PasteMode::Overwrite);
        }

        let (undo, select_all, space, escape, delete, home, end, smaller, larger, tool) = ctx
            .input_mut(|i| {
                (
                    i.consume_key(Modifiers::COMMAND, Key::Z),
                    i.consume_key(Modifiers::COMMAND, Key::A),
                    i.consume_key(Modifiers::NONE, Key::Space),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::NONE, Key::Delete)
                        || i.consume_key(Modifiers::NONE, Key::Backspace),
                    i.consume_key(Modifiers::NONE, Key::Home),
                    i.consume_key(Modifiers::NONE, Key::End),
                    i.consume_key(Modifiers::NONE, Key::OpenBracket),
                    i.consume_key(Modifiers::NONE, Key::CloseBracket),
                    Tool::ALL
                        .into_iter()
                        .find(|t| i.consume_key(Modifiers::NONE, t.key())),
                )
            });
        if undo {
            self.undo(ctx);
        }
        if select_all {
            self.select_all();
        }
        if space {
            self.toggle_play();
        }
        if escape {
            self.deselect();
        }
        if delete {
            self.edit_selection(ctx, EditOp::Gain(0.0));
        }
        if home {
            self.seek(0.0);
        }
        if end {
            self.seek(duration);
        }
        if smaller {
            self.brush_radius = (self.brush_radius / 1.25).max(2.0);
        }
        if larger {
            self.brush_radius = (self.brush_radius * 1.25).min(80.0);
        }
        if let Some(tool) = tool {
            self.tool = tool;
        }

        let span = self.view.t_span();
        let (fit, zoom_in, zoom_out, left, right) = ctx.input(|i| {
            (
                i.key_pressed(Key::F),
                i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                i.key_pressed(Key::Minus),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
            )
        });
        if fit {
            self.zoom_full();
        }
        if zoom_in {
            self.zoom_time_center(1.5);
        }
        if zoom_out {
            self.zoom_time_center(1.0 / 1.5);
        }
        if left {
            self.view.t0 -= span * 0.1;
            self.view.t1 -= span * 0.1;
        }
        if right {
            self.view.t0 += span * 0.1;
            self.view.t1 += span * 0.1;
        }
    }
}

impl eframe::App for AudioPaintApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job(&ctx);
        self.poll_save(&ctx);
        self.update_playback();
        self.update_meters(ctx.input(|i| i.time));

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        for path in dropped {
            self.open(path, &ctx);
        }
        self.handle_keys(&ctx);
        if self.player.position().is_some() || self.meters.iter().any(|m| m.level.is_finite()) {
            ctx.request_repaint();
        }

        egui::Panel::top("menu_bar")
            .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(2))
            .show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("tool_bar").show(ui, |ui| self.tool_bar(ui));
        if self.show_status_bar {
            egui::Panel::bottom("status_bar")
                .frame(egui::Frame::new().fill(theme::PANEL_DARK).inner_margin(3))
                .show(ui, |ui| self.status_bar(ui));
        }
        if self.show_levels || self.show_selection_view {
            egui::Panel::bottom("bottom_panels").show(ui, |ui| self.bottom_panels(ui));
        }
        if self.show_side_panel {
            egui::Panel::left("side_panel")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| self.side_panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.editor_panel(ui));

        self.dialogs(&ctx);
        self.check_reanalysis(&ctx);
    }
}

/// Computes the spectrogram (of one channel or the mix) and per-channel waveform peaks.
fn analyze(
    audio: &AudioData,
    a: Analysis,
    progress: &AtomicU32,
) -> (Arc<Spectrogram>, Vec<Arc<Peaks>>) {
    let channel = a.channel.filter(|&c| c < audio.channels.len());
    let samples = audio.channel_or_mix(channel);
    let spec = Spectrogram::analyze(&samples, audio.sample_rate, a.fft_size, a.window, progress);
    let peaks = audio
        .channels
        .iter()
        .map(|c| Arc::new(Peaks::new(c.clone())))
        .collect();
    (Arc::new(spec), peaks)
}
