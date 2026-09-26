//! Terminal pane — alacritty_terminal event loop + GPUI canvas renderer.
//! Same crate stack cosmos uses for its terminal.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty::{self, Options as PtyOptions, Shell};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui::{
    App, Bounds, Context, FocusHandle, Focusable, Hsla, KeyDownEvent, Keystroke, Pixels,
    ScrollWheelEvent, SharedString, Size, TextRun, Window, canvas, div, fill, point, prelude::*,
    px, size,
};
use parking_lot::Mutex as PMutex;

use crate::theme::{FONT_MONO, Theme};

type Pty = alacritty_terminal::tty::Pty;

/// Wakes the app when the grid changes.
struct Proxy {
    tx: mpsc::Sender<TermEvent>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: TermEvent) {
        let _ = self.tx.send(event);
    }
}

#[derive(Clone)]
struct GCell {
    ch: char,
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    underline: bool,
    wide: bool,
}

struct Row {
    line: i32,
    cells: Vec<GCell>,
    cursor_col: Option<usize>,
    cursor_block: bool,
}

/// A terminal surface for a tab. The PTY/event loop keep running while the
/// tab is hidden; only painting is skipped.
pub struct TerminalPane {
    term: Arc<FairMutex<Term<Proxy>>>,
    sender: alacritty_terminal::event_loop::EventLoopSender,
    _join: std::thread::JoinHandle<(EventLoop<Pty, Proxy>, alacritty_terminal::event_loop::State)>,
    focus: FocusHandle,
    cell: Size<Pixels>,
    scroll: usize,
    pub title: SharedString,
    pub exit_status: Option<String>,
    cols: usize,
    rows: usize,
    rx: PMutex<Receiver<TermEvent>>,
}

pub enum TerminalEvent {
    TitleChanged,
}

impl gpui::EventEmitter<TerminalEvent> for TerminalPane {}

