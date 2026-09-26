# Platform port map

Hi-Fi's platform surface is thin by construction: `hifi-core` is pure Rust
(models, split trees, IPC protocol, the `hifi` CLI — zero UI or engine deps)
and `hifi-app` is a GPUI shell. GPUI itself is cross-platform, and wry ships a
system-webview per OS — so a port is a host, not a rewrite.

## macOS (shipping)

- **UI**: GPUI (`zui` fork, same pin as cosmos) → Metal.
- **Engine**: `wry 0.56` → `WKWebView` via `build_as_child`.
- **Compositing**: `window.enable_scene_overlay()` creates `GPUIOverlayView`;
  each webview is reparented into a clip `NSView` positioned *below* the
  overlay, sized by a `CALayer` mask whose frame is the pane's painted rect
  (`crates/hifi-app/src/webview.rs::sync_bounds`). Chrome drawn by GPUI always
  sits above native page content.
- **Terminal**: `alacritty_terminal` grid → `gpui::canvas`.
- **Persistence/IPC**: `~/Library/Application Support/HiFi/` (`state.json`,
  `ipc.sock`).

Known wry quirk handled here: `WebView::eval` queues into `pending_scripts`
until the first navigation finishes and **drops the callback** — so Hi-Fi calls
`WKWebView::evaluateJavaScript` on the view directly (`WebPaneHost::eval`).

## Linux

- **UI**: same GPUI shell (Wayland/X11, Vulkan).
- **Engine**: wry → WebKitGTK (`WebViewBuilder::build_gtk`). The clip/mask
  dance is unnecessary — WebKitGTK embeds as a GTK widget inside the GPUI
  window via a fixed container; hide/show by widget visibility, no
  overlay-ordering needed.
- **Deps**: `libwebkit2gtk-4.1`, `libgtk-3`, `libsoup-3`.

## Windows

- **UI**: same GPUI shell (Direct3D11).
- **Engine**: **WebKit or Gecko — never WebView2/Chromium.** Practical path:
  `wry` is WebView2-only, so the host is a small native child-window hosting a
  WebKit/Gecko build (e.g. `playwright-webkit` runtime or `GeckoView`
  embedding), synced to pane bounds with `SetWindowPos` from the same
  `sync_bounds` call site (GPUI `on_present` gives the rect; Windows doesn't
  need the mask trick — child windows are clipped by their parent).

## What does NOT change per platform

- `SplitNode` model, spaces/groups/tabs, `state.json` schema.
- IPC wire protocol + every `hifi` CLI subcommand.
- Keybindings, command bar actions, `hifi://` scheme handling.
- Solar icon set, theme tokens, Geist Mono (all bundled assets).
