//! Windows native webview host — a WebView2 child HWND (wry) bound to the
//! GPUI window, ported from cosmos's Windows browser host.
//!
//! - Airspace: the child HWND paints above GPUI's swapchain, so bounds are
//!   clipped to the pane's content mask rather than layered under chrome.
//! - Focus: WebView2 owns Win32 focus inside the page; `AcceleratorKeyPressed`
//!   forwards browser chords back into the GPUI keymap.
//! - Creation pumps the Win32 message loop, so it runs on a foreground task
//!   (never inside a GPUI update); calls made before it lands are replayed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc::Sender;

use gpui::{Bounds, Pixels, px};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
    COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, ICoreWebView2AcceleratorKeyPressedEventHandler,
};
use wry::WebViewExtWindows as _;
use wry::dpi::{LogicalPosition, LogicalSize};

use super::WebEvent;

#[link(name = "user32")]
unsafe extern "system" {
    fn GetKeyState(key: i32) -> i16;
}

fn key_down(vk: i32) -> bool {
    unsafe { GetKeyState(vk) < 0 }
}

/// Virtual-key → gpui key name for the chords the browser chrome owns.
fn chord_key(vk: u32) -> Option<&'static str> {
    Some(match vk {
        0x09 => "tab",
        0x42 => "b",
        0x46 => "f",
        0x4B => "k",
        0x4C => "l",
        0x4E => "n",
        0x52 => "r",
        0x54 => "t",
        0x57 => "w",
        0xBC => ",",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0x74 => "f5",
        _ => return None,
    })
}

struct ParentWindow(Win32WindowHandle);

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(self.0)) })
    }
}

enum Load {
    Url(String),
    Html(String),
}

#[derive(Default)]
struct Inner {
    web: Option<wry::WebView>,
    _accelerator: Option<ICoreWebView2AcceleratorKeyPressedEventHandler>,
    pending: Option<Load>,
    bounds: Option<Bounds<Pixels>>,
    visible: bool,
}

impl Inner {
    fn apply(&self) {
        let Some(web) = &self.web else { return };
        let _ = web.set_visible(self.visible);
        if let (true, Some(b)) = (self.visible, self.bounds) {
            let _ = web.set_bounds(wry::Rect {
                position: wry::dpi::Position::Logical(LogicalPosition::new(
                    f64::from(f32::from(b.origin.x)),
                    f64::from(f32::from(b.origin.y)),
                )),
                size: wry::dpi::Size::Logical(LogicalSize::new(
                    f64::from(f32::from(b.size.width)),
                    f64::from(f32::from(b.size.height)),
                )),
            });
        }
    }
}

/// A live native webview bound to a tab.
pub struct WebPaneHost {
    inner: Rc<RefCell<Inner>>,
    tab: String,
    tx: Sender<WebEvent>,
}

