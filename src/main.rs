// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod auth;
mod cache;
mod diff;
mod gfm;
mod github;
mod icons;
mod images;
mod markdown;
#[cfg(feature = "snapshot")]
mod snapshot;
mod syntax;
mod theme;
mod util;
mod views;

fn main() -> eframe::Result {
    #[cfg(feature = "snapshot")]
    if let (Ok(path), Ok(_)) = (std::env::var("GITHUB_PRS_SCREENSHOT"), std::env::var("GITHUB_PRS_OFFSCREEN")) {
        snapshot::run(&path);
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Pull requests")
            .with_app_id("github-prs")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0]),
        // Only redraw when something changes, so the app idles at ~0% CPU.
        run_and_return: false,
        ..Default::default()
    };
    eframe::run_native("github-prs", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
