# Audio Paint

A desktop app in Rust that shows audio files as a spectral "waterfall" display, similar to the
Spectral Frequency Display in Adobe Audition, and lets you select and edit areas of the
spectrum: mute or lower a noise, or heal a click from the surrounding audio.

Built with [egui/eframe](https://github.com/emilk/egui) for the UI,
[Symphonia](https://github.com/pdeljanov/Symphonia) for decoding,
[RustFFT](https://github.com/ejmahler/RustFFT) for analysis and processing and
[rodio](https://github.com/RustAudio/rodio) for playback and
[LAME](https://lame.sourceforge.io/) (via `mp3lame-encoder`, LGPL) for MP3 encoding.

## Layout

The window follows the layout of Adobe Audition's Waveform editor:

- **Menu bar**: File, Edit, Effects, View, Window, Help
- **Tool bar**: Time Selection, Marquee, Lasso, Paintbrush, Spot Healing and Hand tools, and
  toggles for the waveform and the spectral frequency display
- **Files** panel with all open files, and **History** / **Effects** panels below it
- **Editor**: zoom navigator, one waveform lane per channel with a dB ruler, the spectral
  frequency display with a Hz ruler, the time ruler and the red current-time indicator
- **Transport and zoom** controls with a large time display
- **Levels** meter during playback and **Selection/View** fields that can be typed into
- **Status bar** with the playback state and the file's format, size and duration

## Features

- Opens **MP3, WAV**, and also FLAC, OGG/Vorbis and AAC/M4A, several files at once
  (File > Open, drag and drop, or files passed on the command line)
- Spectrogram (short-time FFT) with selectable FFT size (256 to 16384, default 4096) and window
  (Blackman-Harris, Hann, Hamming), 75 % overlap
- Logarithmic frequency scale by default (more detail in the low frequencies), or linear
- Colour maps: Spectral (Audition-like), Viridis, Grayscale; adjustable dB floor and ceiling
- Spectral display of the mono mix or a single channel (View > Spectral Display Settings)
- Waveform lane per channel with amplitude zoom, and a zoom navigator
- Live cursor readout: time, frequency and level in dB
- Time selection in the waveform, shown across all lanes and the overview
- Playback of the selection or from the cursor, with pause, loop and a playhead the view
  can follow; level meters with peak hold
- **Spectral selection tools**: marquee, lasso, paintbrush, and a spot healing brush
- **Spectral editing**: mute the selection, change its gain, or heal it
- **Effects**: Amplify, Normalize, Fade In, Fade Out, Silence, Invert, Reverse (on the time
  selection, or the whole file without one)
- **History** of every edit; click an entry to go back to it (or Ctrl+Z / Ctrl+Y)
- **Cut, copy and paste** of the selected time range, inserted or overwriting at the cursor
- Asks to save unsaved changes when a file is closed or the app exits
- **Save as** WAV (16-bit, 24-bit or 32-bit float) or MP3 (128 to 320 kbit/s)
- Analysis and editing run in the background on all CPU cores

## Cut, copy, paste and save

- **Cut** (Ctrl+X) copies the time selection to the clipboard and removes it from the file;
  the following audio moves up. It can be undone like any other edit.
- **Copy** (Ctrl+C) copies the time selection, or the time range of the spectral selection,
  from all channels. The clipboard is kept when you open another file, so you can paste
  between files with the same sample rate.
- **Paste insert** (Ctrl+V) inserts the clip at the cursor and moves the following audio back.
- **Paste overwrite** (Ctrl+Shift+V) replaces the audio from the cursor on, extending the
  file if the clip runs past the end.
- The pasted audio is selected afterwards, and every paste can be undone.
- Closing a file with unsaved changes, or closing the app, asks **Save As... / Don't Save /
  Cancel** for each unsaved file; the file closes (or the app exits) once it is saved.
- **Save as** (Ctrl+S) writes WAV 16-bit PCM, WAV 24-bit PCM, WAV 32-bit float (lossless,
  keeps levels above 0 dBFS) or MP3 at 128 to 320 kbit/s (encoded with LAME; more than two
  channels are mixed down to stereo, sample rates above 48 kHz become 48 kHz).

## Spectral editing

| Tool | Key | Use |
|---|---|---|
| Time Selection | T | Drag to select a time range (also in the spectral display) |
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
| Open / Save As / Close | Ctrl+O / Ctrl+S / Ctrl+W |
| Select a time range | Drag in the waveform (or the spectral display with the Time tool) |
| Resize the selection | Drag one of its edges |
| Extend the selection | Shift + click or Shift + drag |
| Deselect and set the playhead | Click in the waveform, or Esc |
| Select all | Double-click the waveform, or Ctrl+A |
| Play / stop | Space, or the transport buttons (also Pause and Loop) |
| Move playhead to start / end | Home / End, or the transport buttons |
| Zoom time | Mouse wheel, + / -, or the zoom buttons |
| Zoom frequency | Ctrl + mouse wheel on the spectral display |
| Zoom amplitude | Ctrl + mouse wheel on the waveform, or the zoom buttons |
| Scroll time | Shift + wheel, Hand tool, middle-drag, or arrow keys |
| Scroll frequency | Alt + wheel, or the Hand tool |
| Zoom to a region | Right-drag a box |
| Jump to a position | Click or drag in the zoom navigator |
| Zoom out full | F, or double-click the navigator |
| Set selection or view numerically | Type or drag in the Selection/View panel |

## Build and run

Requires the Rust toolchain (on Windows with the MSVC build tools).

```
cargo run --release -- path\to\file.mp3
```

The release executable is written to `target/release/audio_paint.exe`. On Windows,
`build.rs` embeds `assets/icon/audio_paint.ico` as the program icon; the window and
taskbar icon is loaded from `assets/icon/png/audio_paint_256.png`.

## Windows installer

```
powershell -ExecutionPolicy Bypass -File installer\build_installer.ps1
```

This builds the release executable and `dist\AudioPaint-<version>.<build>-setup.exe`. It needs [Inno Setup 6](https://jrsoftware.org/isinfo.php)
(`winget install JRSoftware.InnoSetup`).

The installer:

- installs for the current user without admin rights, or for all users if chosen
- adds a Start menu entry, and optionally a desktop icon
- optionally adds "Open with Audio Paint" for MP3, WAV, FLAC, OGG, M4A and AAC files
  (without changing the default player)
- registers an uninstaller in Windows "Apps & features"
- is available in English and German

The executable links the C runtime statically (`.cargo/config.toml`), so it runs on PCs
without the Visual C++ Redistributable. The installer is not code-signed, so Windows
SmartScreen may warn when it is first run ("More info" -> "Run anyway").

## Version and build number

The version is `<version from Cargo.toml>.<build number>`, for example `0.1.0.2`. It is shown
in the window title, in the file properties of `audio_paint.exe` and in the installer.

`build.rs` increments `build_number.txt` on every **release** build that compiles changed
sources (`cargo build --release`, `installeruild_installer.ps1`). Debug builds and editor
checks reuse the current number, and a release build without changes keeps it. Commit
`build_number.txt` together with the code it belongs to.
