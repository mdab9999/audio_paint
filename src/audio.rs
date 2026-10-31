//! Audio file decoding (MP3, WAV, FLAC, OGG, AAC/M4A) via Symphonia.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Fully decoded audio, one `Vec<f32>` per channel.
pub struct AudioData {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioData {
    pub fn num_frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn duration(&self) -> f64 {
        self.num_frames() as f64 / self.sample_rate as f64
    }

    /// Returns the samples of one channel, or the mono mix for `None`.
    pub fn channel_or_mix(&self, channel: Option<usize>) -> Vec<f32> {
        match channel {
            Some(c) => self.channels[c].clone(),
            None => {
                let n = self.channels.len() as f32;
                (0..self.num_frames())
                    .map(|i| self.channels.iter().map(|ch| ch[i]).sum::<f32>() / n)
                    .collect()
            }
        }
    }
}

/// Decodes the whole file. `progress` receives 0..=1000 (per mille) when the length is known.
pub fn decode_file(path: &Path, progress: &AtomicU32) -> Result<AudioData, String> {
    let file = File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("Unsupported format: {e}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("No audio track found")?;
    let track_id = track.id;
    let total_frames = track.num_frames;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("Missing audio codec parameters")?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("Unsupported codec: {e}"))?;

    let mut sample_rate = 0;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut planes: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A track list change (e.g. chained OGG streams) is treated as end of stream.
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(format!("Read error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(buf) => {
                sample_rate = buf.spec().rate();
                buf.copy_to_vecs_planar(&mut planes);
                if channels.is_empty() {
                    channels.resize(planes.len(), Vec::new());
                }
                for (dst, src) in channels.iter_mut().zip(&planes) {
                    dst.extend_from_slice(src);
                }
            }
            // Skip corrupt packets, as most players do.
            Err(Error::DecodeError(_)) | Err(Error::IoError(_)) => continue,
            Err(e) => return Err(format!("Decode error: {e}")),
        }

        if let (Some(total), Some(ch)) = (total_frames, channels.first()) {
            let p = (ch.len() as f64 / total.max(1) as f64 * 1000.0).min(1000.0);
            progress.store(p as u32, Ordering::Relaxed);
        }
    }

    if channels.is_empty() || channels[0].is_empty() || sample_rate == 0 {
        return Err("The file contains no audio samples".into());
    }
    Ok(AudioData {
        sample_rate,
        channels,
    })
}

/// Output formats for "Save as".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveFormat {
    Wav16,
    Wav24,
    WavFloat,
    /// Constant bit rate in kbit/s.
    Mp3(u32),
}

impl SaveFormat {
    pub const MP3_BITRATES: [u32; 5] = [128, 160, 192, 256, 320];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp3(_) => "mp3",
            _ => "wav",
        }
    }

    pub fn name(self) -> String {
        match self {
            Self::Wav16 => "WAV 16-bit PCM".into(),
            Self::Wav24 => "WAV 24-bit PCM".into(),
            Self::WavFloat => "WAV 32-bit float".into(),
            Self::Mp3(kbps) => format!("MP3 {kbps} kbit/s"),
        }
    }
}

pub fn save(path: &Path, audio: &AudioData, format: SaveFormat) -> Result<(), String> {
    let result = match format {
        SaveFormat::Wav16 => write_wav(path, audio, 16),
        SaveFormat::Wav24 => write_wav(path, audio, 24),
        SaveFormat::WavFloat => write_wav(path, audio, 32),
        SaveFormat::Mp3(kbps) => write_mp3(path, audio, kbps),
    };
    result.map_err(|e| format!("Cannot save {}: {e}", path.display()))
}

