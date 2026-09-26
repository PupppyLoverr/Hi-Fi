//! `hifi` — command-line client for the running Hi-Fi browser.
//!
//! Wire protocol is newline-delimited JSON on `ipc.sock`; see hifi_core::ipc.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hifi_core::HifiPaths;
use hifi_core::ipc::{client, methods};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "hifi", version, about = "Hi-Fi browser CLI")]
struct Cli {
    /// IPC response timeout in seconds.
    #[arg(long, global = true, default_value = "15")]
    timeout: u64,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ping the running browser.
    Ping,
    /// Manage tabs.
    Tab {
        #[command(subcommand)]
        command: TabCommand,
    },
    /// Manage groups.
    Group {
        #[command(subcommand)]
        command: GroupCommand,
    },
    /// Manage spaces.
    Space {
        #[command(subcommand)]
        command: SpaceCommand,
    },
    /// Create a git worktree dev group.
    Worktree {
        /// Git branch name for the new worktree.
        branch: String,
        /// Repository path.
        #[arg(long)]
        project_path: Option<String>,
    },
    /// Open a new window.
    Window,
    /// Right-dock surfaces (cosmos right-pane).
    Dock {
        #[command(subcommand)]
        command: DockCommand,
    },
    /// Automation helpers (snapshot/click/type/screenshot).
    Browser {
        #[command(subcommand)]
        command: BrowserCommand,
    },
}

#[derive(Subcommand)]
enum TabCommand {
    /// Open a URL or internal scheme.
    Open {
        url: String,
        /// Open as a new leaf beside this tab id.
        #[arg(long)]
        right_of: Option<String>,
        #[arg(long)]
        left_of: Option<String>,
        #[arg(long)]
        above: Option<String>,
        #[arg(long)]
        below: Option<String>,
        /// Split size for the new pane, e.g. "40%".
        #[arg(long)]
        size: Option<String>,
        /// Target group id.
        #[arg(long)]
        group: Option<String>,
        /// Working directory for terminal/agent/diff tabs.
        #[arg(long)]
        project_path: Option<String>,
        /// Pane kind override: web|terminal|agent|diff|preview|newtab|settings.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Shell/harness command for terminal/agent tabs.
        #[arg(long)]
        command: Option<String>,
        /// Open without focusing.
        #[arg(long)]
        background: bool,
    },
    /// Close a tab.
    Close {
        id: String,
    },
    /// List all tabs (JSON).
    List,
    /// Focus a tab.
    Focus {
        id: String,
    },
    /// Reload a web tab.
    Reload {
        id: String,
    },
    /// Navigate a web tab to a new URL.
    Navigate {
        id: String,
        url: String,
    },
    /// Navigate back/forward.
    Back {
        id: String,
    },
    Forward {
        id: String,
    },
    /// Toggle pinned.
    Pin {
        id: String,
    },
    /// Evaluate JS in a web tab (returns JSON result).
    Exec {
        id: String,
        js: String,
    },
}

#[derive(Subcommand)]
enum GroupCommand {
    List,
    Create {
        name: String,
        #[arg(long)]
        project_path: Option<String>,
    },
}

#[derive(Subcommand)]
enum SpaceCommand {
    List,
    Create { name: String },
    Switch { id: String },
}

#[derive(Subcommand)]
enum DockCommand {
    /// Open a surface in the dock (hifi://terminal|notes|diff or a URL).
    Open { url: Option<String> },
    /// Move an existing tab into the dock.
    Tab { id: String },
    /// Move a docked tab back into the split tree.
    Undock { id: String },
    /// Show/hide the dock.
    Toggle,
    /// List docked tabs (JSON).
    List,
}

