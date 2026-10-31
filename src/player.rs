//! Audio playback through the default output device (via rodio).

use std::num::NonZero;

use rodio::buffer::SamplesBuffer;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player as RodioPlayer};

use crate::audio::AudioData;

#[derive(Default)]
pub struct Player {
    /// Opened on first use, so the app also works without an audio device.
    device: Option<MixerDeviceSink>,
    current: Option<RodioPlayer>,
    start_t: f64,
}

impl Player {
    /// Plays the time range `t0..t1` (seconds) of all channels.
    pub fn play(&mut self, audio: &AudioData, t0: f64, t1: f64) -> Result<(), String> {
        self.stop();
        if self.device.is_none() {
            let mut device = DeviceSinkBuilder::open_default_sink()
                .map_err(|e| format!("Cannot open audio output: {e}"))?;
            device.log_on_drop(false);
            self.device = Some(device);
        }
        let device = self.device.as_ref().unwrap();

        let sr = audio.sample_rate as f64;
        let n = audio.num_frames();
        let a = ((t0 * sr).round().max(0.0) as usize).min(n);
        let b = ((t1 * sr).round().max(0.0) as usize).clamp(a, n);
        if a == b {
            return Ok(());
        }
        let channels = audio.channels.len();
        let mut data = Vec::with_capacity((b - a) * channels);
        for i in a..b {
            data.extend(audio.channels.iter().map(|ch| ch[i]));
        }

        let player = RodioPlayer::connect_new(device.mixer());
        player.append(SamplesBuffer::new(
            NonZero::new(channels as u16).ok_or("No channels")?,
            NonZero::new(audio.sample_rate).ok_or("Invalid sample rate")?,
            data,
        ));
        self.current = Some(player);
        self.start_t = a as f64 / sr;
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(player) = self.current.take() {
            player.stop();
        }
    }

    /// Pauses or resumes; does nothing when nothing is playing.
    pub fn set_paused(&mut self, paused: bool) {
        if let Some(player) = &self.current {
            if paused {
                player.pause();
            } else {
                player.play();
            }
        }
    }

    pub fn is_paused(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|p| p.is_paused() && !p.empty())
    }

    /// Current playback position in seconds (also while paused), or `None` when stopped.
    pub fn position(&self) -> Option<f64> {
        let player = self.current.as_ref()?;
        (!player.empty()).then(|| self.start_t + player.get_pos().as_secs_f64())
    }

    /// Playing and not paused.
    pub fn is_playing(&self) -> bool {
        self.position().is_some() && !self.is_paused()
    }
}
