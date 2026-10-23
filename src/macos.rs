//! macOS bits eframe has no option for: window tweaks, and links that
//! macOS hands the app (say, a pull request link routed here by Finicky).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, class, define_class, msg_send, sel};
use objc2_app_kit::{NSView, NSWindowButton};
use objc2_foundation::{NSPoint, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::{Mutex, OnceLock};

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

/// Links macOS asked the app to open, waiting for the next frame.
static OPENED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Wakes the app when a link arrives, since it only redraws on changes.
static CTX: OnceLock<egui::Context> = OnceLock::new();

/// Apple Event codes for "open this URL": class and ID are both 'GURL',
/// and the URL is the event's direct object, '----'.
const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "GitHubPrsUrlHandler"]
    struct UrlHandler;

    impl UrlHandler {
        #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
        fn handle_get_url(&self, event: &AnyObject, _reply: &AnyObject) {
            // SAFETY: `event` is an NSAppleEventDescriptor, and both methods
            // return objects or nil.
            let url: Option<Retained<NSString>> = unsafe {
                let desc: Option<Retained<AnyObject>> = msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT];
                desc.and_then(|d| msg_send![&d, stringValue])
            };
            let Some(url) = url else { return };
            OPENED.lock().unwrap().push(url.to_string());
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        }
    }
);

/// Start listening for links. Call before the app's window opens, so the
/// link that launched it isn't missed.
pub fn listen_for_links() {
    // SAFETY: plain init on a fresh NSObject subclass.
    let handler: Retained<UrlHandler> = unsafe { msg_send![UrlHandler::alloc(), init] };
    // SAFETY: the selector matches the method above, and its arguments are
    // what the event manager passes.
    unsafe {
        let manager: Retained<AnyObject> = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let _: () =
            msg_send![&manager, setEventHandler: &*handler, andSelector: sel!(handleGetURLEvent:withReplyEvent:), forEventClass: GET_URL, andEventID: GET_URL];
    }
    // The event manager doesn't keep the handler alive; it lives as long as the app.
    std::mem::forget(handler);
}

/// Redraw when a link arrives.
pub fn wake_on_link(ctx: &egui::Context) {
    let _ = CTX.set(ctx.clone());
}

/// Links that arrived since the last call.
pub fn take_links() -> Vec<String> {
    std::mem::take(&mut *OPENED.lock().unwrap())
}
