//! Offscreen screenshots, for reviewing the UI without a visible window
//! (works with the screen locked). Build with `--features snapshot`, then:
//!
//! ```sh
//! GITHUB_PRS_SCREENSHOT=out.png GITHUB_PRS_OFFSCREEN=1 ./target/release/github-prs
//! ```
//!
//! All the usual screenshot switches work (`GITHUB_PRS_TAB`, `_ROW`, `_THEME`,
//! `_STATE`, `_SCROLL`, `_GOTO`, …). `GITHUB_PRS_SCRIPT` then runs steps after
//! the data loads, separated by `;`:
//!
//! - `click:Label` clicks the first widget whose text contains Label
//! - `at:x,y` clicks at a point (logical pixels, from the window's top left)
//! - `hover:x,y` moves the mouse there
//! - `key:cmd+k`, `key:escape`, `key:j` presses keys
//! - `type:some text` types into the focused field
//! - `wait:500` waits that many milliseconds
//! - `shot:path.png` saves a picture mid-script
//!
//! Nothing is ever sent to GitHub that would change it: writes are blocked.

use crate::app::App;
use egui::{Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

pub fn run(path: &str) {
    crate::github::READ_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
    // GITHUB_PRS_CACHED=1: only what's cached on disk, no API calls at all.
    if std::env::var("GITHUB_PRS_CACHED").is_ok() {
        crate::github::OFFLINE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let (w, h) = size();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(2.0)
        .with_theme(match std::env::var("GITHUB_PRS_THEME").as_deref() {
            Ok("dark") => egui::Theme::Dark,
            _ => egui::Theme::Light,
        })
        // The harness first runs "until stable"; our spinners never are.
        .with_max_steps(4)
        .wgpu()
        .build_eframe(|cc| App::new(cc));

    // Wait for data, like the windowed screenshot does.
    let start = Instant::now();
    while !harness.state().shot_ready() && start.elapsed() < Duration::from_secs(60) {
        frames(&mut harness, 1);
    }
    frames(&mut harness, 10);
    log(&format!("data loaded after {:?}", start.elapsed()));

    let script = std::env::var("GITHUB_PRS_SCRIPT").unwrap_or_default();
    for step in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let (cmd, arg) = step.split_once(':').unwrap_or((step, ""));
        match cmd {
            "click" => match harness.query_all_by_label_contains(arg).next() {
                Some(node) => node.click(),
                None => eprintln!("snapshot: nothing labeled {arg:?}"),
            },
            "at" | "hover" => {
                let pos = point(arg);
                harness.event(egui::Event::PointerMoved(pos));
                if cmd == "at" {
                    for pressed in [true, false] {
                        harness.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                    }
                }
            }
            "key" => {
                let (mods, key) = keys(arg);
                harness.key_press_modifiers(mods, key);
            }
            "type" => harness.event(egui::Event::Text(arg.to_string())),
            "wait" => {
                let until = Instant::now() + Duration::from_millis(arg.parse().unwrap_or(500));
                while Instant::now() < until {
                    frames(&mut harness, 1);
                }
            }
            "shot" => save(&mut harness, arg),
            _ => eprintln!("snapshot: unknown step {step:?}"),
        }
        frames(&mut harness, 6);
    }
    save(&mut harness, path);
    log(&format!("saved {path} after {:?}", start.elapsed()));
    // Background prefetch threads would keep the process alive; we're done.
    std::process::exit(0);
}

fn log(msg: &str) {
    eprintln!("snapshot: {msg}");
}

/// Run frames in real time, so background network replies arrive.
fn frames(harness: &mut Harness<'_, App>, n: usize) {
    for _ in 0..n {
        let t = Instant::now();
        harness.step();
        if std::env::var("GITHUB_PRS_PERF").is_ok() {
            log(&format!("step {:?}", t.elapsed()));
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

fn save(harness: &mut Harness<'_, App>, path: &str) {
    match harness.render() {
        Ok(img) => {
            if let Err(e) = img.save(path) {
                eprintln!("snapshot: couldn't save {path}: {e}");
            }
        }
        Err(e) => eprintln!("snapshot: render failed: {e}"),
    }
}

/// GITHUB_PRS_SIZE=1280x800, default 1440x900.
fn size() -> (f32, f32) {
    std::env::var("GITHUB_PRS_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1440.0, 900.0))
}

fn point(s: &str) -> Pos2 {
    let (x, y) = s.split_once(',').unwrap_or(("0", "0"));
    Pos2::new(x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0))
}

/// "cmd+shift+b" -> (COMMAND|SHIFT, B)
fn keys(s: &str) -> (Modifiers, Key) {
    let mut mods = Modifiers::NONE;
    let mut key = Key::Escape;
    for part in s.split('+') {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "ctrl" => mods |= Modifiers::COMMAND,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" => mods |= Modifiers::ALT,
            "enter" | "return" => key = Key::Enter,
            "esc" | "escape" => key = Key::Escape,
            "end" => key = Key::End,
            "home" => key = Key::Home,
            "down" | "arrowdown" => key = Key::ArrowDown,
            "up" | "arrowup" => key = Key::ArrowUp,
            "tab" => key = Key::Tab,
            "/" | "slash" => key = Key::Slash,
            _ => match Key::from_name(part).or_else(|| Key::from_name(&part.to_ascii_uppercase())) {
                Some(k) => key = k,
                None => eprintln!("snapshot: unknown key {part:?}, sending Escape"),
            },
        }
    }
    (mods, key)
}
