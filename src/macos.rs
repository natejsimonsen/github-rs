//! macOS window tweaks eframe has no option for.

use objc2_app_kit::{NSView, NSWindowButton};
use objc2_foundation::NSPoint;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Where the window buttons go: centered in the app's header, starting at
/// its left padding. `TRAFFIC_LIGHTS_W` in the header leaves room for them.
const HEADER_H: f64 = crate::theme::HEADER_H as f64;
const LEFT: f64 = 16.0;

/// Moves the close / minimize / zoom buttons down into the app's header.
/// macOS puts them back on some resizes and on leaving full screen, so call
/// this every frame; it only touches them when they've moved.
pub fn place_traffic_lights(window: &impl HasWindowHandle) {
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: the handle points at the window's live content view.
    let view: &NSView = unsafe { h.ns_view.cast().as_ref() };
    let Some(window) = view.window() else { return };
    let buttons = [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton];
    let Some(buttons) = buttons.map(|b| window.standardWindowButton(b)).into_iter().collect::<Option<Vec<_>>>() else { return };
    // The buttons sit in the title bar's container view; make that as tall
    // as the header, or they'd be cut off below its usual 28pt.
    // SAFETY: the window keeps its title bar views alive while it's open.
    let Some(bar) = (unsafe { buttons[0].superview().and_then(|v| v.superview()) }) else { return };
    let mut frame = bar.frame();
    let top = window.frame().size.height;
    if frame.size.height != HEADER_H || frame.origin.y != top - HEADER_H {
        frame.size.height = HEADER_H;
        frame.origin.y = top - HEADER_H;
        bar.setFrame(frame);
    }
    let step = buttons[1].frame().origin.x - buttons[0].frame().origin.x;
    for (i, b) in buttons.iter().enumerate() {
        let size = b.frame().size;
        let want = NSPoint::new(LEFT + i as f64 * step, ((HEADER_H - size.height) / 2.0).round());
        if b.frame().origin != want {
            b.setFrameOrigin(want);
        }
    }
}
