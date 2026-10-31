//! Spectral editing: STFT, scale the selected time-frequency cells, overlap-add resynthesis.
//!
//! Only the region around the selection is processed. Frames that the selection does not
//! touch reconstruct the input exactly, so audio outside the selection is unchanged.

use rayon::prelude::*;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::audio::AudioData;
use crate::selection::{Grid, Selection, feather, rasterize};

const N: usize = 2048;
const HOP: usize = N / 4;
const BINS: usize = N / 2 + 1;
/// Extra samples processed around the selection, covering the feathered mask edge.
const MARGIN: i64 = 2 * N as i64;
/// Frames processed in parallel before overlap-adding (bounds memory use).
const CHUNK: usize = 256;

#[derive(Clone, Copy, Debug)]
pub enum EditOp {
    /// Multiply the selected area by this linear gain (0 = mute).
    Gain(f32),
    /// Replace the selected area with a level interpolated from the surrounding audio.
    Heal,
}

/// STFT frame `k0 + x` and bin `y` as raster pixels.
struct StftGrid {
    k0: i64,
    frames: usize,
    hop_s: f64,
    bin_hz: f64,
}

impl Grid for StftGrid {
    fn size(&self) -> (usize, usize) {
        (self.frames, BINS)
    }
    fn t_of_x(&self, x: f64) -> f64 {
        (self.k0 as f64 + x - 0.5) * self.hop_s
    }
    fn x_of_t(&self, t: f64) -> f64 {
        t / self.hop_s - self.k0 as f64 + 0.5
    }
    fn f_of_y(&self, y: f64) -> f64 {
        (y - 0.5) * self.bin_hz
    }
    fn y_of_f(&self, f: f64) -> f64 {
        f / self.bin_hz + 0.5
    }
}

/// Returns a copy of `audio` with `op` applied to `selection` in the given channels.
pub fn apply(
    audio: &AudioData,
    selection: &Selection,
    op: EditOp,
    channels: &[usize],
) -> AudioData {
    let mut out = AudioData {
        sample_rate: audio.sample_rate,
        channels: audio.channels.clone(),
    };
    let Some((ta, tb, _, _)) = selection.bounds() else {
        return out;
    };
    let sr = audio.sample_rate as f64;
    let a = (ta * sr).floor() as i64;
    let b = (tb * sr).ceil() as i64;
    let (hop, half) = (HOP as i64, (N / 2) as i64);
    let k0 = (a - MARGIN - half).div_euclid(hop);
    let k1 = (b + MARGIN + half).div_euclid(hop) + 1;
    let grid = StftGrid {
        k0,
        frames: (k1 - k0) as usize,
        hop_s: HOP as f64 / sr,
        bin_hz: sr / N as f64,
    };

    // mask[bin * frames + frame]
    let mut mask = rasterize(&selection.shapes, &grid);
    feather(&mut mask, grid.frames, BINS, 2);

    let processed: Vec<(usize, Vec<f32>)> = channels
        .par_iter()
        .filter(|&&c| c < audio.channels.len())
        .map(|&c| {
            let ch = &audio.channels[c];
            (
                c,
                process_channel(ch, &mask, k0, grid.frames, op, (a - MARGIN, b + MARGIN)),
            )
        })
        .collect();
    for (c, samples) in processed {
        out.channels[c] = samples;
    }
    out
}

