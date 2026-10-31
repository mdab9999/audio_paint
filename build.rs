//! Embeds the application icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/audio_paint.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/audio_paint.ico");
        res.set("FileDescription", "Audio Paint");
        res.set("ProductName", "Audio Paint");
        res.compile()
            .expect("failed to embed the Windows icon resource");
    }
}