impl WebPaneHost {
    pub fn new(
        window: &gpui::Window,
        cx: &gpui::App,
        tab: String,
        url: &str,
        tx: Sender<WebEvent>,
        ipc_enabled: bool,
    ) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|e| format!("window handle unavailable: {e}"))?;
        let RawWindowHandle::Win32(raw) = handle.as_raw() else {
            return Err("window does not expose a Win32 handle".into());
        };
        let parent = ParentWindow(raw);
        let inner = Rc::new(RefCell::new(Inner {
            pending: (!url.is_empty() && url != "hifi://newtab").then(|| Load::Url(url.into())),
            ..Inner::default()
        }));
        let task_inner = inner.clone();
        let task_tab = tab.clone();
        let task_tx = tx.clone();
        window
            .spawn(cx, async move |_| {
                match build(
                    &parent,
                    &task_tab,
                    task_tx.clone(),
                    ipc_enabled,
                    &task_inner,
                ) {
                    Ok((web, accelerator)) => {
                        let mut inner = task_inner.borrow_mut();
                        match inner.pending.take() {
                            Some(Load::Url(url)) => {
                                let _ = web.load_url(&url);
                            }
                            Some(Load::Html(html)) => {
                                let _ = web.load_html(&html);
                            }
                            None => {}
                        }
                        inner.web = Some(web);
                        inner._accelerator = Some(accelerator);
                        inner.apply();
                    }
                    Err(message) => {
                        let _ = task_tx.send(WebEvent::Error {
                            tab: task_tab,
                            message,
                        });
                    }
                }
            })
            .detach();
        Ok(Self { inner, tab, tx })
    }

    fn with_web(&self, f: impl FnOnce(&wry::WebView)) {
        if let Ok(inner) = self.inner.try_borrow()
            && let Some(web) = &inner.web
        {
            f(web);
        }
    }

    /// Sync the child HWND to the pane's painted (already mask-clipped) bounds.
    pub fn sync_bounds(&self, bounds: Bounds<Pixels>, visible: bool) {
        let show = visible && bounds.size.width > px(0.5) && bounds.size.height > px(0.5);
        let Ok(mut inner) = self.inner.try_borrow_mut() else {
            return;
        };
        if inner.bounds == Some(bounds) && inner.visible == show {
            return;
        }
        inner.bounds = Some(bounds);
        inner.visible = show;
        inner.apply();
    }

    pub fn hide(&self) {
        let Ok(mut inner) = self.inner.try_borrow_mut() else {
            return;
        };
        if inner.visible {
            inner.visible = false;
            inner.apply();
        }
    }

    /// Hand Win32 focus back to the GPUI window so overlays receive text.
    pub fn release_focus(&self) {
        self.with_web(|w| {
            let _ = w.focus_parent();
        });
    }

    #[allow(dead_code)]
    pub fn focus(&self) {
        self.with_web(|w| {
            let _ = w.focus();
        });
    }

    #[allow(dead_code)]
    pub fn focus_parent(&self) {
        self.release_focus();
    }

    pub fn load_html(&self, html: &str) {
        let mut inner = self.inner.borrow_mut();
        match &inner.web {
            Some(web) => {
                let _ = web.load_html(html);
            }
            None => inner.pending = Some(Load::Html(html.into())),
        }
    }

    pub fn load(&self, url: &str) {
        let mut inner = self.inner.borrow_mut();
        match &inner.web {
            Some(web) => {
                let _ = web.load_url(url);
            }
            None => inner.pending = Some(Load::Url(url.into())),
        }
    }

    pub fn reload(&self) {
        self.with_web(|w| {
            let _ = w.reload();
        });
    }

    pub fn back(&self) {
        self.with_web(|w| {
            let _ = w.go_back();
        });
    }

    pub fn forward(&self) {
        self.with_web(|w| {
            let _ = w.go_forward();
        });
    }

    /// Evaluate JS; the JSON-encoded result is delivered to `cb`.
    pub fn eval(&self, js: String, cb: impl FnOnce(Option<String>) + Send + 'static) {
        let cb = Mutex::new(Some(cb));
        let inner = self.inner.borrow();
        let Some(web) = &inner.web else {
            if let Some(cb) = cb.lock().ok().and_then(|mut c| c.take()) {
                cb(None);
            }
            return;
        };
        let _ = web.evaluate_script_with_callback(&js, move |raw| {
            if let Some(cb) = cb.lock().ok().and_then(|mut c| c.take()) {
                cb(Some(raw));
            }
        });
    }

    /// PNG screenshot via `ICoreWebView2::CapturePreview` → file path.
    pub fn screenshot(&self, path: String) {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
        let tab = self.tab.clone();
        let tx = self.tx.clone();
        let report = move |message: String| {
            let _ = tx.send(WebEvent::Error { tab, message });
        };
        let inner = self.inner.borrow();
        let Some(web) = &inner.web else {
            report("page is still starting".into());
            return;
        };
        let stream = match unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) } {
            Ok(stream) => stream,
            Err(e) => {
                report(format!("could not open the capture stream: {e}"));
                return;
            }
        };
        let reader = stream.clone();
        let handler =
            webview2_com::CapturePreviewCompletedHandler::create(Box::new(move |result| {
                if let Ok(bytes) = result.and_then(|()| read_stream(&reader)) {
                    let _ = std::fs::write(&path, bytes);
                }
                Ok(())
            }));
        if let Err(e) = unsafe {
            web.webview().CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
        } {
            report(format!("could not capture the page: {e}"));
        }
    }
}