/// Writes PCM (16 or 24 bit, clipped to full scale) or, for `bits == 32`, IEEE float.
fn write_wav(path: &Path, audio: &AudioData, bits: u16) -> Result<(), String> {
    use std::io::{BufWriter, Write};

    let channels = audio.channels.len() as u16;
    let frames = audio.num_frames();
    let bytes = bits / 8;
    let data_len = frames as u64 * channels as u64 * bytes as u64;
    if data_len > u32::MAX as u64 - 64 {
        return Err("audio is too long for a WAV file".into());
    }
    let block_align = channels * bytes;
    let format_tag: u16 = if bits == 32 { 3 } else { 1 }; // 3 = IEEE float, 1 = PCM

    let write = || -> std::io::Result<()> {
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(b"RIFF")?;
        w.write_all(&(36 + data_len as u32).to_le_bytes())?;
        w.write_all(b"WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&format_tag.to_le_bytes())?;
        w.write_all(&channels.to_le_bytes())?;
        w.write_all(&audio.sample_rate.to_le_bytes())?;
        w.write_all(&(audio.sample_rate * block_align as u32).to_le_bytes())?;
        w.write_all(&block_align.to_le_bytes())?;
        w.write_all(&bits.to_le_bytes())?;
        w.write_all(b"data")?;
        w.write_all(&(data_len as u32).to_le_bytes())?;
        for i in 0..frames {
            for ch in &audio.channels {
                let s = ch[i];
                match bits {
                    16 => {
                        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                        w.write_all(&v.to_le_bytes())?;
                    }
                    24 => {
                        let v = (s.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                        w.write_all(&v.to_le_bytes()[..3])?;
                    }
                    _ => w.write_all(&s.to_le_bytes())?,
                }
            }
        }
        w.flush()
    };
    write().map_err(|e| e.to_string())
}

/// Encodes with LAME at a constant bit rate. More than two channels are mixed down to stereo;
/// sample rates above 48 kHz are resampled to 48 kHz by the encoder.
fn write_mp3(path: &Path, audio: &AudioData, kbps: u32) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, MonoPcm, Quality};

    let bitrate = match kbps {
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        256 => Bitrate::Kbps256,
        _ => Bitrate::Kbps320,
    };
    let stereo: Option<(Vec<f32>, Vec<f32>)> = match audio.channels.len() {
        1 => None,
        2 => Some((audio.channels[0].clone(), audio.channels[1].clone())),
        _ => {
            // Even-numbered channels to the left, odd-numbered to the right.
            let mix = |parity: usize| -> Vec<f32> {
                let picked: Vec<&Vec<f32>> =
                    audio.channels.iter().skip(parity).step_by(2).collect();
                (0..audio.num_frames())
                    .map(|i| picked.iter().map(|c| c[i]).sum::<f32>() / picked.len() as f32)
                    .collect()
            };
            Some((mix(0), mix(1)))
        }
    };

    let err = |e: &dyn std::fmt::Display| format!("MP3 encoder: {e}");
    let mut builder = Builder::new().ok_or("cannot create the MP3 encoder")?;
    builder
        .set_num_channels(if stereo.is_some() { 2 } else { 1 })
        .map_err(|e| err(&e))?;
    builder
        .set_sample_rate(audio.sample_rate)
        .map_err(|e| err(&e))?;
    if audio.sample_rate > 48_000 {
        builder
            .set_output_sample_rate(std::num::NonZeroU32::new(48_000))
            .map_err(|e| err(&e))?;
    }
    builder.set_brate(bitrate).map_err(|e| err(&e))?;
    builder.set_quality(Quality::Best).map_err(|e| err(&e))?;
    let mut encoder = builder.build().map_err(|e| err(&e))?;

    const CHUNK: usize = 1 << 16;
    let mut out = Vec::new();
    for start in (0..audio.num_frames()).step_by(CHUNK) {
        let end = (start + CHUNK).min(audio.num_frames());
        out.reserve(mp3lame_encoder::max_required_buffer_size(end - start));
        match &stereo {
            Some((l, r)) => encoder.encode_to_vec(
                DualPcm {
                    left: &l[start..end],
                    right: &r[start..end],
                },
                &mut out,
            ),
            None => encoder.encode_to_vec(MonoPcm(&audio.channels[0][start..end]), &mut out),
        }
        .map_err(|e| err(&e))?;
    }
    out.reserve(7200); // LAME's documented maximum for the final flush
    encoder
        .flush_to_vec::<FlushNoGap>(&mut out)
        .map_err(|e| err(&e))?;
    std::fs::write(path, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("audio_paint_test_{name}"))
    }

    fn sine(sr: u32, secs: f32, freq: f32) -> Vec<f32> {
        (0..(sr as f32 * secs) as usize)
            .map(|i| 0.5 * (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn wav_float_round_trip_is_exact() {
        let audio = AudioData {
            sample_rate: 22050,
            channels: vec![vec![0.0, 0.5, -0.25, 1.0], vec![0.1, -0.1, 0.2, -0.2]],
        };
        let path = temp("float.wav");
        save(&path, &audio, SaveFormat::WavFloat).unwrap();
        let back = decode_file(&path, &AtomicU32::new(0)).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.sample_rate, 22050);
        assert_eq!(back.channels, audio.channels);
    }

    #[test]
    fn wav_pcm_round_trips_within_quantization() {
        let audio = AudioData {
            sample_rate: 44100,
            channels: vec![sine(44100, 0.1, 440.0), sine(44100, 0.1, 880.0)],
        };
        for (format, tolerance) in [
            (SaveFormat::Wav16, 1.0 / 32767.0),
            (SaveFormat::Wav24, 1.0 / 8e6),
        ] {
            let path = temp("pcm.wav");
            save(&path, &audio, format).unwrap();
            let back = decode_file(&path, &AtomicU32::new(0)).unwrap();
            std::fs::remove_file(&path).ok();
            assert_eq!(back.channels.len(), 2);
            let max_err = back.channels[1]
                .iter()
                .zip(&audio.channels[1])
                .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
            assert!(max_err <= tolerance, "{format:?}: error {max_err}");
        }
    }

    #[test]
    fn mp3_encodes_and_decodes() {
        let sr = 44100;
        let audio = AudioData {
            sample_rate: sr,
            channels: vec![sine(sr, 1.0, 1000.0), sine(sr, 1.0, 1000.0)],
        };
        let path = temp("tone.mp3");
        save(&path, &audio, SaveFormat::Mp3(192)).unwrap();
        let back = decode_file(&path, &AtomicU32::new(0)).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.sample_rate, sr);
        assert_eq!(back.channels.len(), 2);
        // MP3 adds encoder delay and padding, so allow a little extra length.
        let secs = back.duration();
        assert!((0.99..1.2).contains(&secs), "duration {secs}");
        let peak = back.channels[0].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.4..0.6).contains(&peak), "peak {peak}");
    }
}
