//! Short-time Fourier analysis and rendering of the spectral (waterfall) view.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use eframe::egui::{Color32, ColorImage};
use rayon::prelude::*;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::colormap::Colormap;

/// Levels are stored as `u8` covering this dB range (~0.6 dB per step).
pub const DB_MIN: f32 = -150.0;
pub const DB_MAX: f32 = 5.0;

/// Lowest frequency shown on the logarithmic scale.
pub const LOG_FREQ_MIN: f64 = 20.0;

pub const FFT_SIZES: [usize; 7] = [256, 512, 1024, 2048, 4096, 8192, 16384];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowKind {
    Hann,
    Hamming,
    BlackmanHarris,
}

impl WindowKind {
    pub const ALL: [Self; 3] = [Self::BlackmanHarris, Self::Hann, Self::Hamming];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hann => "Hann",
            Self::Hamming => "Hamming",
            Self::BlackmanHarris => "Blackman-Harris",
        }
    }

    fn coefficients(self, n: usize) -> Vec<f32> {
        let tau = std::f64::consts::TAU;
        (0..n)
            .map(|i| {
                let x = i as f64 / n as f64;
                let w = match self {
                    Self::Hann => 0.5 - 0.5 * (tau * x).cos(),
                    Self::Hamming => 0.54 - 0.46 * (tau * x).cos(),
                    Self::BlackmanHarris => {
                        0.35875 - 0.48829 * (tau * x).cos() + 0.14128 * (2.0 * tau * x).cos()
                            - 0.01168 * (3.0 * tau * x).cos()
                    }
                };
                w as f32
            })
            .collect()
    }
}