impl TerminalPane {
    pub fn spawn(
        command: Option<&str>,
        cwd: Option<&str>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Self> {
        let dims = TermDims { cols: 120, rows: 30 };
        let (tx, rx) = mpsc::channel::<TermEvent>();
        let proxy = Proxy { tx: tx.clone() };
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &dims, proxy)));

        let options = PtyOptions {
            shell: command
                .map(|c| Shell::new(program_of(c), shell_args(c)))
                .or_else(default_shell),
            working_directory: cwd.map(std::path::PathBuf::from),
            drain_on_exit: false,
            env: Default::default(),
            #[cfg(target_os = "windows")]
            escape_args: false,
        };
        let window_size = WindowSize {
            num_lines: dims.rows as u16,
            num_cols: dims.cols as u16,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, window_size, 0)?;
        let event_proxy = Proxy { tx };
        let event_loop = EventLoop::new(term.clone(), event_proxy, pty, false, false)?;
        let sender = event_loop.channel();
        let join = event_loop.spawn();

        Ok(Self {
            term,
            sender,
            _join: join,
            focus: cx.focus_handle(),
            cell: size(px(8.), px(16.)),
            scroll: 0,
            title: "Terminal".into(),
            exit_status: None,
            cols: dims.cols,
            rows: dims.rows,
            rx: PMutex::new(rx),
        })
    }

    /// Drain pending PTY events; call on each frame. `true` = repaint needed.
    pub fn pump_events(&mut self, cx: &mut Context<Self>) -> bool {
        let mut dirty = false;
        loop {
            let event = self.rx.lock().try_recv().ok();
            match event {
                Some(TermEvent::Wakeup) => dirty = true,
                Some(TermEvent::Title(t)) => {
                    self.title = t.into();
                    cx.emit(TerminalEvent::TitleChanged);
                }
                Some(TermEvent::ResetTitle) => {
                    self.title = "Terminal".into();
                    cx.emit(TerminalEvent::TitleChanged);
                }
                Some(TermEvent::ChildExit(code)) => {
                    self.exit_status = Some(format!("exit {code}"));
                    cx.emit(TerminalEvent::TitleChanged);
                    dirty = true;
                }
                Some(TermEvent::PtyWrite(s)) => self.write(s.into_bytes()),
                Some(_) => {}
                None => break,
            }
        }
        dirty
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    fn resize(&mut self, cols: usize, rows: usize, window: &Window) {
        if cols == 0 || rows == 0 || (cols == self.cols && rows == self.rows) {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.lock().resize(TermDims { cols, rows });
        let _ = self.sender.send(Msg::Resize(WindowSize {
            num_lines: rows as u16,
            num_cols: cols as u16,
            cell_width: f32::from(self.cell.width).max(1.) as u16,
            cell_height: f32::from(self.cell.height).max(1.) as u16,
        }));
        let _ = window;
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let delta = match event.delta {
            gpui::ScrollDelta::Pixels(p) => {
                -(f32::from(p.y) / f32::from(self.cell.height).max(1.)).round() as i32
            }
            gpui::ScrollDelta::Lines(l) => -(l.y.round() as i32),
        };
        if delta == 0 {
            return;
        }
        let mut term = self.term.lock();
        let max = term.grid().history_size() as i32;
        let next = (self.scroll as i32 + delta).clamp(0, max);
        if next != self.scroll as i32 {
            term.scroll_display(Scroll::Delta(self.scroll as i32 - next));
            self.scroll = next as usize;
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn key_down(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke) {
            self.write(bytes);
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn measure(&mut self, window: &Window) -> Size<Pixels> {
        let runs = [TextRun {
            len: 1,
            font: gpui::font(SharedString::from(FONT_MONO)),
            color: gpui::white().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let line = window
            .text_system()
            .shape_line("M".into(), px(13.), &runs, None);
        let w = line.x_for_index(1).max(px(6.));
        let h = (window.line_height() * 1.1).max(px(13.));
        size(w, h)
    }

    /// Snapshot the visible grid into plain cells (short lock, no refs held).
    fn snapshot(&self, rows_max: usize) -> Vec<Row> {
        let term = self.term.lock();
        let content = term.renderable_content();
        let cursor = content.cursor;
        let cursor_on = self.scroll == 0 && !matches!(cursor.shape, CursorShape::Hidden);
        let mut out: Vec<Row> = Vec::with_capacity(rows_max + 1);
        let mut current: Option<Row> = None;
        for indexed in content.display_iter {
            let point = indexed.point;
            let cell = indexed.cell;
            let li = point.line.0;
            if li < 0 || li as usize >= rows_max {
                continue;
            }
            let need_new = match &current {
                Some(r) => r.line != li,
                None => true,
            };
            if need_new {
                if let Some(r) = current.take() {
                    out.push(r);
                }
                current = Some(Row {
                    line: li,
                    cells: Vec::with_capacity(point.column.0 + 8),
                    cursor_col: None,
                    cursor_block: true,
                });
            }
            let row = current.as_mut().unwrap();
            while row.cells.len() < point.column.0 {
                row.cells.push(blank());
            }
            let mut fg = map_color(cell.fg);
            let mut bgc = (!matches!(cell.bg, Color::Named(NamedColor::Background)))
                .then(|| map_color(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                let tmp = bgc.unwrap_or(fg);
                bgc = Some(fg);
                fg = tmp;
            }
            row.cells.push(GCell {
                ch: cell.c,
                fg,
                bg: bgc,
                bold: cell.flags.contains(Flags::BOLD),
                underline: cell.flags.contains(Flags::UNDERLINE),
                wide: cell.flags.contains(Flags::WIDE_CHAR),
            });
            if cursor_on
                && point.column == cursor.point.column
                && point.line == cursor.point.line
            {
                row.cursor_col = Some(point.column.0);
                row.cursor_block = matches!(cursor.shape, CursorShape::Block | CursorShape::Underline | CursorShape::Beam);
            }
        }
        if let Some(r) = current {
            out.push(r);
        }
        out
    }
}

fn blank() -> GCell {
    GCell {
        ch: ' ',
        fg: gpui::white().into(),
        bg: None,
        bold: false,
        underline: false,
        wide: false,
    }
}

struct TermDims {
    cols: usize,
    rows: usize,
}
impl Dimensions for TermDims {
    fn total_lines(&self) -> usize {
        self.rows + 1000
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

fn program_of(command: &str) -> String {
    command.split_whitespace().next().unwrap_or(command).to_string()
}
fn shell_args(command: &str) -> Vec<String> {
    command.split_whitespace().skip(1).map(|s| s.to_string()).collect()
}
fn default_shell() -> Option<Shell> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    Some(Shell::new(shell, vec!["-l".into()]))
}

fn map_color(c: Color) -> Hsla {
    match c {
        Color::Named(n) => named(n),
        Color::Spec(rgb) => gpui::rgb(u32::from_be_bytes([0, rgb.r, rgb.g, rgb.b])).into(),
        Color::Indexed(i) => indexed(i),
    }
}

fn named(n: NamedColor) -> Hsla {
    use NamedColor::*;
    gpui::rgb(match n {
        Black => 0x1b1b1f,
        Red => 0xf26d6d,
        Green => 0x59c56f,
        Yellow => 0xe6b450,
        Blue => 0x6aa8ff,
        Magenta => 0xc792ea,
        Cyan => 0x5ad4e6,
        White => 0xd5d8df,
        BrightBlack => 0x636672,
        BrightRed => 0xff8785,
        BrightGreen => 0x8ede95,
        BrightYellow => 0xffd479,
        BrightBlue => 0x9cc4ff,
        BrightMagenta => 0xe2b0ff,
        BrightCyan => 0x9bedf5,
        BrightWhite => 0xffffff,
        Foreground => 0xd5d8df,
        Background => 0x0e0e10,
        Cursor => 0xd5d8df,
        BrightForeground => 0xffffff,
        DimForeground => 0x9a9daa,
        _ => 0xd5d8df,
    })
    .into()
}

fn indexed(i: u8) -> Hsla {
    let n = i as u32;
    let rgb = match n {
        0..=15 => [
            0x1b1b1f, 0xf26d6d, 0x59c56f, 0xe6b450, 0x6aa8ff, 0xc792ea, 0x5ad4e6, 0xd5d8df,
            0x636672, 0xff8785, 0x8ede95, 0xffd479, 0x9cc4ff, 0xe2b0ff, 0x9bedf5, 0xffffff,
        ][n as usize],
        16..=231 => {
            let n = n - 16;
            let f = |v: u32| if v == 0 { 0 } else { 55 + 40 * v };
            (f(n / 36) << 16) | (f((n % 36) / 6) << 8) | f(n % 6)
        }
        _ => {
            let v = 8 + 10 * (n - 232);
            (v << 16) | (v << 8) | v
        }
    };
    gpui::rgb(rgb).into()
}

fn keystroke_to_bytes(ks: &Keystroke) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    if m.platform || m.function {
        return None;
    }
    let bytes: Vec<u8> = match ks.key.as_str() {
        "enter" => vec![b'\r'],
        "backspace" => vec![0x7f],
        "delete" => b"\x1b[3~".to_vec(),
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }
        }
        "escape" => vec![0x1b],
        "up" => b"\x1b[A".to_vec(),
        "down" => b"\x1b[B".to_vec(),
        "right" => b"\x1b[C".to_vec(),
        "left" => b"\x1b[D".to_vec(),
        "home" => b"\x1b[H".to_vec(),
        "end" => b"\x1b[F".to_vec(),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "space" => vec![b' '],
        _ => {
            let ch = ks.key_char.as_ref()?;
            let mut s = ch.clone().into_bytes();
            if m.control && let Some(&b) = s.first() {
                if b.is_ascii_lowercase() {
                    s = vec![b - b'a' + 1];
                } else if b == b'[' {
                    s = vec![0x1b];
                }
            }
            if m.alt {
                let mut v = vec![0x1b];
                v.extend(s);
                s = v;
            }
            s
        }
    };
    Some(bytes)
}

impl Focusable for TerminalPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

struct GridPaint {
    rows: Vec<Row>,
    cell: Size<Pixels>,
    cursor_bg: Hsla,
}

impl gpui::Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pump_events(cx) {
            cx.notify();
        }
        self.cell = self.measure(window);
        let theme = Theme::of(cx);
        let entity = cx.entity();
        let accent = theme.palette.accent;
        // Terminals stay dark in both themes — ANSI palettes assume it, and
        // translucent light washes shell text out over the wallpaper.
        div()
            .key_context("TerminalPane")
            .track_focus(&self.focus_handle(cx))
            .on_key_down(cx.listener(Self::key_down))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .size_full()
            .bg(gpui::hsla(240., 0.06, 0.05, 0.96))
            .p(px(8.))
            .font_family(FONT_MONO)
            .text_size(px(13.))
            .child(canvas(
                move |bounds, window, cx| {
                    let mut grid = GridPaint {
                        rows: Vec::new(),
                        cell: size(px(8.), px(16.)),
                        cursor_bg: accent,
                    };
                    entity.update(cx, |pane, _| {
                        let cell = pane.cell;
                        let cols = (bounds.size.width / cell.width).floor() as usize;
                        let rows = (bounds.size.height / cell.height).floor() as usize;
                        pane.resize(cols, rows, window);
                        grid.cell = cell;
                        grid.rows = pane.snapshot(rows.max(1));
                    });
                    grid
                },
                move |bounds, grid, window, cx| {
                    paint_grid(bounds, grid, window, cx);
                },
            ))
    }
}

fn paint_grid(
    bounds: Bounds<Pixels>,
    grid: GridPaint,
    window: &mut Window,
    cx: &mut App,
) {
    let cell = grid.cell;
    for row in grid.rows {
        let top = bounds.top() + cell.height * row.line as f32;
        if top > bounds.bottom() {
            break;
        }
        // Background runs.
        let mut x = bounds.left();
        for c in &row.cells {
            let w = cell.width * if c.wide { 2. } else { 1. };
            if let Some(bg) = c.bg {
                window.paint_quad(fill(
                    Bounds::new(point(x, top), size(w, cell.height)),
                    bg,
                ));
            }
            x += w;
        }
        // Text runs: group consecutive cells sharing a style.
        let mut x = bounds.left();
        let mut i = 0;
        let mut cursor_at: Option<Pixels> = None;
        while i < row.cells.len() {
            let mut j = i;
            let style_of = |c: &GCell| (c.fg, c.bold, c.underline);
            while j + 1 < row.cells.len() && style_of(&row.cells[j + 1]) == style_of(&row.cells[i]) {
                j += 1;
            }
            let text: String = row.cells[i..=j].iter().map(|c| c.ch).collect();
            let start_x = x;
            let width = cell.width * (j - i + 1) as f32;
            let first = &row.cells[i];
            if !text.trim().is_empty() || first.bg.is_some() {
                let font = gpui::font(SharedString::from(FONT_MONO));
                let font = if first.bold {
                    gpui::Font {
                        weight: gpui::FontWeight::BOLD,
                        ..font
                    }
                } else {
                    font
                };
                let runs = [TextRun {
                    len: text.len(),
                    font,
                    color: first.fg,
                    background_color: None,
                    underline: first.underline.then_some(gpui::UnderlineStyle {
                        color: Some(first.fg),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    strikethrough: None,
                }];
                {
                    let line = window.text_system().shape_line(
                        SharedString::from(text),
                        px(13.),
                        &runs,
                        None,
                    );
                    let _ = line.paint(
                        point(start_x, top),
                        cell.height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
            if let Some(cc) = row.cursor_col
                && cc >= i
                && cc <= j
            {
                cursor_at = Some(start_x + cell.width * (cc - i) as f32);
            }
            x += width;
            i = j + 1;
        }
        if let Some(cx0) = cursor_at {
            window.paint_quad(fill(
                Bounds::new(
                    point(cx0.min(bounds.right() - cell.width), top),
                    size(cell.width, cell.height),
                ),
                grid.cursor_bg.opacity(0.85),
            ));
        }
    }
}
