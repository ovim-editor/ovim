//! Keep unhandled macOS arrow keys out of Tao's text-input fallback.

use anyhow::{Context, Result};
use objc2::ffi::{
    objc_getAssociatedObject, objc_setAssociatedObject, OBJC_ASSOCIATION_RETAIN_NONATOMIC,
};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSEvent, NSResponder};
use std::ffi::c_void;

static RESPONDER_ASSOCIATION: u8 = 0;

define_class!(
    // SAFETY: NSResponder has no additional subclassing invariants, and the
    // interceptor has no ivars or Drop implementation.
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    struct ArrowFallbackResponder;

    impl ArrowFallbackResponder {
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if (123..=126).contains(&event.keyCode()) {
                // WebKit already handled navigation. TaoView's fallback
                // interpretKeyEvents would insert AppKit's private arrow
                // characters at cursor boundaries (tauri-apps/tauri#10194).
                let app = NSApplication::sharedApplication(self.mtm());
                if let Some(menu) = app.mainMenu() {
                    menu.performKeyEquivalent(event);
                }
                return;
            }

            // SAFETY: NSResponder forwards unhandled keys to nextResponder,
            // which remains the original TaoView in this chain.
            unsafe { msg_send![super(self), keyDown: event] }
        }
    }
);

/// Attach a retained responder to the main webview after Tauri creates it.
pub(super) fn install(webview: tauri::webview::PlatformWebview) -> Result<()> {
    let _mtm = MainThreadMarker::new().context("Arrow responder needs the main thread")?;
    // SAFETY: Tauri keeps this WKWebView alive during the callback.
    let view = unsafe { (webview.inner() as *const AnyObject).as_ref() }
        .context("Main webview pointer is null")?;
    anyhow::ensure!(
        view.class().name().to_string_lossy().contains("WryWebView"),
        "Unexpected main webview class: {}",
        view.class().name().to_string_lossy()
    );

    // SAFETY: WKWebView and TaoView are live NSResponders. The new responder
    // is retained by an association on the webview for the same lifetime.
    unsafe {
        let association_key = &RESPONDER_ASSOCIATION as *const u8 as *const c_void;
        if !objc_getAssociatedObject(view, association_key).is_null() {
            return Ok(());
        }
        let previous: Option<Retained<NSResponder>> = msg_send![view, nextResponder];
        let previous = previous.context("Main webview has no next responder")?;
        let interceptor: Retained<ArrowFallbackResponder> =
            msg_send![ArrowFallbackResponder::class(), new];
        interceptor.setNextResponder(Some(&previous));
        objc_setAssociatedObject(
            view as *const AnyObject as *mut AnyObject,
            association_key,
            &*interceptor as *const ArrowFallbackResponder as *mut AnyObject,
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );
        let _: () = msg_send![view, setNextResponder: &*interceptor];
    }
    Ok(())
}
