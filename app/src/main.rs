#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    // Optional argument: a folder of stems to open at start.
    let folder = std::env::args().nth(1);
    eframe::run_native(
        "[stemyla] Stem explorer",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1200.0, 720.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut app = app::StemApp::new(cc);
            if let Some(dir) = folder {
                match engine::Song::load_folder(std::path::Path::new(&dir)) {
                    Ok(s) => app.open_song(s),
                    Err(e) => app.status = format!("Could not load {dir}: {e:#}"),
                }
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    app::start_web();
}
