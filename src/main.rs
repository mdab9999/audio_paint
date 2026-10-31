// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod colormap;
mod edit;
mod player;
mod selection;
mod spectrum;

use std::path::PathBuf;

use eframe::egui;

/// "Audio Paint <version>.<build>", e.g. "Audio Paint 0.1.0.7".
pub const APP_TITLE: &str = concat!(
    "Audio Paint ",
    env!("CARGO_PKG_VERSION"),
    ".",
    env!("AUDIO_PAINT_BUILD")
);

fn main() -> eframe::Result {
    // Audio files can be passed on the command line (or dropped onto the .exe).
    let initial_files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([1100.0, 680.0])
            .with_maximized(true)
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/icon/png/audio_paint_256.png"
                ))
                .expect("embedded icon is a valid PNG"),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Audio Paint",
        options,
        Box::new(|cc| Ok(Box::new(app::AudioPaintApp::new(cc, initial_files)))),
    )
}
