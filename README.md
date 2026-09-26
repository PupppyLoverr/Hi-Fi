# Hi-Fi

A fast native browser where **your browser is your IDE**: tabs, splits, spaces,
a command bar, real terminals, worktrees, an agent pane, and a `hifi` CLI that
drives everything over a local socket — with **no Chromium** anywhere in it.

Hi-Fi is two things at once:

- **An everyday browser** — Arc/Safari-style: a see-through frosted sidebar
  with pinned/unpinned sections and group headers, spaces with their own
  accents, page titles and favicons, back/forward/reload, and real `https://`
  sites plus `localhost`.
- **An agent orchestrator** — Radius-style: terminal tabs that are real PTYs,
  `git worktree`-backed dev groups, diff panes, agent panes that run
  `claude`/`codex`/`cursor`/your shell in a PTY, and a `hifi` CLI that opens,
  focuses, splits and drives pages from scripts or agents.

## Stack

Hi-Fi is a **Rust + GPUI** app — the same stack as
[cosmos](https://github.com/PupppyLoverr/cosmos):

| Layer | Implementation |
|-------|----------------|
| Shell | [GPUI](https://github.com/zeronsh/zui) (Metal-rendered, GPU-accelerated) |
| Engine | `WKWebView` via [wry](https://github.com/tauri-apps/wry) on macOS — system WebKit, never Chromium |
| Terminal | `alacritty_terminal` PTY grid painted as GPUI canvas |
| Icons | 127 Solar-linear SVGs (cosmos's icon set) |
| Core | `hifi-core` — pure Rust domain model + IPC wire protocol |

The webview is composited the way cosmos does it: `WKWebView` lives inside a
clip `NSView` under GPUI's `GPUIOverlayView`, and a `CALayer` mask pins the
native page to the pane's painted region — so chrome, menus and overlays drawn
by GPUI always sit on top, and splits are just mask rects.

### Cross-platform

`hifi-core` (spaces/groups/tabs, split trees, IPC protocol, the `hifi` CLI) has
no engine or UI dependencies — it's plain Rust and compiles on macOS, Linux and
Windows. The per-platform piece is only the view layer + engine host inside
`hifi-app`:

| Platform | Engine | Status |
|----------|--------|--------|
| macOS    | `WKWebView` (wry) | shipping today |
| Linux    | WebKitGTK via wry | same code path — `wry::WebViewBuilder::build_gtk` |
| Windows  | WebKit/Gecko — **never WebView2** | new host, not a rewrite |

Honest trade-off: WebKit isn't Blink. A handful of Chrome-only sites render
imperfectly. Hi-Fi sends a Safari-compatible user agent — it never claims to
be Chrome.

## Build

Requirements: Rust stable (`rustup`), macOS 14+.

```sh
git clone https://github.com/PupppyLoverr/Hi-Fi
cd Hi-Fi
cargo build --release
./target/release/hifi-app     # the browser
./target/release/hifi         # the CLI — put it on your PATH
```

Package a macOS app bundle (release build, stripped, ad-hoc signed):

```sh
./scripts/bundle.sh           # → build/Hi-Fi.app
```

Measured on Apple silicon (macOS 14, three restored tabs): `Hi-Fi.app` is
~10 MB on disk (8.4 MB app binary, 0.7 MB CLI). The app process's
`phys_footprint` is ~41–43 MB; WebKit's page, GPU and network helpers are
separate system processes (~100 MB RSS combined with one live page).

## Run

| Key | Action |
|-----|--------|
| `⌘T` | command bar — URLs, open tabs across spaces, actions |
| `⌘L` | omnibox — edit the focused tab's address in place |
| `⌘W` | close tab |
| `⌘⇧]` / `⌘⇧[` | next / previous tab |
| `⌘1…9` | jump to tab N |
| `⌘\` | toggle sidebar |

Sidebar: pinned row at top, unpinned tabs under their **group headers**, and a
bottom space switcher. `hifi://` scheme pages: `hifi://newtab`,
`hifi://terminal`, `hifi://agent`, `hifi://diff`, `hifi://preview`,
`hifi://settings`.

## CLI

`hifi` talks to the running app over
`~/Library/Application Support/HiFi/ipc.sock` (one JSON object per line).

```sh
hifi ping
hifi tab open https://github.com --right-of <tabid> --size 40%
hifi tab list | hifi tab focus <id> | hifi tab close <id>
hifi tab exec <id> 'document.title'
hifi group create NAME [--tab ID]
hifi space list | create NAME | switch ID|NAME
hifi worktree create <repo-path> [name]
hifi browser snapshot <id> | click <id> @eN|CSS | type <id> @eN|CSS TEXT
hifi browser screenshot <id>      # PNG → path printed
```

### Browser-use

`hifi browser snapshot` returns `{url,title,elements:[{i,role,sel,label,x,y}]}`
— selectors feed straight into `browser click` / `browser type`.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ hifi-app (GPUI shell)                                │
│  GPUI scene graph + Metal rendering                  │
│  WKWebView panes in clip NSViews under the GPUI      │
│  overlay (CALayer mask = pane rect)                  │
│  TerminalPane (alacritty grid → canvas)              │
│  IPC server — JSON-over-unix-socket ─────────────┐   │
└──────────────────────────────────────────────────┼───┘
                                                   │ ipc.sock
┌──────────────────────────────────────────────────┼───┐
│ hifi-core — pure Rust, no engine deps            │   │
│  Models (Space/Group/Tab/SplitNode)              │   │
│  IPC protocol · wire client                      │   │
│  hifi CLI ───────────────────────────────────────┘   │
└──────────────────────────────────────────────────────┘
```

- **Split trees** — every group's layout is a `SplitNode` tree; a leaf holds a
  *stack* of tabs and shows the group's active tab when it's in the stack.
- **Persistence** — one JSON store at
  `~/Library/Application Support/HiFi/state.json`: spaces, groups, tabs,
  layouts. Relaunch restores the last session.
- **Worktrees** — `hifi worktree create <repo> [name]` creates
  `~/HiFi/worktrees/<repo>/<name>` on branch `hifi/<name>` and opens a
  Terminal + Diff group for it.

## Limits (v0)

- macOS first — Linux/Windows use the same crates + code path, new hosts only.
- No extensions, no reader mode, no password manager, no sync.
- WebKit ≠ Blink: occasional Chrome-only sites misbehave.
- Agent tabs wrap a PTY; they don't sandbox the agent.

## License

MIT OR Apache-2.0 — see [LICENSE](LICENSE).
