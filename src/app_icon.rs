//! Draws the app icon: a pull request symbol on a dark tile.
//!
//! The app hands it to the window at startup. Without that, eframe puts its
//! own egui logo in the Dock, over the bundle's AppIcon.icns.
//! `examples/make_icon.rs` saves it as a PNG for that .icns.

use image::{Rgba, RgbaImage};

/// Distance from point p to the segment a–b.
fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// Signed distance to a rounded square (negative inside).
fn tile_dist(p: (f32, f32), min: f32, max: f32, r: f32) -> f32 {
    let c = (min + max) / 2.0;
    let h = (max - min) / 2.0 - r;
    let qx = (p.0 - c).abs() - h;
    let qy = (p.1 - c).abs() - h;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// The icon as a `size`×`size` image. Laid out on a 1024 grid, then scaled.
pub fn draw(size: u32) -> RgbaImage {
    let k = size as f32 / 1024.0;

    // The PR icon on its 16-unit grid, same shape as icons.rs.
    let rings = [(4.25, 3.5), (4.25, 12.5), (11.75, 12.5)];
    let paths: [&[(f32, f32)]; 3] =
        [&[(4.25, 5.25), (4.25, 10.75)], &[(11.75, 10.75), (11.75, 6.0), (11.3, 4.6), (10.2, 3.8), (7.5, 3.5)], &[(9.25, 1.75), (7.5, 3.5), (9.25, 5.25)]];
    // Apple's icon grid: an 824px tile centered in 1024, glyph ~560px wide.
    let scale = 35.0 * k;
    let offset = (size as f32 - 16.0 * scale) / 2.0;
    let grid = |(x, y): (f32, f32)| (offset + x * scale, offset + y * scale);
    let half_stroke = 0.75 * scale;
    let ring_r = 1.75 * scale;

    let bg_top = [0x2d, 0x33, 0x3b];
    let bg_bottom = [0x16, 0x1b, 0x22];
    let green = [0x3f, 0xb9, 0x50];

    let mut img = RgbaImage::new(size, size);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let p = (x as f32 + 0.5, y as f32 + 0.5);
        let tile = (0.5 - tile_dist(p, 100.0 * k, 924.0 * k, 185.0 * k)).clamp(0.0, 1.0);
        if tile == 0.0 {
            continue;
        }
        let t = y as f32 / size as f32;
        let mut c: [f32; 3] = std::array::from_fn(|i| bg_top[i] as f32 * (1.0 - t) + bg_bottom[i] as f32 * t);

        let mut d = f32::MAX;
        for &r in &rings {
            let (cx, cy) = grid(r);
            d = d.min((((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt() - ring_r).abs());
        }
        for path in paths {
            for w in path.windows(2) {
                d = d.min(seg_dist(p, grid(w[0]), grid(w[1])));
            }
        }
        let ink = (half_stroke + 0.5 - d).clamp(0.0, 1.0);
        for i in 0..3 {
            c[i] = c[i] * (1.0 - ink) + green[i] as f32 * ink;
        }
        *px = Rgba([c[0] as u8, c[1] as u8, c[2] as u8, (tile * 255.0) as u8]);
    }
    img
}

/// The icon for the window and Dock. 512px stays sharp at full Dock zoom.
#[allow(dead_code)] // Only the app uses it, not examples/make_icon.rs.
pub fn icon_data() -> egui::IconData {
    let img = draw(512);
    egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() }
}
