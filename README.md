# Hi-Fi

A native browser where **your browser is your IDE**: tabs, splits, spaces, a
command bar, real terminals, worktrees, an agent pane, and a CLI that drives
everything over a local socket — with **no Chromium** anywhere in it.

Hi-Fi is two things at once:

- **An everyday browser** — Arc/Safari-style: hidden-until-needed chrome, a
  translucent sidebar with pinned/unpinned sections and group headers, spaces
  with their own accents, page titles and favicons, find-in-page, downloads,
  back/forward/reload, and real `https://` sites plus `localhost`.
- **An agent orchestrator** — Radius-style: terminal tabs that are real PTYs,
  `git worktree`-backed dev groups, diff panes, agent panes that run
  `claude`/`codex`/`cursor`/your shell in a PTY, and a `hifi` CLI that opens,
  focuses, splits and drives pages from scripts or agents.

## Why not Chromium

Chromium-based shells (Electron, CEF, WebView2, Chromium-embedded-anything)
drag along Blink + a Node-shaped runtime + hundreds of MB of baseline weight.
Hi-Fi uses the platform's own engine instead:

| Platform | Engine | UI layer |
|----------|--------|----------|
| macOS    | `WKWebView` (WebKit, system) | AppKit + SwiftUI |
| Linux    | `WebKitGTK` | GTK / Swift-GTK |
| Windows  | WebKit or Gecko — **never WebView2** | native or Swift-WinUI |

The entire domain layer (`HiFiCore` — tabs/groups/spaces/split trees, the IPC
protocol, the `hifi` CLI, keybindings, the `EngineHost` engine interface) is
pure Swift with **zero WebKit imports** and compiles on all three OSes via the
official Swift toolchains. Only the view + engine host is per-platform, so
macOS ships first and Linux/Windows are new hosts, not rewrites.
See [PLATFORM.md](PLATFORM.md) for the port map.

Honest trade-off: WebKit isn't Blink. A handful of Chrome-only sites (some
video-call widgets, some devtools-ish pages) render imperfectly or refuse.
Hi-Fi sends a Safari-compatible user agent — it never claims to be Chrome.

## Build

Requirements: macOS 14+, Xcode 16+ with the command-line tools (uses only
system frameworks + [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)
via SwiftPM — fetched automatically).

```sh
git clone https://github.com/PupppyLoverr/Hi-Fi
cd Hi-Fi
./scripts/bundle.sh        # builds + stages build/Hi-Fi.app
open build/Hi-Fi.app
```

`bundle.sh` produces `build/Hi-Fi.app` containing:

- `Contents/MacOS/HiFiApp` — the browser
- `Contents/MacOS/hifi` — the CLI (add it to your `PATH`, e.g.
  `export PATH="$PWD/build/Hi-Fi.app/Contents/MacOS:$PATH"`)
- `Contents/Resources/AppIcon.icns` — the app icon

To build without bundling: `swift build` gives you `HiFiApp` and `hifi` in
`.build/debug/`.

## Run

```sh
open build/Hi-Fi.app
```

| Key | Action |
|-----|--------|
| `⌘T` | command bar — URLs, open tabs across spaces, actions |
| `⌘L` | address field scoped to the focused pane |
| `⌘W` | close tab |
| `⌘⇧]` / `⌘⇧[` | next / previous tab |
| `⌃⇥` | cycle tabs |
| `⌘1…9` | jump to tab N |
| `⌘\` | toggle sidebar |
| `⌘⏎` | split right |
| `⌘F` | find in page |
| `⌘⇧N` | new space |

Sidebar: pinned row at top, unpinned tabs under their **group headers**, and a
bottom space switcher with a per-space accent dot. Command bar actions include
`split right`, `split down`, `new space`, `new worktree`, `new terminal`,
`new agent`, `new diff`, `toggle sidebar`, `pin tab`, `close tab`.

`hifi://` scheme pages: `hifi://newtab`, `hifi://terminal`, `hifi://agent`,
`hifi://diff`, `hifi://preview`, `hifi://settings`.

## CLI

