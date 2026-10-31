# Audio Paint

A desktop app in Rust that shows audio files as a spectral "waterfall" display, similar to the
Spectral Frequency Display in Adobe Audition, and lets you select and edit areas of the
spectrum: mute or lower a noise, or heal a click from the surrounding audio.

Built with [egui/eframe](https://github.com/emilk/egui) for the UI,
[Symphonia](https://github.com/pdeljanov/Symphonia) for decoding,
[RustFFT](https://github.com/ejmahler/RustFFT) for analysis and processing and
[rodio](https://github.com/RustAudio/rodio) for playback.

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
- Time selection in the waveform, shown across all lanes and the overview
- Playback of the whole file, the selection, or from the cursor, with a moving play cursor
  that the view can follow
- **Spectral selection tools**: marquee, lasso, paintbrush, and a spot healing brush
- **Spectral editing**: mute the selection, change its gain, or heal it; undo/redo
- **Export** the edited audio as a 32-bit float WAV
- Analysis and editing run in the background on all CPU cores

## Spectral editing

| Tool | Key | Use |
|---|---|---|
| Hand | H | Pan (drag) and zoom (double-click to fit) |
| Marquee | E | Drag a rectangular time-frequency block |
| Lasso | D | Draw a freehand shape around a noise |
| Paintbrush | B | Paint a selection; strokes add up, Alt + paint erases |
| Spot healing brush | S | Paint over a blemish; it is repaired when you release the mouse |

- Shift adds to the selection and Alt subtracts from it. A click with marquee or lasso clears
  the spectral selection; Esc clears all selections.
- `[` and `]` change the brush size.
- With a selection, the floating panel next to it sets a **gain** in dB ("Apply gain"),
  **mutes** it (or press Delete) or **heals** it. Without a spectral selection the same
  actions apply to the time selection across all frequencies.
- Edits apply to all channels when the display shows the mix, or only to the displayed
  channel. Ctrl+Z / Ctrl+Y undo and redo.
- Processing uses an STFT (2048-point Hann, 75 % overlap) with a slightly feathered mask.
  Audio outside the selected area is left bit-for-bit unchanged.
- Healing interpolates the level across the painted area in time and frequency. It works
  best on short, broadband blemishes such as clicks. Tones that move in pitch through the
  painted area (sweeps, vibrato) can get a gap there.

## Other controls

| Action | Input |
|---|---|
| Select a time range | Drag in the waveform |
| Resize the selection | Drag one of its edges |
| Extend the selection | Shift + click or Shift + drag |
| Deselect and set the cursor | Click in the waveform, or Esc / "Deselect" |
| Select all | Double-click the waveform, Ctrl+A or "Select all" |
| Play selection (or from cursor) / stop | Space, or "Play" / "Stop" |
| Play the whole file | "Play all" |
| Zoom time | Mouse wheel |
| Zoom frequency | Ctrl + mouse wheel (or pinch) |
| Pan time | Shift + wheel, hand tool drag, middle-drag, or arrow keys |
| Pan frequency | Alt + wheel, or hand tool drag |
| Zoom to a region | Right-drag a box |
| Jump to a position | Click or drag in the overview strip |
| Zoom to fit | Double-click with the hand tool, Home or F |
| Zoom time in/out | + / - |

## Build and run

Requires the Rust toolchain (on Windows with the MSVC build tools).

```
cargo run --release -- path\to\file.mp3
```

The release executable is written to `target/release/audio_paint.exe`.
