//! macOS native webview host — a wry `WKWebView` parented under the GPUI
//! window's content view, composited beneath deferred GPUI overlays
//! (`window.enable_scene_overlay()`). GPUI paints the chrome; this module
//! keeps the native view's frame synced to the pane's painted bounds.
//!
//! The same compositing boundary cosmos uses on macOS.

use std::cell::Cell;
use std::sync::mpsc::Sender;

use gpui::{Bounds, Pixels};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DeclaredClass, MainThreadOnly, class, define_class, msg_send};
use objc2_app_kit::{
    NSColor, NSEvent, NSEventMask, NSEventModifierFlags, NSView, NSWindowOrderingMode,
};
use objc2_foundation::{
    MainThreadMarker, NSDictionary, NSError, NSJSONSerialization, NSKeyValueChangeKey,
    NSKeyValueObservingOptions, NSObject, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol,
    NSString,
};
use objc2_web_kit::{
    WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationResponse, WKNavigationResponsePolicy, WKWebView, WKWebViewConfiguration,
};
use wry::{WebViewBuilderExtMacos as _, WebViewExtMacOS as _};

use super::WebEvent;

pub struct HostIvars {
    tab: String,
    tx: Sender<WebEvent>,
}

const OBSERVED: [&str; 3] = ["title", "canGoBack", "canGoForward"];

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "HifiBrowserObserver"]
    #[ivars = HostIvars]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            _key: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
            _context: *mut std::ffi::c_void,
        ) {
            if let Some(view) = object.and_then(|o| o.downcast_ref::<WKWebView>()) {
                self.push_state(view, unsafe { view.isLoading() });
            }
        }
    }

    unsafe impl WKNavigationDelegate for Observer {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn action_policy(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            decision: &block2::Block<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let url = unsafe { action.request().URL() }
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
                .unwrap_or_default();
            if url.starts_with("hifi://") {
                decision.call((WKNavigationActionPolicy::Cancel,));
                let _ = self.ivars().tx.send(WebEvent::Url {
                    tab: self.ivars().tab.clone(),
                    url,
                });
                return;
            }
            decision.call((WKNavigationActionPolicy::Allow,));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn response_policy(
            &self,
            _view: &WKWebView,
            response: &WKNavigationResponse,
            decision: &block2::Block<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let ok = unsafe { response.canShowMIMEType() };
            decision.call((
                if ok {
                    WKNavigationResponsePolicy::Allow
                } else {
                    WKNavigationResponsePolicy::Cancel
                },
            ));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn start(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            self.push_state(view, true);
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn commit(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            if let Some(url) = unsafe { view.URL() }
                .and_then(|u| u.absoluteString())
                .map(|u| u.to_string())
            {
                let _ = self.ivars().tx.send(WebEvent::Url {
                    tab: self.ivars().tab.clone(),
                    url,
                });
            }
            self.push_state(view, unsafe { view.isLoading() });
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn finish(&self, view: &WKWebView, _nav: Option<&WKNavigation>) {
            self.push_state(view, false);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn provisional_error(
            &self,
            view: &WKWebView,
            _nav: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.nav_failed(view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn navigation_error(
            &self,
            view: &WKWebView,
            _nav: Option<&WKNavigation>,
            error: &NSError,
        ) {
            self.nav_failed(view, error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, view: &WKWebView) {
            let _ = self.ivars().tx.send(WebEvent::Error {
                tab: self.ivars().tab.clone(),
                message: "web process terminated".into(),
            });
            unsafe { view.reload() };
        }
    }
);

impl Observer {
    fn new(tab: String, tx: Sender<WebEvent>, mtm: MainThreadMarker) -> Retained<Self> {
        let object = mtm.alloc().set_ivars(HostIvars { tab, tx });
        unsafe { msg_send![super(object), init] }
    }

    fn nav_failed(&self, view: &WKWebView, error: &NSError) {
        if error.code() == -999 {
            return;
        }
        let desc = error.localizedDescription().to_string();
        let _ = self.ivars().tx.send(WebEvent::Error {
            tab: self.ivars().tab.clone(),
            message: desc,
        });
        self.push_state(view, false);
    }

    fn push_state(&self, view: &WKWebView, loading: bool) {
        let tx = &self.ivars().tx;
        let tab = self.ivars().tab.clone();
        let _ = tx.send(WebEvent::Loading {
            tab: tab.clone(),
            loading,
        });
        let _ = tx.send(WebEvent::CanGo {
            tab: tab.clone(),
            back: unsafe { view.canGoBack() },
            forward: unsafe { view.canGoForward() },
        });
        if !loading {
            let title = unsafe { view.title() }
                .map(|t| t.to_string())
                .unwrap_or_default();
            let _ = tx.send(WebEvent::Title { tab, title });
        }
    }
}

/// A live native webview bound to a tab.
pub struct WebPaneHost {
    pub web: wry::WebView,
    view: Retained<WKWebView>,
    /// Retained to keep the KVO/navigation delegate alive.
    #[allow(dead_code)]
    observer: Retained<Observer>,
    /// Clip view inside wry's container: the webview lives inside it so a
    /// CALayer mask can clip native content to the pane's painted region.
    clip: Retained<NSView>,
    clip_mask: Retained<AnyObject>,
    parent: Retained<NSView>,
    _monitor: Option<Retained<AnyObject>>,
    visible: Cell<bool>,
}

impl WebPaneHost {
    pub fn new(
        window: &gpui::Window,
        _cx: &gpui::App,
        tab: String,
        url: &str,
        tx: Sender<WebEvent>,
        ipc_enabled: bool,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("must run on main thread")?;

        let new_tab_tx = tx.clone();
        let configuration = unsafe { WKWebViewConfiguration::new(mtm) };
        let mut builder = wry::WebViewBuilder::new()
            .with_webview_configuration(configuration)
            .with_visible(false)
            .with_focused(false)
            .with_new_window_req_handler(move |url, _| {
                let _ = new_tab_tx.send(WebEvent::NewTab { url });
                wry::NewWindowResponse::Deny
            })
            .with_download_started_handler(|_, _| false);
        if ipc_enabled {
            // `window.ipc.postMessage(str)` → WebEvent::NotesSave (editor
            // surfaces persist their body through this channel).
            let ipc_tab = tab.clone();
            let ipc_tx = tx.clone();
            builder = builder.with_ipc_handler(move |req| {
                let _ = ipc_tx.send(WebEvent::NotesSave {
                    tab: ipc_tab.clone(),
                    html: req.body().clone(),
                });
            });
        }
        let web = builder.build_as_child(window).map_err(|e| e.to_string())?;

        // Overlay must exist before the clip is ordered beneath it, and cosmos
        // enables it after build_as_child (GPUI rebuilds the view hierarchy).
        window.enable_scene_overlay().map_err(|e| e.to_string())?;

        let view: Retained<WKWebView> = Retained::into_super(web.webview());
        view.setWantsLayer(true);

        // Reparent into a clip view kept below GPUI's overlay plane so the
        // native page paints under chrome/overlays (cosmos's compositing).
        let parent = unsafe { view.superview() }.ok_or("webview parent missing")?;
        let clip: Retained<NSView> = unsafe { msg_send![class!(NSView), new] };
        clip.setWantsLayer(true);
        clip.setAutoresizesSubviews(false);
        let clip_mask: Retained<AnyObject> = unsafe { msg_send![class!(CALayer), new] };
        unsafe {
            // A mask layer without contents has alpha 0 and hides everything;
            // black makes the masked region opaque (cosmos does the same).
            let color = NSColor::blackColor().CGColor();
            let _: () = msg_send![&*clip_mask, setBackgroundColor: &*color];
            let layer: *mut AnyObject = msg_send![&*clip, layer];
            let _: () = msg_send![layer, setMasksToBounds: true];
            let _: () = msg_send![layer, setMask: &*clip_mask];
        }
        clip.setHidden(true);
        parent.addSubview(&clip);
        // Keep native content beneath the shared GPUI overlay view.
        for sibling in parent.subviews() {
            if sibling.class().name() == c"GPUIOverlayView" {
                parent.addSubview_positioned_relativeTo(
                    &clip,
                    NSWindowOrderingMode::Below,
                    Some(&sibling),
                );
                break;
            }
        }
        clip.addSubview(&view);

        let observer = Observer::new(tab.clone(), tx.clone(), mtm);
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*observer)));
        }
        for key in OBSERVED {
            unsafe {
                view.addObserver_forKeyPath_options_context(
                    &observer,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::empty(),
                    std::ptr::null_mut(),
                );
            }
        }

        // Swallow browser chords while the webview is first responder and
        // re-dispatch them through GPUI's keymap (cmd-t, cmd-w, …).
        let monitor_view = view.clone();
        let monitor_tx = tx.clone();
        let monitor_tab = tab.clone();
        let callback =
            block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                let e = unsafe { event.as_ref() };
                if monitor_view.isHidden()
                    || !has_focus(&monitor_view)
                    || e.window(mtm) != monitor_view.window()
                {
                    return event.as_ptr();
                }
                let mods = e.modifierFlags();
                let key = e
                    .charactersIgnoringModifiers()
                    .map(|s| s.to_string().to_lowercase())
                    .unwrap_or_default();
                let mut combo = String::new();
                if mods.contains(NSEventModifierFlags::Command) {
                    combo.push_str("cmd-");
                }
                if mods.contains(NSEventModifierFlags::Control) {
                    combo.push_str("ctrl-");
                }
                if mods.contains(NSEventModifierFlags::Option) {
                    combo.push_str("alt-");
                }
                if mods.contains(NSEventModifierFlags::Shift) {
                    combo.push_str("shift-");
                }
                combo.push_str(&key);
                let mut browser_key = matches!(
                    combo.as_str(),
                    "cmd-t"
                        | "cmd-w"
                        | "cmd-l"
                        | "cmd-["
                        | "cmd-]"
                        | "cmd-r"
                        | "cmd-shift-r"
                        | "cmd-,"
                        | "cmd-f"
                        | "cmd-b"
                        | "cmd-shift-\\"
                        | "ctrl-tab"
                        | "ctrl-shift-tab"
                        | "cmd-shift-["
                        | "cmd-shift-]"
                );
                // Editing chords stay in the page on editor surfaces —
                // contenteditable's native bold (cmd-b) beats the sidebar
                // toggle there.
                if ipc_enabled && combo == "cmd-b" {
                    browser_key = false;
                }
                if browser_key && gpui::Keystroke::parse(&combo).is_ok() {
                    if monitor_tx
                        .send(WebEvent::Keystroke {
                            // tab retained for future per-tab chords
                            combo: format!("{combo}|{monitor_tab}"),
                        })
                        .is_ok()
                    {
                        std::ptr::null_mut()
                    } else {
                        event.as_ptr()
                    }
                } else {
                    event.as_ptr()
                }
            });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &callback)
        };

        let host = Self {
            web,
            view,
            observer,
            clip,
            clip_mask,
            parent,
            _monitor: monitor,
            visible: Cell::new(false),
        };
        if !url.is_empty() && url != "hifi://newtab" {
            let _ = host.web.load_url(url);
        }
        Ok(host)
    }

    /// Sync the native view's frame to the pane's painted bounds. GPUI paints
    /// top-left-origin; AppKit views may be bottom-left-origin — flip when the
    /// parent isn't flipped (cosmos's math: the clip fills the parent and a
    /// layer mask pins the page to the pane rect).
    pub fn sync_bounds(&self, bounds: Bounds<Pixels>, visible: bool) {
        let x = f64::from(f32::from(bounds.origin.x));
        let y_top = f64::from(f32::from(bounds.origin.y));
        let w = f64::from(f32::from(bounds.size.width)).max(0.);
        let h = f64::from(f32::from(bounds.size.height)).max(0.);
        let parent_height = self.parent.bounds().size.height;
        let y = if self.parent.isFlipped() {
            y_top
        } else {
            parent_height - y_top - h
        };
        unsafe {
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
        }
        // Keep the host stationary; move only the mask + page frame.
        self.clip.setFrame(self.parent.bounds());
        let region = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            objc2_foundation::NSSize::new(w, h),
        );
        unsafe {
            let _: () = msg_send![&*self.clip_mask, setFrame: region];
        }
        self.view.setFrame(region);
        unsafe {
            let _: () = msg_send![class!(CATransaction), setDisableActions: false];
        }
        let show = visible && w > 0.5 && h > 0.5;
        if show != self.visible.get() {
            self.visible.set(show);
            let _ = self.web.set_visible(show);
            self.clip.setHidden(!show);
        }
    }

    pub fn hide(&self) {
        if self.visible.get() {
            self.visible.set(false);
            let _ = self.web.set_visible(false);
            self.clip.setHidden(true);
        }
    }

    /// Hand keyboard focus back to the GPUI view if this page holds it, so
    /// GPUI overlays (command bar, find bar) receive typed text.
    pub fn release_focus(&self) {
        unsafe {
            let win: *mut AnyObject = msg_send![&*self.parent, window];
            if win.is_null() {
                return;
            }
            let responder: *mut AnyObject = msg_send![win, firstResponder];
            if responder.is_null() {
                return;
            }
            let is_view: bool = msg_send![responder, isKindOfClass: class!(NSView)];
            if !is_view {
                return;
            }
            let inside: bool = msg_send![responder, isDescendantOf: &*self.clip];
            if inside {
                let _: bool = msg_send![win, makeFirstResponder: &*self.parent];
            }
        }
    }

    #[allow(dead_code)] // focus cycling API for pane focus handoff
    pub fn focus(&self) {
        let _ = self.web.focus();
    }

    #[allow(dead_code)]
    pub fn focus_parent(&self) {
        let _ = self.web.focus_parent();
    }

    /// Replace the page with an HTML string (editor surfaces).
    pub fn load_html(&self, html: &str) {
        let _ = self.web.load_html(html);
    }

    pub fn load(&self, url: &str) {
        let _ = self.web.load_url(url);
    }

    pub fn reload(&self) {
        let _ = self.web.reload();
    }

    pub fn back(&self) {
        unsafe { self.view.goBack() };
    }

    pub fn forward(&self) {
        unsafe { self.view.goForward() };
    }

    /// Evaluate JS directly on the WKWebView (wry's eval queues into
    /// `pending_scripts` until the first navigation finishes — restored or
    /// hifi:// tabs would swallow the callback; calling the view avoids that).
    /// Result delivered as a JSON string to `cb`.
    pub fn eval(&self, js: String, cb: impl FnOnce(Option<String>) + Send + 'static) {
        use objc2::AllocAnyThread as _;
        let cb = std::sync::Mutex::new(Some(cb));
        let handler = block2::RcBlock::new(move |val: *mut AnyObject, _err: *mut NSError| {
            let mut result = String::new();
            unsafe {
                if !val.is_null()
                    && let Ok(data) = NSJSONSerialization::dataWithJSONObject_options_error(
                        &*val,
                        objc2_foundation::NSJSONWritingOptions::FragmentsAllowed,
                    )
                {
                    let s = NSString::alloc();
                    let s = NSString::initWithData_encoding(
                        s,
                        &data,
                        objc2_foundation::NSUTF8StringEncoding,
                    );
                    if let Some(s) = s {
                        result = s.to_string();
                    }
                }
            }
            if let Some(cb) = cb.lock().unwrap().take() {
                cb(Some(result));
            }
        });
        unsafe {
            self.view
                .evaluateJavaScript_completionHandler(&NSString::from_str(&js), Some(&handler));
        }
    }

    /// PNG screenshot via WKSnapshotConfiguration → file path.
    pub fn screenshot(&self, path: String) {
        use objc2::AllocAnyThread as _;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        let mtm = MainThreadMarker::new().expect("main thread");
        let conf = unsafe { objc2_web_kit::WKSnapshotConfiguration::new(mtm) };
        let completion = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            if image.is_null() {
                return;
            }
            unsafe {
                let image = &*image;
                let Some(tiff) = image.TIFFRepresentation() else {
                    return;
                };
                let Some(rep) = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff)
                else {
                    return;
                };
                if let Some(png) = rep.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                ) {
                    let _ = png.writeToFile_atomically(&NSString::from_str(&path), true);
                }
            }
        });
        unsafe {
            self.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&conf), &completion);
        }
    }
}

fn has_focus(view: &NSView) -> bool {
    view.window()
        .and_then(|w| w.firstResponder())
        .and_then(|r| r.downcast::<NSView>().ok())
        .is_some_and(|v| v.isDescendantOf(view))
}
