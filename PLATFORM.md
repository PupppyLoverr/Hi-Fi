# Platform map — how Hi-Fi ships on macOS, Linux, and Windows

Hi-Fi's rule: **the domain layer never imports an engine**. `HiFiCore` is
pure Swift (Foundation only) — no WebKit, no AppKit, no UIKit — and compiles
with the official Swift toolchains on all three OSes. Each platform supplies
exactly one **host**: a view layer plus an `EngineHost` implementation.

## The port boundary

`Sources/HiFiCore/EngineHost.swift` defines what "a web page" means to the
rest of the app:

- create/destroy a page (per-profile data store)
- navigate, go back/forward, reload, stop
- evaluate JavaScript (`evalJS`) — powers `hifi page snapshot|click|type|press|html`
- snapshot to PNG — powers `hifi page screenshot`
- find in page, set UA string, popup→tab policy, download events

A host implements that protocol over the platform engine and renders it into
the platform's view type. Everything else — sidebar, spaces, groups, split
trees, command bar, IPC, persistence, the `hifi` CLI, the `hifi://` scheme —
is shared.

## macOS (shipping)

| Piece | Implementation |
|-------|----------------|
| Engine | `WKWebView` + `WKProcessPool` + per-profile `WKWebsiteDataStore` |
| Chrome | AppKit `NSWindow` + SwiftUI sidebar/command bar/find bar |
| Splits | `NSSplitViewController` tree driven by `SplitNode` |
| Terminal | `SwiftTerm` `LocalProcessTerminalView` (real PTY) |
| IPC | `UnixSocketServer` on `~/Library/Application Support/HiFi/ipc.sock` |
| Persist | `~/Library/Application Support/HiFi/state.json` |

Entry: `Sources/HiFiApp/main.swift` → `AppDelegate` → `MainWindowController`.
Single instance enforced at launch (a second process exits if the socket
already answers `ping`).

## Linux (next host)

- Engine: **WebKitGTK** (`WebKitWebView`, `WebKitWebsiteDataManager` per
  profile, `webkit_web_view_run_javascript`, `webkit_web_view_get_snapshot`).
- UI: GTK4 via Swift GTK bindings (or Adwaita). Sidebar/command-bar models
  reuse `HiFiCore` wholesale.
- Terminal: same PTY story — `forkpty`/`openpty` + a GTK terminal widget
  (VTE or SwiftTerm-compatible shim).
- IPC: identical unix-socket protocol — path becomes
  `~/.local/share/hifi/ipc.sock`; state file `~/.local/share/hifi/state.json`.
- New target name: `Sources/HiFiLinux` (executable `HiFiApp`), same
  `HiFiCore` dependency.

## Windows

- Engine: WebKit (preferred, e.g. WebKitWindows port) or **Gecko** via
  embedding — **never WebView2** (it is Chromium).
- UI: WinUI/Win32 shell; the same `HiFiCore` models drive the chrome.
- Terminal: ConPTY (already a PTY API) + a VT widget.
- IPC: the same JSON-lines protocol over a unix-domain socket (Windows has
  AF_UNIX since 2018) or a named pipe shim behind `UnixSocket`.
- State: `%LOCALAPPDATA%\HiFi\state.json`.

## What does NOT change across platforms

- `hifi` CLI — one binary, same flags, same wire protocol everywhere.
- `hifi.toml` project config — `[worktree] copy_files`, `[scripts] setup`.
- `HIFI_*` env contract — `HIFI_GROUP_ID`, `HIFI_TAB_ID`,
  `HIFI_WORKSPACE_NAME`, `HIFI_WORKSPACE_PATH`, `HIFI_SOCKET`.
- Tab/group/space/split semantics and the `hifi://` scheme.

## Measuring "light" (README claim)

Methodology for the RSS-vs-Safari comparison:

1. Fresh launch, one `hifi://newtab`, idle 10s → record app RSS.
2. Open `apple.com`, `github.com`, `developer.mozilla.org` in three tabs,
   idle 10s → record app + web-content RSS.
3. Repeat in Safari with the same three tabs.

Capture via `ps -o rss -p <pid>` (and `footprint` for the WKWebView process
tree). The measurement belongs in CI once the runner image supports it —
until then it's a manual number, so it's stated as a method here rather than
a claim with made-up figures.
