// Hide the console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Messages and actions carry whole PR rows; a few hundred bytes copied on a
// click isn't worth boxing them.
#![allow(clippy::large_enum_variant)]

mod app;
mod auth;
mod cache;
mod diff;
mod gfm;
mod github;
mod icons;
mod images;
#[cfg(target_os = "macos")]
mod macos;
mod markdown;
mod settings;
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
            .with_min_inner_size([900.0, 560.0])
            // macOS: no title bar. The app fills the window, its header drags
            // the window, and the traffic lights sit centered in the header
            // (see `macos::place_traffic_lights`).
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        // Only redraw when something changes, so the app idles at ~0% CPU.
        run_and_return: false,
        ..Default::default()
    };
    eframe::run_native("github-prs", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
