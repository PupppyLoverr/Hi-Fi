//! Linux native webview host, ported from cosmos: WebKitGTK renders
//! offscreen in an isolated helper process (`helper.c`) and streams frames
//! over a pipe. GPUI composites those frames like any other image, so pages
//! clip, blur and layer with the rest of the shell, and input is forwarded
//! back to the helper as synthesized GDK events.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use gpui::{Bounds, Pixels, RenderImage};
use serde_json::{Value, json};

use super::WebEvent;

type EvalCallback = Box<dyn FnOnce(Option<String>) + Send>;

#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
struct PageState {
    url: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    loading: bool,
    #[serde(default)]
    can_back: bool,
    #[serde(default)]
    can_forward: bool,
    error: Option<String>,
}

struct Worker {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    routes: Arc<Mutex<HashMap<u32, Weak<Route>>>>,
    next_id: AtomicU32,
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Worker {
    fn send(&self, id: u32, mut command: Value) -> Result<(), String> {
        command["id"] = id.into();
        let data = serde_json::to_vec(&command).map_err(|e| e.to_string())?;
        let mut pipe = self.stdin.lock().map_err(|e| e.to_string())?;
        pipe.write_all(&(data.len() as u32).to_le_bytes())
            .and_then(|_| pipe.write_all(&data))
            .map_err(|e| e.to_string())
    }
}

struct Route {
    tab: String,
    tx: Sender<WebEvent>,
    state: Mutex<PageState>,
    frame: Mutex<Option<Arc<RenderImage>>>,
    evals: Mutex<VecDeque<EvalCallback>>,
}

impl Route {
    fn apply_state(&self, next: PageState) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let tab = self.tab.clone();
        if next.url != state.url
            && let Some(url) = next.url.clone().filter(|u| u != "about:blank")
        {
            let _ = self.tx.send(WebEvent::Url {
                tab: tab.clone(),
                url,
            });
        }
        if next.title != state.title && !next.title.is_empty() {
            let _ = self.tx.send(WebEvent::Title {
                tab: tab.clone(),
                title: next.title.clone(),
            });
        }
        let loading = next.loading && next.error.is_none();
        if loading != state.loading {
            let _ = self.tx.send(WebEvent::Loading {
                tab: tab.clone(),
                loading,
            });
        }
        if (next.can_back, next.can_forward) != (state.can_back, state.can_forward) {
            let _ = self.tx.send(WebEvent::CanGo {
                tab: tab.clone(),
                back: next.can_back,
                forward: next.can_forward,
            });
        }
        if next.error != state.error
            && let Some(message) = next.error.clone()
        {
            let _ = self.tx.send(WebEvent::Error { tab, message });
        }
        *state = PageState { loading, ..next };
    }
}

