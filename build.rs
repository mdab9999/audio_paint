//! Maintains the build number and embeds the icon and version info into the Windows executable.
//!
//! Every release build that compiles changed sources increments `build_number.txt`. Debug builds
//! (and the background checks of editors such as rust-analyzer) reuse the current number.
//! The number is available to the program as `env!("AUDIO_PAINT_BUILD")`.

use std::fs;

const BUILD_NUMBER_FILE: &str = "build_number.txt";

fn main() {
    // Rerun (and so count a new build) only when an input of the program changes. The counter
    // file itself is deliberately not listed, otherwise every build would trigger the next one.
    for path in ["src", "assets/icon", "Cargo.toml", "Cargo.lock", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
    }

    let current: u32 = fs::read_to_string(BUILD_NUMBER_FILE)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let build = if release {
        let next = current + 1;
        fs::write(BUILD_NUMBER_FILE, format!("{next}\n")).expect("cannot update build_number.txt");
        next
    } else {
        current
    };
    println!("cargo:rustc-env=AUDIO_PAINT_BUILD={build}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap();
        let part = |name: &str| -> u64 { std::env::var(name).unwrap().parse().unwrap_or(0) };
        let packed = part("CARGO_PKG_VERSION_MAJOR") << 48
            | part("CARGO_PKG_VERSION_MINOR") << 32
            | part("CARGO_PKG_VERSION_PATCH") << 16
            | build as u64 & 0xFFFF;

        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/audio_paint.ico");
        res.set("FileDescription", "Audio Paint");
        res.set("ProductName", "Audio Paint");
        res.set("FileVersion", &format!("{version}.{build}"));
        res.set("ProductVersion", &format!("{version}.{build}"));
        res.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
        res.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
        res.compile()
            .expect("failed to embed the Windows icon resource");
    }
}