fn db_to_q(db: f32) -> u8 {
    ((db - DB_MIN) / (DB_MAX - DB_MIN) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

pub fn q_to_db(q: u8) -> f32 {
    DB_MIN + q as f32 / 255.0 * (DB_MAX - DB_MIN)
}

/// Maps the normalized vertical position `u` (0 = bottom, 1 = top) to a frequency in Hz.
pub fn u_to_freq(u: f64, nyquist: f64, log: bool) -> f64 {
    if log && nyquist > LOG_FREQ_MIN {
        LOG_FREQ_MIN * (nyquist / LOG_FREQ_MIN).powf(u)
    } else {
        u * nyquist
    }
}

pub fn freq_to_u(f: f64, nyquist: f64, log: bool) -> f64 {
    if log && nyquist > LOG_FREQ_MIN {
        (f.max(1e-9) / LOG_FREQ_MIN).ln() / (nyquist / LOG_FREQ_MIN).ln()
    } else {
        f / nyquist
    }
}

/// Magnitude spectrogram. Frame `i` is centred on sample `i * hop`.
///
/// `levels[0]` holds every frame; each further level halves the frame count by taking the
/// maximum of neighbouring frames, so zoomed-out views stay fast and keep short transients.
pub struct Spectrogram {
    pub sample_rate: u32,
    pub fft_size: usize,
    pub hop: usize,
    pub bins: usize,
    pub num_frames: usize,
    levels: Vec<Vec<u8>>,
}

impl Spectrogram {
    pub fn analyze(
        samples: &[f32],
        sample_rate: u32,
        fft_size: usize,
        window: WindowKind,
        progress: &AtomicU32,
    ) -> Self {
        let hop = fft_size / 4;
        let bins = fft_size / 2 + 1;
        let num_frames = samples.len().div_ceil(hop).max(1);

        let win = window.coefficients(fft_size);
        // Scale so that a full-scale sine wave reads 0 dBFS.
        let norm = win.iter().sum::<f32>() / 2.0;
        let fft = FftPlanner::<f32>::new().plan_fft_forward(fft_size);
        let done = AtomicUsize::new(0);

        let mut data = vec![0u8; num_frames * bins];
        data.par_chunks_mut(bins).enumerate().for_each_init(
            || {
                (
                    vec![Complex32::default(); fft_size],
                    vec![Complex32::default(); fft.get_inplace_scratch_len()],
                )
            },
            |(buf, scratch), (frame, out)| {
                let start = (frame * hop) as isize - (fft_size / 2) as isize;
                for (i, (c, w)) in buf.iter_mut().zip(&win).enumerate() {
                    let idx = start + i as isize;
                    let s = if idx >= 0 && (idx as usize) < samples.len() {
                        samples[idx as usize]
                    } else {
                        0.0
                    };
                    *c = Complex32::new(s * w, 0.0);
                }
                fft.process_with_scratch(buf, scratch);
                for (o, c) in out.iter_mut().zip(buf.iter()) {
                    let mag = c.norm() / norm;
                    *o = db_to_q(20.0 * mag.max(1e-12).log10());
                }

                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(512) {
                    progress.store((n * 1000 / num_frames) as u32, Ordering::Relaxed);
                }
            },
        );

        let mut levels = vec![data];
        let mut frames = num_frames;
        while frames > 1 {
            let prev = levels.last().unwrap();
            let next_frames = frames.div_ceil(2);
            let mut next = vec![0u8; next_frames * bins];
            next.par_chunks_mut(bins).enumerate().for_each(|(j, out)| {
                let a = &prev[2 * j * bins..(2 * j + 1) * bins];
                match prev.get((2 * j + 1) * bins..(2 * j + 2) * bins) {
                    Some(b) => {
                        for ((o, x), y) in out.iter_mut().zip(a).zip(b) {
                            *o = (*x).max(*y);
                        }
                    }
                    None => out.copy_from_slice(a),
                }
            });
            levels.push(next);
            frames = next_frames;
        }

        Self {
            sample_rate,
            fft_size,
            hop,
            bins,
            num_frames,
            levels,
        }
    }

    pub fn nyquist(&self) -> f64 {
        self.sample_rate as f64 / 2.0
    }

    pub fn bin_hz(&self) -> f64 {
        self.sample_rate as f64 / self.fft_size as f64
    }

    /// Fractional frame index for time `t` in seconds.
    pub fn frame_at(&self, t: f64) -> f64 {
        t * self.sample_rate as f64 / self.hop as f64
    }

    fn level_frames(&self, level: usize) -> usize {
        self.levels[level].len() / self.bins
    }

    /// Fills `out` with the per-bin maximum over frames `[fa, fb)`.
    /// Returns `false` if that range lies outside the audio.
    fn column(&self, fa: f64, fb: f64, out: &mut [u8]) -> bool {
        let last = self.num_frames as f64 - 0.5;
        if fb < -0.5 || fa > last {
            return false;
        }
        let span = fb - fa;
        if span < 1.0 {
            let i = ((fa + fb) / 2.0).round().clamp(0.0, last) as usize;
            out.copy_from_slice(&self.levels[0][i * self.bins..(i + 1) * self.bins]);
            return true;
        }

        let level = (span.log2().floor() as usize).min(self.levels.len() - 1);
        let scale = (1usize << level) as f64;
        let n = self.level_frames(level);
        let ia = (fa / scale).floor().max(0.0) as usize;
        let ib = ((fb / scale).ceil() as usize).clamp(ia + 1, n);
        if ia >= n {
            return false;
        }
        let data = &self.levels[level];
        out.copy_from_slice(&data[ia * self.bins..(ia + 1) * self.bins]);
        for i in ia + 1..ib {
            for (o, v) in out
                .iter_mut()
                .zip(&data[i * self.bins..(i + 1) * self.bins])
            {
                *o = (*o).max(*v);
            }
        }
        true
    }

    /// Level in dB at time `t` and frequency `f`, if inside the analysed range.
    pub fn value_at(&self, t: f64, f: f64) -> Option<f32> {
        let frame = self.frame_at(t).round();
        let bin = (f / self.bin_hz()).round();
        if frame < 0.0 || frame >= self.num_frames as f64 || bin < 0.0 || bin >= self.bins as f64 {
            return None;
        }
        let i = frame as usize * self.bins + bin as usize;
        Some(q_to_db(self.levels[0][i]))
    }
}

/// Everything that determines the rendered spectrogram image.
#[derive(Clone, PartialEq, Debug)]
pub struct RenderParams {
    pub t0: f64,
    pub t1: f64,
    pub u0: f64,
    pub u1: f64,
    pub width: usize,
    pub height: usize,
    pub log_scale: bool,
    pub floor_db: f32,
    pub ceil_db: f32,
    pub colormap: Colormap,
}

enum RowSource {
    /// Interpolate between bin `i` and `i + 1`.
    Interp(usize, f32),
    /// Maximum over bins `lo..hi`.
    Range(usize, usize),
}

pub const BACKGROUND: Color32 = Color32::from_gray(18);

/// Renders the view described by `p` into an image of `p.width` x `p.height` pixels.
pub fn render(spec: &Spectrogram, p: &RenderParams) -> ColorImage {
    let (w, h) = (p.width.max(1), p.height.max(1));
    let nyq = spec.nyquist();
    let bin_hz = spec.bin_hz();
    let last_bin = spec.bins - 1;

    let rows: Vec<RowSource> = (0..h)
        .map(|y| {
            let u_top = p.u1 - y as f64 / h as f64 * (p.u1 - p.u0);
            let u_bot = p.u1 - (y + 1) as f64 / h as f64 * (p.u1 - p.u0);
            let b_hi = u_to_freq(u_top, nyq, p.log_scale) / bin_hz;
            let b_lo = u_to_freq(u_bot, nyq, p.log_scale) / bin_hz;
            if b_hi - b_lo <= 1.0 {
                let c = ((b_lo + b_hi) / 2.0).clamp(0.0, last_bin as f64);
                let i = (c.floor() as usize).min(last_bin.saturating_sub(1));
                RowSource::Interp(i, (c - i as f64) as f32)
            } else {
                let lo = (b_lo.round() as usize).min(last_bin);
                let hi = (b_hi.round() as usize).clamp(lo + 1, spec.bins);
                RowSource::Range(lo, hi)
            }
        })
        .collect();

    let range = (p.ceil_db - p.floor_db).max(1.0);
    let lut: Vec<Color32> = (0..=255u8)
        .map(|q| {
            p.colormap
                .color(((q_to_db(q) - p.floor_db) / range).clamp(0.0, 1.0))
        })
        .collect();

    let frame_t0 = spec.frame_at(p.t0);
    let frames_per_px = (spec.frame_at(p.t1) - frame_t0) / w as f64;

    let columns: Vec<Vec<Color32>> = (0..w)
        .into_par_iter()
        .map_init(
            || vec![0u8; spec.bins],
            |col, x| {
                let fa = frame_t0 + x as f64 * frames_per_px;
                if !spec.column(fa, fa + frames_per_px, col) {
                    return vec![BACKGROUND; h];
                }
                rows.iter()
                    .map(|row| {
                        let q = match *row {
                            RowSource::Interp(i, frac) => {
                                let a = col[i] as f32;
                                let b = col[(i + 1).min(last_bin)] as f32;
                                (a + (b - a) * frac).round() as u8
                            }
                            RowSource::Range(lo, hi) => col[lo..hi].iter().copied().max().unwrap(),
                        };
                        lut[q as usize]
                    })
                    .collect()
            },
        )
        .collect();

    let mut pixels = vec![BACKGROUND; w * h];
    for (x, col) in columns.iter().enumerate() {
        for (y, c) in col.iter().enumerate() {
            pixels[y * w + x] = *c;
        }
    }
    ColorImage::new([w, h], pixels)
}

/// Min/max summary of a signal for fast waveform drawing at any zoom level.
pub struct Peaks {
    pub samples: Vec<f32>,
    block_min: Vec<f32>,
    block_max: Vec<f32>,
}

const PEAK_BLOCK: usize = 256;

impl Peaks {
    pub fn new(samples: Vec<f32>) -> Self {
        let (block_min, block_max) = samples
            .chunks(PEAK_BLOCK)
            .map(|c| {
                c.iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)))
            })
            .unzip();
        Self {
            samples,
            block_min,
            block_max,
        }
    }

    /// Min and max of samples `a..b` (block-accurate for long ranges).
    pub fn range(&self, a: usize, b: usize) -> Option<(f32, f32)> {
        let b = b.min(self.samples.len());
        if a >= b {
            return None;
        }
        let fold = |(lo, hi): (f32, f32), (x, y): (f32, f32)| (lo.min(x), hi.max(y));
        let init = (f32::MAX, f32::MIN);
        if b - a < 4 * PEAK_BLOCK {
            Some(self.samples[a..b].iter().map(|&s| (s, s)).fold(init, fold))
        } else {
            let (ba, bb) = (a / PEAK_BLOCK, b.div_ceil(PEAK_BLOCK));
            Some(
                self.block_min[ba..bb]
                    .iter()
                    .zip(&self.block_max[ba..bb])
                    .map(|(&x, &y)| (x, y))
                    .fold(init, fold),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peak_has_right_frequency_and_level() {
        let sr = 48000;
        let samples: Vec<f32> = (0..sr)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let spec = Spectrogram::analyze(&samples, sr, 4096, WindowKind::Hann, &AtomicU32::new(0));
        let db = spec.value_at(0.5, 1000.0).unwrap();
        assert!((db - -6.0).abs() < 1.5, "level at 1 kHz was {db} dB");
        let off = spec.value_at(0.5, 5000.0).unwrap();
        assert!(off < -60.0, "level at 5 kHz was {off} dB");
    }

    #[test]
    fn render_produces_requested_size() {
        let samples = vec![0.0; 10_000];
        let spec = Spectrogram::analyze(&samples, 8000, 512, WindowKind::Hann, &AtomicU32::new(0));
        let p = RenderParams {
            t0: 0.0,
            t1: 1.25,
            u0: 0.0,
            u1: 1.0,
            width: 300,
            height: 200,
            log_scale: true,
            floor_db: -100.0,
            ceil_db: 0.0,
            colormap: Colormap::Spectral,
        };
        assert_eq!(render(&spec, &p).size, [300, 200]);
    }
}
