---
name: testing-hifi-gpui
description: How to build, run and end-to-end test the Rust + GPUI rewrite of Hi-Fi (hifi-app) on the macOS VM, including keybindings, state files and known UI-automation gotchas.
---

# Testing Hi-Fi (Rust + GPUI) on macOS

## Build and run
- `$HOME/.cargo/bin` may not exist; cargo may only be available at
  `$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin`. Add that to PATH.
- `cargo build -p hifi-app` builds `target/debug/hifi-app` and `target/debug/hifi` (CLI).
- Restart with: `pkill -f hifi-app; RUST_BACKTRACE=1 nohup target/debug/hifi-app > /tmp/hifi.log 2>&1 &`
  so panics and backtraces end up in `/tmp/hifi.log`.
- Before testing, check `pgrep -fl hifi` and make sure only one app instance is running.

## State
- Session/settings: `~/Library/Application Support/HiFi/state.json`; IPC socket `ipc.sock` in the same directory.
- A tab that crashes on render (e.g. Diff) is restored on every launch. To recover, remove that tab id from state.json while the app is not running.
- Theme/accent may only apply at startup (`Theme::install` in main.rs). If a live change does nothing, restart to confirm it was persisted.

## Keybindings (crates/hifi-app/src/main.rs `bind_keys`, `secondary-` = Cmd on macOS)
Cmd+T command bar, Cmd+L address, Cmd+[ / Cmd+] back/forward, Cmd+R reload,
Alt+Cmd+B right dock, Cmd+Shift+N notes page, Cmd+, settings, Cmd+Q quit.
Also test keys while a native WKWebView has focus: they are forwarded back to GPUI via `WebEvent::Keystroke`, and that path has caused re-entrancy panics before.

## Flows and tips
- Splits: `target/debug/hifi tab open <url> --right-of <id> --size 40%`. Drag the divider with left_mouse_down, several mouse_move steps, then take a screenshot while the button is still held.
- Terminal: open it from the sidebar, click inside it and type. Check that every row renders, not just row 0. With a dock Notes page open, clicking the terminal must still give it focus.
- Diff: make a temporary edit to a tracked file (e.g. append to README.md) and confirm it appears. Revert with `git checkout README.md`. The Diff view does not auto-refresh; use its refresh icon.
- Dock: resize by dragging the narrow GPUI gutter exactly on the dock's left edge (shell.rs `DockResize`). Zoom in on the edge first to find it.
- Maximise the window: osascript window control is not permitted, so Option-click the green traffic-light button.
- Before testing, check `pgrep -fl hifi` for other builds (e.g. `target-redesign/debug/hifi-app`) and kill them.
- Agent harness binaries (claude, codex, opencode...) are not installed on the VM. The agent tab shows "Failed to spawn command"; that is expected.
- After resizing a split that contains a web pane, check that the web content reflows to the new width and is not just clipped.
- Leftover dock tabs (earlier Notes pages) persist across launches, so tab positions shift. Re-screenshot before clicking.
