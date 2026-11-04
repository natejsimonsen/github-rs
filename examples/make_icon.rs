//! Saves the app icon (see src/app_icon.rs) as a 1024×1024 PNG. Used by
//! `scripts/bundle-macos.sh`.
//!
//! Run: cargo run --release --example make_icon -- out.png

#[path = "../src/app_icon.rs"]
mod app_icon;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "icon.png".into());
    app_icon::draw(1024).save(&out).expect("write icon");
}