`hifi` talks to the running app over
`~/Library/Application Support/HiFi/ipc.sock` (one JSON object per line).
If the app isn't running it launches it and waits up to 8s.

```sh
hifi ping
hifi tab open https://github.com --right-of <tabid> --size 40%
hifi tab list --window
hifi tab focus <id>      | hifi tab close <id>
hifi tab split <id> --right <newid> --size 40%
hifi group create NAME [--tab ID]  | group rename ID NAME | group show ID
hifi space list | create NAME | switch ID|NAME
hifi worktree create <repo-path> [name]
hifi agent start [--project-path PATH] [--harness claude|codex|cursor|shell]
hifi page snapshot|url|html [--tab ID]
hifi page click @eN|CSS | type @eN|CSS TEXT | press KEY
hifi page screenshot [--tab ID] PATH
hifi --json ...          # machine-readable output everywhere
```

### Browser-use

`hifi page snapshot` returns the page as
`{url,title,nodes:[{ref:"@eN",role,name,tag,href,value,rect}]}` — refs feed
straight into `page click` / `page type`. The first agent action on a group
prompts **"Allow agent control?"** once per group, persisted in `state.json`.

## Demo

```sh
./scripts/demo.sh
```

A scripted walkthrough: opens a docs page, splits a terminal 40% right,
creates a named group, snapshots the page, prints `tab list`.

## Architecture

```
┌──────────────────────────────────────────────────────┐
│ HiFiApp (macOS host)                                 │
│  AppKit window + SwiftUI chrome + WKWebView panes    │
│  TerminalPaneView (SwiftTerm PTY)  DiffPaneView      │
│  IPCServer — JSON-over-unix-socket  ─────────────┐   │
└──────────────────────────────────────────────────┼───┘
                                                   │ ipc.sock
┌──────────────────────────────────────────────────┼───┐
│ HiFiCore — pure Swift, no WebKit                 │   │
│  Models (Space/Group/Tab/SplitNode/Profile)      │   │
│  IPC protocol · UnixSocket · MiniTOML · Keymap   │   │
│  EngineHost protocol (the port boundary)         │   │
│  hifi CLI (also pure Swift)  ────────────────────┘   │
└──────────────────────────────────────────────────────┘
```

- **Split trees** — every group's layout is a `SplitNode` tree
  (`leaf | split(direction, first, second, ratio)`); drag-resize persists.
- **Worktrees** — `hifi worktree create <repo> [name]` creates
  `~/HiFi/worktrees/<repo>/<name>` on branch `hifi/<name>`, copies files from
  `hifi.toml [worktree] copy_files`, runs `.hifi/setup.sh` or
  `hifi.toml [scripts] setup` with `HIFI_*` env, then opens a Terminal + Diff
  group for it.
- **Persistence** — one JSON store at
  `~/Library/Application Support/HiFi/state.json`: spaces, groups, tabs,
  layouts, profiles, agent permissions, 30-day history, worktree registry.
  Relaunch restores the last session.
- **Profiles** — separate `WKWebsiteDataStore` per profile (cookies/cache
  isolated). Popups open as tabs.
- **Logging** — `~/Library/Application Support/HiFi/hifi.log` (no secrets, no
  page content).

## Limits (v0)

- macOS only, today — the Linux/Windows hosts aren't written yet (see
  PLATFORM.md).
- No sandboxing of the app itself (WebKit processes are still sandboxed).
- No extensions, no reader mode, no password manager, no sync.
- WebKit ≠ Blink: occasional Chrome-only sites misbehave.
- The command bar's fuzzy matching is substring-based, not clever.
- Agent tabs wrap a PTY; they don't sandbox the agent.

## Next

- WebKitGTK host (`Sources/HiFiLinux`) — the `EngineHost` port.
- Windows host (WebKit/Gecko, not WebView2).
- Tree-style tabs + collapsed group state in the sidebar.
- `hifi page scroll` / multi-action batches.
- Real RSS-vs-Safari measurement in CI (see PLATFORM.md for methodology).

## License

MIT OR Apache-2.0 — see [LICENSE](LICENSE).
