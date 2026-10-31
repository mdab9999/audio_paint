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

fn main() -> eframe::Result {
    // An audio file can be passed on the command line (or dropped onto the .exe).
    let initial_file = std::env::args_os().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Audio Paint")
            .with_inner_size([1100.0, 680.0])
            .with_maximized(true)
            .with_min_inner_size([640.0, 420.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Audio Paint",
        options,
        Box::new(|cc| Ok(Box::new(app::AudioPaintApp::new(cc, initial_file)))),
    )
}
