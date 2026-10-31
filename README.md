# Audio Paint

A desktop app in Rust that shows audio files as a spectral "waterfall" display, similar to the
Spectral Frequency Display in Adobe Audition. You can zoom and pan freely in time and frequency.

Built with [egui/eframe](https://github.com/emilk/egui) for the UI,
[Symphonia](https://github.com/pdeljanov/Symphonia) for decoding and
[RustFFT](https://github.com/ejmahler/RustFFT) for the analysis.

## Features

- Opens **MP3, WAV**, and also FLAC, OGG/Vorbis and AAC/M4A
  (open dialog, drag and drop, or a file passed on the command line)
- Spectrogram (short-time FFT) with selectable FFT size (256 to 16384) and window
  (Blackman-Harris, Hann, Hamming), 75 % overlap
- Linear or logarithmic frequency scale
- Colour maps: Spectral (Audition-like), Viridis, Grayscale; adjustable dB floor and ceiling
- Mono mix or a single channel
- Waveform lane and overview strip, all synchronised with the spectral view
- Live cursor readout: time, frequency and level in dB
- Analysis runs in the background on all CPU cores; zoomed-out views use a max-pooled
  pyramid so short transients stay visible

## Controls

| Action | Input |
|---|---|
| Zoom time | Mouse wheel |
| Zoom frequency | Ctrl + mouse wheel (or pinch) |
| Pan time | Shift + wheel, drag, or arrow keys |
| Pan frequency | Alt + wheel, or drag vertically |
| Zoom to a region | Right-drag a box |
| Jump to a position | Click or drag in the overview strip |
| Zoom to fit | Double-click, Home or F |
| Zoom time in/out | + / - |

## Build and run

Requires the Rust toolchain (on Windows with the MSVC build tools).

```
cargo run --release -- path\to\file.mp3
```

The release executable is written to `target/release/audio_paint.exe`.
