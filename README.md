<p align="center"><img src="Resources/icon-1024.png" width="112" alt="Hi-Fi"></p>

<h1 align="center">Hi-Fi</h1>

<p align="center"><b>The browser that does the work.</b><br>
A native browser, a Notion-style workspace and a runtime for Claude Code, Codex, OpenCode and eight more agents, in one app of about 10 MB.</p>

<p align="center">
<a href="https://github.com/PupppyLoverr/Hi-Fi/releases">Download</a> ·
<a href="site/index.html">Website</a> ·
<a href="docs/PLANS.md">Plans</a> ·
<a href="PLATFORM.md">Platform notes</a>
</p>

![Hi-Fi: sidebar, an agent chat with tool steps, and a live page side by side](site/assets/app.jpg)

## Why Hi-Fi

Most work now happens across a browser, a notes app, a terminal and an AI chat. Hi-Fi puts them in one window and lets agents do the busywork in their own tabs while you keep browsing.

- **Browser.** Tabs, groups, spaces, nested splits, a right-hand dock, a ⌘K command bar, real favicons and a frosted sidebar. It uses the system engine (WebKit on macOS and Linux, WebView2 on Windows), so there's no bundled Chromium.
- **Workspace.** Pages are a block editor with `/` commands, Markdown shortcuts, to-dos and code blocks. Terminals are real PTYs. Git worktree groups and diff panes sit next to the app you're building.
- **Agents.** Ask from the New Tab page and the harness you pick runs as a real process. It browses in hidden tabs that belong to it, edits files and runs commands in the folder you choose, and each step streams into a collapsible timeline.
- **Documents.** Agents create PDF, Word, PowerPoint and Excel files from one JSON schema. Open any of them in a tab to preview, edit and save.

## Agents

Hi-Fi supports every production harness that Cosmos does:

| Harness | Protocol |
|---|---|
| Claude Code, Amp | stream-json |
| Codex | `exec --json` |
| OpenCode, Cursor, Devin, Grok, Hermes, Pi, Antigravity, Kimi Code | ACP |

Install a harness CLI and sign in to it the usual way. Hi-Fi finds it on your `PATH` and uses your existing subscription or API key. OpenCode works with free models out of the box.

Every run gets the Hi-Fi MCP server, which provides:

- **Browser tools:** `browser_open`, `browser_read`, `browser_snapshot`, `browser_click`, `browser_type`, `browser_screenshot` and more, scoped to tabs the agent opened. Agents can't read or drive your own tabs.
- **Document tools:** `doc_create`, `doc_read` and `doc_open`. See [`skills/documents/SKILL.md`](skills/documents/SKILL.md).

Each chat has a **guard** level:

| Guard | Allows |
|---|---|
| Strict | Reading and browsing only; no edits or shell commands |
| Balanced | Edits and commands; blocks destructive ones such as `rm -rf`, `sudo`, `git push`, `git reset --hard` |
| Full | Everything the harness can do |

Run several agents at once by opening chats in splits. Each one has its own tabs, folder and guard.

## Install

Download a build from [Releases](https://github.com/PupppyLoverr/Hi-Fi/releases), or build from source with Rust stable:

```sh
git clone https://github.com/PupppyLoverr/Hi-Fi
cd Hi-Fi
cargo build --release --workspace
./target/release/hifi-app        # the app
./target/release/hifi            # the CLI; put it on your PATH
./scripts/bundle.sh              # macOS: build/Hi-Fi.app
```

Linux needs the WebKitGTK and GTK development packages (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`). Windows needs the WebView2 runtime, which ships with Windows 10 and 11.

## Keyboard

| Key | Action |
|---|---|
| ⌘K or ⌘T | Command bar: URLs, open tabs, actions |
| ⌘L | Edit the current tab's address |
| ⌘W | Close tab |
| ⌘⇧] and ⌘⇧[ | Next and previous tab |
| ⌘1 to ⌘9 | Jump to a tab |
| ⌘\ | Toggle the sidebar |
| ⌥⌘B | Toggle the dock |

On Windows and Linux, use Ctrl instead of ⌘.

Internal pages: `hifi://newtab`, `hifi://agent`, `hifi://notes`, `hifi://terminal`, `hifi://diff`, `hifi://settings`, `hifi://preview/<path>`.

## CLI

`hifi` talks to the running app over a local socket that only your user can open, authenticated with a token generated at each launch.

```sh
hifi tab open https://github.com --right-of <tab> --size 40%
hifi tab list
hifi browser snapshot <tab>
hifi browser click <tab> @e3
hifi worktree create ~/code/app feature-x
hifi doc create report.docx --json spec.json
hifi doc open report.docx
hifi mcp                          # MCP stdio server for any agent
```

## Architecture

```
hifi-app    GPUI shell (zui, the GPUI fork Cosmos uses): sidebar, panes, dock,
            native web views, terminal, agent runner, document editor
hifi-core   models, split trees, routing, persistence, IPC protocol
hifi-docs   PDF / DOCX / PPTX / XLSX writer, reader and editor page
hifi-cli    the hifi CLI and MCP server
hifi-webkit Linux WebKitGTK helper
```

- Web pages are native views composited under GPUI's overlay, so menus and chrome always draw on top and splits are just clip rects.
- State is plain JSON in your app data folder (`~/Library/Application Support/HiFi` on macOS). Pages, chats and transcripts never leave your machine.
- Release builds use fat LTO, a single codegen unit and stripped symbols. On Apple silicon the app bundle is about 10 MB and the app process uses about 40 MB at idle; the system WebKit processes are extra.

## Security

- The IPC socket is `0600`, needs a per-launch token and caps request size.
- Agents can only open, read and drive tabs they created.
- Navigation to `file:`, `javascript:` and `data:` URLs is blocked before it reaches tab state.
- Guard levels are enforced in each harness's own permission config, not just in the UI.

## Contributing

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

CI runs these on macOS, Ubuntu and Windows for every pull request.

## License

MIT. See [LICENSE](LICENSE).
