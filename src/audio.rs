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

/// Writes the audio as a 32-bit float WAV file (lossless for the edited samples).
pub fn write_wav(path: &Path, audio: &AudioData) -> Result<(), String> {
    use std::io::{BufWriter, Write};

    let channels = audio.channels.len() as u16;
    let frames = audio.num_frames();
    let data_len = frames as u64 * channels as u64 * 4;
    if data_len > u32::MAX as u64 - 64 {
        return Err("Audio is too long for a WAV file".into());
    }
    let byte_rate = audio.sample_rate * channels as u32 * 4;

    let write = || -> std::io::Result<()> {
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(b"RIFF")?;
        w.write_all(&(36 + data_len as u32).to_le_bytes())?;
        w.write_all(b"WAVEfmt ")?;
        w.write_all(&16u32.to_le_bytes())?;
        w.write_all(&3u16.to_le_bytes())?; // IEEE float
        w.write_all(&channels.to_le_bytes())?;
        w.write_all(&audio.sample_rate.to_le_bytes())?;
        w.write_all(&byte_rate.to_le_bytes())?;
        w.write_all(&(channels * 4).to_le_bytes())?;
        w.write_all(&32u16.to_le_bytes())?;
        w.write_all(b"data")?;
        w.write_all(&(data_len as u32).to_le_bytes())?;
        for i in 0..frames {
            for ch in &audio.channels {
                w.write_all(&ch[i].to_le_bytes())?;
            }
        }
        w.flush()
    };
    write().map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip() {
        let audio = AudioData {
            sample_rate: 22050,
            channels: vec![vec![0.0, 0.5, -0.25, 1.0], vec![0.1, -0.1, 0.2, -0.2]],
        };
        let path = std::env::temp_dir().join("audio_paint_round_trip.wav");
        write_wav(&path, &audio).unwrap();
        let back = decode_file(&path, &AtomicU32::new(0)).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(back.sample_rate, 22050);
        assert_eq!(back.channels, audio.channels);
    }
}