fn process_channel(
    x: &[f32],
    mask: &[f32],
    k0: i64,
    frames: usize,
    op: EditOp,
    (write_a, write_b): (i64, i64),
) -> Vec<f32> {
    let len = x.len() as i64;
    let win: Vec<f32> = (0..N)
        .map(|n| (0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / N as f64).cos()) as f32)
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(N);
    let inv = planner.plan_fft_inverse(N);
    let seg_start = k0 * HOP as i64 - (N / 2) as i64;

    let frame_spectrum = |k: usize| {
        let s0 = seg_start + (k * HOP) as i64;
        let mut buf: Vec<Complex32> = (0..N)
            .map(|n| {
                let i = s0 + n as i64;
                let v = if (0..len).contains(&i) {
                    x[i as usize]
                } else {
                    0.0
                };
                Complex32::new(v * win[n], 0.0)
            })
            .collect();
        fwd.process(&mut buf);
        buf
    };
    let m = |k: usize, j: usize| mask[j * frames + k];
    let active: Vec<bool> = (0..frames)
        .map(|k| (0..BINS).any(|j| m(k, j) > 1e-4))
        .collect();

    // Per-cell gain factor, indexed [frame * BINS + bin].
    let factors: Vec<f32> = match op {
        EditOp::Gain(g) => (0..frames * BINS)
            .map(|i| 1.0 + m(i / BINS, i % BINS) * (g - 1.0))
            .collect(),
        EditOp::Heal => {
            let mags: Vec<f32> = (0..frames)
                .into_par_iter()
                .flat_map_iter(|k| {
                    frame_spectrum(k)[..BINS]
                        .iter()
                        .map(|c| c.norm())
                        .collect::<Vec<_>>()
                })
                .collect();
            let target = heal_targets(&mags, mask, frames);
            (0..frames * BINS)
                .map(|i| {
                    let mi = m(i / BINS, i % BINS);
                    let ratio = (target[i] - (mags[i] + 1e-9).ln()) * mi;
                    ratio.exp().clamp(0.0, 4.0)
                })
                .collect()
        }
    };

    let seg_len = (frames - 1) * HOP + N;
    let mut acc = vec![0.0f32; seg_len];
    let mut wsum = vec![0.0f32; seg_len];
    for chunk_start in (0..frames).step_by(CHUNK) {
        let chunk_end = (chunk_start + CHUNK).min(frames);
        let outputs: Vec<Vec<f32>> = (chunk_start..chunk_end)
            .into_par_iter()
            .map(|k| {
                let s0 = seg_start + (k * HOP) as i64;
                if !active[k] {
                    // Untouched frame: its resynthesis equals the windowed input.
                    return (0..N)
                        .map(|n| {
                            let i = s0 + n as i64;
                            if (0..len).contains(&i) {
                                x[i as usize] * win[n]
                            } else {
                                0.0
                            }
                        })
                        .collect();
                }
                let mut spec = frame_spectrum(k);
                for j in 0..BINS {
                    let f = factors[k * BINS + j];
                    spec[j] *= f;
                    if j > 0 && j < N / 2 {
                        spec[N - j] *= f; // keep the spectrum Hermitian
                    }
                }
                inv.process(&mut spec);
                spec.iter().map(|c| c.re / N as f32).collect()
            })
            .collect();
        for (k, y) in (chunk_start..chunk_end).zip(outputs) {
            let off = k * HOP;
            for n in 0..N {
                acc[off + n] += win[n] * y[n];
                wsum[off + n] += win[n] * win[n];
            }
        }
    }

    let mut out = x.to_vec();
    for s in write_a.max(0)..write_b.min(len) {
        let i = (s - seg_start) as usize;
        if wsum[i] > 1e-3 {
            out[s as usize] = acc[i] / wsum[i];
        }
    }
    out
}