fn helper_path() -> Result<std::path::PathBuf, String> {
    use std::hash::{Hash, Hasher};
    const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hifi-webkit"));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    HELPER.hash(&mut hasher);
    let root = dirs::cache_dir()
        .ok_or("could not locate the cache directory")?
        .join("hifi/browser");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join(format!("webkit-{:016x}", hasher.finish()));
    if std::fs::read(&path).ok().as_deref() != Some(HELPER) {
        let temp = root.join(format!(".webkit-{}", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&temp)?;
            f.write_all(HELPER)?;
            f.sync_all()?;
            std::fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.map_err(|e| e.to_string())?;
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// One helper process serves every page; restarted if it exits.
fn worker() -> Result<Arc<Worker>, String> {
    static CURRENT: OnceLock<Mutex<Weak<Worker>>> = OnceLock::new();
    let mut current = CURRENT
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(worker) = current.upgrade()
        && worker
            .child
            .lock()
            .map_err(|e| e.to_string())?
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none()
    {
        return Ok(worker);
    }
    let mut child = Command::new(helper_path()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            format!("could not start WebKitGTK: {e}. Install the WebKitGTK 4.1 runtime.")
        })?;
    let stdin = child.stdin.take().ok_or("helper stdin missing")?;
    let stdout = child.stdout.take().ok_or("helper stdout missing")?;
    let routes: Arc<Mutex<HashMap<u32, Weak<Route>>>> = Arc::default();
    let reader_routes = routes.clone();
    std::thread::Builder::new()
        .name("hifi-browser-frames".into())
        .spawn(move || read_packets(stdout, reader_routes))
        .map_err(|e| e.to_string())?;
    let worker = Arc::new(Worker {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        routes,
        next_id: AtomicU32::new(1),
    });
    *current = Arc::downgrade(&worker);
    Ok(worker)
}

fn read_packets(
    mut stdout: std::process::ChildStdout,
    routes: Arc<Mutex<HashMap<u32, Weak<Route>>>>,
) {
    let result = (|| -> std::io::Result<()> {
        loop {
            let mut header = [0u8; 9];
            stdout.read_exact(&mut header)?;
            let id = u32::from_le_bytes([header[1], header[2], header[3], header[4]]);
            let length = u32::from_le_bytes([header[5], header[6], header[7], header[8]]) as usize;
            if length > 8192 * 8192 * 4 + 12 {
                return Err(std::io::Error::other("browser packet is too large"));
            }
            let mut data = vec![0; length];
            stdout.read_exact(&mut data)?;
            let route = routes
                .lock()
                .ok()
                .and_then(|r| r.get(&id).and_then(Weak::upgrade));
            let Some(route) = route else { continue };
            match header[0] {
                b'F' => {
                    if data.len() < 12 {
                        continue;
                    }
                    let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                    data.drain(..12);
                    let Some(pixels) = image::RgbaImage::from_raw(width, height, data) else {
                        continue;
                    };
                    if let Ok(mut frame) = route.frame.lock() {
                        *frame = Some(Arc::new(RenderImage::new([image::Frame::new(pixels)])));
                    }
                    let _ = route.tx.send(WebEvent::Redraw);
                }
                b'S' => {
                    if let Ok(state) = serde_json::from_slice::<PageState>(&data) {
                        route.apply_state(state);
                    }
                }
                b'C' => {
                    let _ = route.tx.send(WebEvent::Clipboard {
                        text: String::from_utf8_lossy(&data).into_owned(),
                    });
                }
                b'N' => {
                    let _ = route.tx.send(WebEvent::NewTab {
                        url: String::from_utf8_lossy(&data).into_owned(),
                    });
                }
                b'P' => {
                    let _ = route.tx.send(WebEvent::NotesSave {
                        tab: route.tab.clone(),
                        html: String::from_utf8_lossy(&data).into_owned(),
                    });
                }
                b'J' => {
                    let cb = route.evals.lock().ok().and_then(|mut q| q.pop_front());
                    if let Some(cb) = cb {
                        cb(Some(String::from_utf8_lossy(&data).into_owned()));
                    }
                }
                // 'I' (IME state) and 'M' (context/option menus) are not surfaced.
                _ => {}
            }
        }
    })();
    if result.is_err()
        && let Ok(routes) = routes.lock()
    {
        for route in routes.values().filter_map(Weak::upgrade) {
            let _ = route.tx.send(WebEvent::Error {
                tab: route.tab.clone(),
                message: "The browser helper stopped. Check that WebKitGTK 4.1 is installed, then reopen the tab.".into(),
            });
        }
    }
}

/// A live offscreen page bound to a tab.
pub struct WebPaneHost {
    worker: Arc<Worker>,
    route: Arc<Route>,
    id: u32,
    bounds: Cell<Bounds<Pixels>>,
    geometry: Cell<Option<(u32, u32, u32)>>,
    visible: Cell<bool>,
    image: RefCell<Option<Arc<RenderImage>>>,
    pressed: Cell<Option<gpui::MouseButton>>,
    focused: Cell<bool>,
}

impl WebPaneHost {
    pub fn new(
        _window: &gpui::Window,
        _cx: &gpui::App,
        tab: String,
        url: &str,
        tx: Sender<WebEvent>,
        ipc_enabled: bool,
    ) -> Result<Self, String> {
        let worker = worker()?;
        let id = worker.next_id.fetch_add(1, Ordering::Relaxed);
        let route = Arc::new(Route {
            tab,
            tx,
            state: Mutex::default(),
            frame: Mutex::default(),
            evals: Mutex::default(),
        });
        worker
            .routes
            .lock()
            .map_err(|e| e.to_string())?
            .insert(id, Arc::downgrade(&route));
        worker.send(id, json!({"cmd": "create", "ipc": ipc_enabled}))?;
        let host = Self {
            worker,
            route,
            id,
            bounds: Cell::new(Bounds::default()),
            geometry: Cell::new(None),
            visible: Cell::new(true),
            image: RefCell::new(None),
            pressed: Cell::new(None),
            focused: Cell::new(false),
        };
        if !url.is_empty() && url != "hifi://newtab" {
            host.load(url);
        }
        Ok(host)
    }

    fn command(&self, value: Value) {
        let _ = self.worker.send(self.id, value);
    }

    /// Resize the offscreen page to the pane and show/hide it.
    pub fn sync_bounds(&self, bounds: Bounds<Pixels>, visible: bool) {
        self.bounds.set(bounds);
        self.set_visible(visible);
    }

    fn set_visible(&self, visible: bool) {
        if self.visible.replace(visible) != visible {
            self.command(json!({"cmd": "visible", "value": u8::from(visible)}));
        }
    }

    pub fn hide(&self) {
        self.focused.set(false);
        self.set_visible(false);
    }

    /// Paint the latest frame into `bounds` (called from the pane canvas).
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut gpui::Window) {
        let scale = window.scale_factor();
        self.bounds.set(bounds);
        self.set_visible(true);
        let geometry = (
            (f32::from(bounds.size.width) * scale)
                .round()
                .clamp(1., 8192.) as u32,
            (f32::from(bounds.size.height) * scale)
                .round()
                .clamp(1., 8192.) as u32,
            scale.to_bits(),
        );
        if self.geometry.get() != Some(geometry) {
            self.geometry.set(Some(geometry));
            self.command(json!({
                "cmd": "resize",
                "width": geometry.0,
                "height": geometry.1,
                "scale": scale,
            }));
        }
        if let Some(frame) = self.route.frame.lock().ok().and_then(|mut f| f.take())
            && let Some(old) = self.image.borrow_mut().replace(frame)
        {
            let _ = window.drop_image(old);
        }
        if let Some(image) = self.image.borrow().clone() {
            let _ = window.paint_image(bounds, gpui::Corners::default(), image, 0, false);
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused.get()
    }

    pub fn release_focus(&self) {
        self.focused.set(false);
    }

    #[allow(dead_code)]
    pub fn focus(&self) {
        self.focused.set(true);
    }

    #[allow(dead_code)]
    pub fn focus_parent(&self) {
        self.release_focus();
    }

    pub fn load_html(&self, html: &str) {
        self.command(json!({"cmd": "html", "html": html}));
    }

    pub fn load(&self, url: &str) {
        self.command(json!({"cmd": "load", "url": url}));
    }

    pub fn reload(&self) {
        self.command(json!({"cmd": "reload"}));
    }

    pub fn back(&self) {
        self.command(json!({"cmd": "back"}));
    }

    pub fn forward(&self) {
        self.command(json!({"cmd": "forward"}));
    }

    /// Evaluate JS; results pair with callbacks in submission order.
    pub fn eval(&self, js: String, cb: impl FnOnce(Option<String>) + Send + 'static) {
        if let Ok(mut q) = self.route.evals.lock() {
            q.push_back(Box::new(cb));
        }
        self.command(json!({"cmd": "eval", "script": js}));
    }

    /// PNG of the last composited frame — the offscreen render is the page.
    pub fn screenshot(&self, path: String) {
        let Some(image) = self.image.borrow().clone() else {
            return;
        };
        let size = image.size(0);
        let Some(bgra) = image.as_bytes(0) else {
            return;
        };
        let rgba: Vec<u8> = bgra
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect();
        let _ = image::save_buffer(
            path,
            &rgba,
            size.width.0 as u32,
            size.height.0 as u32,
            image::ExtendedColorType::Rgba8,
        );
    }

    pub fn pointer(
        &self,
        kind: &str,
        position: gpui::Point<Pixels>,
        button: Option<gpui::MouseButton>,
        modifiers: gpui::Modifiers,
    ) {
        if kind == "down" {
            self.focused.set(true);
            self.pressed.set(button);
        }
        if kind == "up" && self.pressed.replace(None) != button {
            return;
        }
        let p = position - self.bounds.get().origin;
        let drag = if kind == "move" {
            button.map(|b| 1 << (mouse_button(b) + 7)).unwrap_or(0)
        } else {
            0
        };
        self.command(json!({
            "cmd": kind,
            "x": f32::from(p.x),
            "y": f32::from(p.y),
            "button": button.map(mouse_button).unwrap_or(0),
            "mods": modifiers_mask(modifiers) | drag,
        }));
    }

    pub fn scroll(&self, event: &gpui::ScrollWheelEvent) {
        let delta = event.delta.pixel_delta(gpui::px(16.));
        let p = event.position - self.bounds.get().origin;
        self.command(json!({
            "cmd": "scroll",
            "x": f32::from(p.x),
            "y": f32::from(p.y),
            "dx": -f32::from(delta.x) / 40.,
            "dy": -f32::from(delta.y) / 40.,
            "mods": modifiers_mask(event.modifiers),
        }));
    }

    /// Forward a keystroke; clipboard chords route through GPUI's clipboard.
    pub fn key(&self, stroke: &gpui::Keystroke, down: bool, cx: &mut gpui::App) {
        if down && stroke.modifiers.control && !stroke.modifiers.alt {
            match stroke.key.as_str() {
                "c" | "x" => {
                    let cmd = if stroke.key == "c" { "copy" } else { "cut" };
                    self.command(json!({"cmd": cmd}));
                    return;
                }
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                        self.command(json!({"cmd": "text", "text": text}));
                    }
                    return;
                }
                "a" => {
                    self.command(json!({"cmd": "select-all"}));
                    return;
                }
                _ => {}
            }
        }
        let printable = stroke.key_char.as_deref().filter(|s| {
            s.chars().count() == 1
                && !s.chars().any(char::is_control)
                && !stroke.modifiers.control
                && !stroke.modifiers.platform
        });
        let key = match printable.unwrap_or(stroke.key.as_str()) {
            "enter" => "Return",
            "backspace" => "BackSpace",
            "delete" => "Delete",
            "escape" => "Escape",
            "tab" => "Tab",
            "left" => "Left",
            "right" => "Right",
            "up" => "Up",
            "down" => "Down",
            "home" => "Home",
            "end" => "End",
            "pageup" => "Page_Up",
            "pagedown" => "Page_Down",
            "space" => "space",
            key => key,
        };
        self.command(json!({
            "cmd": if down { "key_down" } else { "key_up" },
            "key": key,
            "text": stroke.key_char.as_deref().unwrap_or(""),
            "mods": modifiers_mask(stroke.modifiers),
        }));
    }
}

impl Drop for WebPaneHost {
    fn drop(&mut self) {
        self.command(json!({"cmd": "close"}));
        if let Ok(mut routes) = self.worker.routes.lock() {
            routes.remove(&self.id);
        }
    }
}

fn modifiers_mask(m: gpui::Modifiers) -> u32 {
    u32::from(m.shift)
        | (u32::from(m.control) << 2)
        | (u32::from(m.alt) << 3)
        | (u32::from(m.platform) << 26)
}

fn mouse_button(button: gpui::MouseButton) -> u32 {
    match button {
        gpui::MouseButton::Left => 1,
        gpui::MouseButton::Middle => 2,
        gpui::MouseButton::Right => 3,
        gpui::MouseButton::Navigate(gpui::NavigationDirection::Back) => 8,
        gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward) => 9,
    }
}