fn build(
    parent: &ParentWindow,
    tab: &str,
    tx: Sender<WebEvent>,
    ipc_enabled: bool,
    inner: &Rc<RefCell<Inner>>,
) -> Result<(wry::WebView, ICoreWebView2AcceleratorKeyPressedEventHandler), String> {
    let history = Rc::downgrade(inner);
    let profile = hifi_core::HifiPaths::detect().root.join("webview2");
    let mut context = wry::WebContext::new(Some(profile));
    let (nav_tx, nav_tab) = (tx.clone(), tab.to_string());
    let (title_tx, title_tab) = (tx.clone(), tab.to_string());
    let (load_tx, load_tab) = (tx.clone(), tab.to_string());
    let new_tab_tx = tx.clone();
    let mut builder = wry::WebViewBuilder::new_with_web_context(&mut context)
        .with_visible(false)
        .with_focused(false)
        .with_navigation_handler(move |url| {
            if url.starts_with("hifi://") {
                let _ = nav_tx.send(WebEvent::Url {
                    tab: nav_tab.clone(),
                    url,
                });
                return false;
            }
            true
        })
        .with_document_title_changed_handler(move |title| {
            let _ = title_tx.send(WebEvent::Title {
                tab: title_tab.clone(),
                title,
            });
        })
        .with_on_page_load_handler(move |event, url| {
            let loading = matches!(event, wry::PageLoadEvent::Started);
            let tab = load_tab.clone();
            if !url.is_empty() && url != "about:blank" {
                let _ = load_tx.send(WebEvent::Url {
                    tab: tab.clone(),
                    url,
                });
            }
            let _ = load_tx.send(WebEvent::Loading {
                tab: tab.clone(),
                loading,
            });
            if let Some(inner) = history.upgrade()
                && let Ok(inner) = inner.try_borrow()
                && let Some(web) = &inner.web
            {
                let _ = load_tx.send(WebEvent::CanGo {
                    tab,
                    back: web.can_go_back().unwrap_or(false),
                    forward: web.can_go_forward().unwrap_or(false),
                });
            }
        })
        .with_new_window_req_handler(move |url, _| {
            let _ = new_tab_tx.send(WebEvent::NewTab { url });
            wry::NewWindowResponse::Deny
        })
        .with_download_started_handler(|_, _| false);
    if ipc_enabled {
        let (ipc_tx, ipc_tab) = (tx.clone(), tab.to_string());
        builder = builder.with_ipc_handler(move |req| {
            let _ = ipc_tx.send(WebEvent::NotesSave {
                tab: ipc_tab.clone(),
                html: req.body().clone(),
            });
        });
    }
    let web = builder.build_as_child(parent).map_err(|e| e.to_string())?;

    let key_tx = tx;
    let key_tab = tab.to_string();
    let handler =
        webview2_com::AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            unsafe {
                let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                {
                    return Ok(());
                }
                let mut vk = 0u32;
                args.VirtualKey(&mut vk)?;
                let Some(key) = chord_key(vk) else {
                    return Ok(());
                };
                let mut combo = String::new();
                if key_down(0x11) {
                    combo.push_str("ctrl-");
                }
                if key_down(0x12) {
                    combo.push_str("alt-");
                }
                if key_down(0x10) {
                    combo.push_str("shift-");
                }
                combo.push_str(key);
                // Editing chords stay in the page on editor surfaces.
                if (ipc_enabled && combo == "ctrl-b") || !super::is_browser_chord(&combo) {
                    return Ok(());
                }
                if key_tx
                    .send(WebEvent::Keystroke {
                        combo: format!("{combo}|{key_tab}"),
                    })
                    .is_ok()
                {
                    args.SetHandled(true)?;
                }
                Ok(())
            }
        }));
    let mut token = 0i64;
    unsafe {
        web.controller()
            .add_AcceleratorKeyPressed(&handler, &mut token)
            .map_err(|e| format!("WebView2 key hook unavailable: {e}"))?;
    }
    Ok((web, handler))
}

/// Drains an `IStream` written by `CapturePreview` back into bytes.
fn read_stream(stream: &windows::Win32::System::Com::IStream) -> windows_core::Result<Vec<u8>> {
    use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};
    unsafe {
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let size = stat.cbSize as usize;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut data = vec![0u8; size];
        let mut done = 0usize;
        while done < size {
            let mut count = 0u32;
            stream
                .Read(
                    data[done..].as_mut_ptr() as *mut _,
                    (size - done).min(u32::MAX as usize) as u32,
                    Some(&mut count),
                )
                .ok()?;
            if count == 0 {
                break;
            }
            done += count as usize;
        }
        data.truncate(done);
        Ok(data)
    }
}