/// Log-magnitude targets for the masked cells, interpolated across each gap in time and
/// in frequency; the shorter gap gets more weight. Returns ln-magnitudes `[frame * BINS + bin]`.
fn heal_targets(mags: &[f32], mask: &[f32], frames: usize) -> Vec<f32> {
    let ln: Vec<f32> = mags.iter().map(|m| (m + 1e-9).ln()).collect();
    let hole = |k: usize, j: usize| mask[j * frames + k] > 0.02;
    // (sum of value/len, sum of 1/len) per cell
    let mut num = vec![0.0f32; ln.len()];
    let mut den = vec![0.0f32; ln.len()];

    // Fills one gap [s, e) along a line, given the line's values and its source cells.
    let mut fill =
        |cells: &dyn Fn(usize) -> usize, count: usize, is_hole: &dyn Fn(usize) -> bool| {
            let mut i = 0;
            while i < count {
                if !is_hole(i) {
                    i += 1;
                    continue;
                }
                let s = i;
                while i < count && is_hole(i) {
                    i += 1;
                }
                let e = i;
                let left = (s > 0).then(|| ln[cells(s - 1)]);
                let right = (e < count).then(|| ln[cells(e)]);
                let weight = 1.0 / (e - s) as f32;
                for p in s..e {
                    let v = match (left, right) {
                        (Some(l), Some(r)) => l + (r - l) * (p - s + 1) as f32 / (e - s + 1) as f32,
                        (Some(v), None) | (None, Some(v)) => v,
                        (None, None) => continue,
                    };
                    num[cells(p)] += v * weight;
                    den[cells(p)] += weight;
                }
            }
        };
    for j in 0..BINS {
        fill(&|k| k * BINS + j, frames, &|k| hole(k, j));
    }
    for k in 0..frames {
        fill(&|j| k * BINS + j, BINS, &|j| hole(k, j));
    }

    ln.iter()
        .zip(num.iter().zip(&den))
        .map(|(&orig, (&n, &d))| if d > 0.0 { n / d } else { orig })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PasteMode {
    /// Shift the audio after the paste position to make room.
    Insert,
    /// Replace the audio after the paste position (extending the file if needed).
    Overwrite,
}

/// Copies the time range `t0..t1` (seconds) of all channels.
pub fn extract(audio: &AudioData, t0: f64, t1: f64) -> AudioData {
    let sr = audio.sample_rate as f64;
    let n = audio.num_frames();
    let a = ((t0 * sr).round().max(0.0) as usize).min(n);
    let b = ((t1 * sr).round().max(0.0) as usize).clamp(a, n);
    AudioData {
        sample_rate: audio.sample_rate,
        channels: audio.channels.iter().map(|c| c[a..b].to_vec()).collect(),
    }
}

/// Pastes `clip` at time `at` (seconds). Returns the new audio and the pasted range in seconds.
/// Channels are matched by index; a clip with fewer channels repeats its last channel.
pub fn paste(
    dest: &AudioData,
    clip: &AudioData,
    at: f64,
    mode: PasteMode,
) -> Result<(AudioData, (f64, f64)), String> {
    if clip.sample_rate != dest.sample_rate {
        return Err(format!(
            "Cannot paste: the clip is {} Hz but this file is {} Hz",
            clip.sample_rate, dest.sample_rate
        ));
    }
    let clip_len = clip.num_frames();
    if clip_len == 0 || clip.channels.is_empty() {
        return Err("The clipboard is empty".into());
    }
    let len = dest.num_frames();
    let pos = ((at * dest.sample_rate as f64).round().max(0.0) as usize).min(len);
    let channels = dest
        .channels
        .iter()
        .enumerate()
        .map(|(c, ch)| {
            let src = &clip.channels[c.min(clip.channels.len() - 1)];
            match mode {
                PasteMode::Insert => {
                    let mut out = Vec::with_capacity(len + clip_len);
                    out.extend_from_slice(&ch[..pos]);
                    out.extend_from_slice(src);
                    out.extend_from_slice(&ch[pos..]);
                    out
                }
                PasteMode::Overwrite => {
                    let mut out = ch.clone();
                    out.resize(len.max(pos + clip_len), 0.0);
                    out[pos..pos + clip_len].copy_from_slice(src);
                    out
                }
            }
        })
        .collect();
    let sr = dest.sample_rate as f64;
    let range = (pos as f64 / sr, (pos + clip_len) as f64 / sr);
    Ok((
        AudioData {
            sample_rate: dest.sample_rate,
            channels,
        },
        range,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Shape;

    #[test]
    fn copy_then_paste_insert_and_overwrite() {
        let audio = AudioData {
            sample_rate: 10,
            channels: vec![(0..10).map(|i| i as f32).collect(), vec![0.5; 10]],
        };
        let clip = extract(&audio, 0.2, 0.5); // samples 2, 3, 4
        assert_eq!(clip.channels[0], vec![2.0, 3.0, 4.0]);

        let (ins, range) = paste(&audio, &clip, 0.8, PasteMode::Insert).unwrap();
        assert_eq!(range, (0.8, 1.1));
        assert_eq!(
            ins.channels[0],
            vec![
                0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 2.0, 3.0, 4.0, 8.0, 9.0
            ]
        );
        assert_eq!(ins.channels[1].len(), 13);

        let (over, _) = paste(&audio, &clip, 0.1, PasteMode::Overwrite).unwrap();
        assert_eq!(
            over.channels[0],
            vec![0.0, 2.0, 3.0, 4.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );
        // Overwriting past the end extends the file.
        let (ext, _) = paste(&audio, &clip, 0.9, PasteMode::Overwrite).unwrap();
        assert_eq!(&ext.channels[0][8..], &[8.0, 2.0, 3.0, 4.0]);

        let other_rate = AudioData {
            sample_rate: 20,
            channels: clip.channels.clone(),
        };
        assert!(paste(&audio, &other_rate, 0.0, PasteMode::Insert).is_err());
    }

    fn tone(sr: u32, secs: f64, freqs: &[f32]) -> Vec<f32> {
        (0..(sr as f64 * secs) as usize)
            .map(|i| {
                let t = i as f32 / sr as f32;
                freqs
                    .iter()
                    .map(|f| 0.3 * (std::f32::consts::TAU * f * t).sin())
                    .sum()
            })
            .collect()
    }

    /// RMS of a single frequency component over `range` (in samples).
    fn level(x: &[f32], sr: u32, f: f32, range: std::ops::Range<usize>) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for i in range.clone() {
            let ph = std::f64::consts::TAU * f as f64 * i as f64 / sr as f64;
            re += x[i] as f64 * ph.cos();
            im += x[i] as f64 * ph.sin();
        }
        (2.0 * (re * re + im * im).sqrt() / range.len() as f64) as f32
    }

    #[test]
    fn mute_removes_only_the_selected_tone_and_time() {
        let sr = 44100;
        let audio = AudioData {
            sample_rate: sr,
            channels: vec![tone(sr, 3.0, &[1000.0, 5000.0])],
        };
        let mut sel = Selection::default();
        sel.push(Shape::rect(1.0, 2.0, 4000.0, 6000.0), true);
        let out = apply(&audio, &sel, EditOp::Gain(0.0), &[0]);
        let y = &out.channels[0];

        let inside = 50_000..85_000; // within 1.0 .. 2.0 s
        assert!(
            level(y, sr, 5000.0, inside.clone()) < 0.003,
            "5 kHz not muted"
        );
        assert!(
            (level(y, sr, 1000.0, inside) - 0.3).abs() < 0.02,
            "1 kHz changed"
        );
        // Far outside the selection the audio is bit-for-bit identical.
        assert_eq!(&y[..30_000], &audio.channels[0][..30_000]);
        assert_eq!(&y[120_000..], &audio.channels[0][120_000..]);
    }

    #[test]
    fn heal_removes_a_short_click() {
        let sr = 44100;
        let mut x = tone(sr, 2.0, &[440.0]);
        for v in &mut x[44100..44120] {
            *v += 0.9;
        }
        let audio = AudioData {
            sample_rate: sr,
            channels: vec![x],
        };
        let mut sel = Selection::default();
        sel.push(Shape::rect(0.97, 1.03, 0.0, 22050.0), true);
        let out = apply(&audio, &sel, EditOp::Heal, &[0]);
        let peak = |c: &[f32]| c[43500..44700].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak(&audio.channels[0]) > 1.0);
        assert!(
            peak(&out.channels[0]) < 0.6,
            "click still at {}",
            peak(&out.channels[0])
        );
    }
}