#[derive(Subcommand)]
enum BrowserCommand {
    /// Accessibility-tree snapshot of a web tab (agents read this).
    Snapshot { id: String },
    /// Click an element by snapshot ref or CSS selector.
    Click { id: String, target: String },
    /// Type text into an element.
    Type {
        id: String,
        target: String,
        text: String,
    },
    /// PNG screenshot of a web tab → path printed.
    Screenshot { id: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = HifiPaths::detect();
    let socket = paths.socket_file();
    let t = cli.timeout;

    let (method, params) = match cli.command {
        Command::Ping => (methods::PING, json!({})),
        Command::Tab { command } => match command {
            TabCommand::Open {
                url,
                right_of,
                left_of,
                above,
                below,
                size,
                group,
                project_path,
                kind,
                command,
                background,
            } => (
                methods::TAB_OPEN,
                json!({
                    "url": url,
                    "rightOf": right_of,
                    "leftOf": left_of,
                    "above": above,
                    "below": below,
                    "size": size,
                    "group": group,
                    "projectPath": project_path,
                    "kind": kind,
                    "command": command,
                    "background": background,
                }),
            ),
            TabCommand::Close { id } => (methods::TAB_CLOSE, json!({"id": id})),
            TabCommand::List => (methods::TAB_LIST, json!({})),
            TabCommand::Focus { id } => (methods::TAB_FOCUS, json!({"id": id})),
            TabCommand::Reload { id } => (methods::TAB_RELOAD, json!({"id": id})),
            TabCommand::Navigate { id, url } => {
                (methods::TAB_NAVIGATE, json!({"id": id, "url": url}))
            }
            TabCommand::Back { id } => (methods::TAB_BACK, json!({"id": id})),
            TabCommand::Forward { id } => (methods::TAB_FORWARD, json!({"id": id})),
            TabCommand::Pin { id } => (methods::TAB_PIN, json!({"id": id})),
            TabCommand::Exec { id, js } => (methods::TAB_EXEC, json!({"id": id, "js": js})),
        },
        Command::Group { command } => match command {
            GroupCommand::List => (methods::GROUP_LIST, json!({})),
            GroupCommand::Create { name, project_path } => (
                methods::GROUP_CREATE,
                json!({"name": name, "projectPath": project_path}),
            ),
        },
        Command::Space { command } => match command {
            SpaceCommand::List => (methods::SPACE_LIST, json!({})),
            SpaceCommand::Create { name } => (methods::SPACE_CREATE, json!({"name": name})),
            SpaceCommand::Switch { id } => (methods::SPACE_SWITCH, json!({"id": id})),
        },
        Command::Worktree {
            branch,
            project_path,
        } => (
            methods::WORKTREE_CREATE,
            json!({"branch": branch, "projectPath": project_path}),
        ),
        Command::Window => (methods::WINDOW_NEW, json!({})),
        Command::Dock { command } => match command {
            DockCommand::Open { url } => (
                methods::DOCK_OPEN,
                json!({"url": url.unwrap_or_else(|| "hifi://terminal".into())}),
            ),
            DockCommand::Tab { id } => (methods::DOCK_TAB, json!({"id": id})),
            DockCommand::Undock { id } => (methods::DOCK_UNDOCK, json!({"id": id})),
            DockCommand::Toggle => (methods::DOCK_TOGGLE, json!({})),
            DockCommand::List => (methods::DOCK_LIST, json!({})),
        },
        Command::Browser { command } => match command {
            BrowserCommand::Snapshot { id } => (methods::TAB_SNAPSHOT, json!({"id": id})),
            BrowserCommand::Click { id, target } => {
                (methods::TAB_CLICK, json!({"id": id, "target": target}))
            }
            BrowserCommand::Type { id, target, text } => (
                methods::TAB_TYPE,
                json!({"id": id, "target": target, "text": text}),
            ),
            BrowserCommand::Screenshot { id } => (methods::TAB_SCREENSHOT, json!({"id": id})),
        },
    };

    let response =
        client::call(&socket, method, params, t).with_context(|| format!("calling {method}"))?;
    if !response.ok {
        anyhow::bail!(response.error.unwrap_or_else(|| "unknown error".into()));
    }
    match response.result {
        Some(Value::Null) | None => {}
        Some(Value::String(s)) => println!("{s}"),
        Some(v) => println!("{}", serde_json::to_string_pretty(&v)?),
    }
    Ok(())
}
