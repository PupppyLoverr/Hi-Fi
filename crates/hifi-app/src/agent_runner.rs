//! Real agent execution: spawns a CLI harness (OpenCode first-class, any other
//! harness as a plain-text stream) in the chat's working folder and turns
//! its output into transcript events.
//!
//! The harness gets Hi-Fi's browser through the `hifi mcp` server, scoped to
//! the chat tab (`HIFI_AGENT_TAB`), so every page it opens is an agent-owned
//! hidden tab — the user's own panes never move. File edits and shell access
//! are gated by the chat's [`Guard`] level through the harness' own permission
//! config.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use futures::channel::mpsc::UnboundedSender;
use hifi_core::{Guard, HifiPaths};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenCode's JSON event stream.
    Opencode,
    /// Claude-compatible stream JSON output.
    ClaudeStream,
    /// Codex `exec --json` JSONL output.
    CodexExec,
    /// Agent Client Protocol over newline-delimited JSON-RPC.
    Acp,
}

pub struct Harness {
    /// Stable identifier persisted in chat tabs and settings.
    pub id: &'static str,
    /// Human-readable name shown in the harness picker.
    pub label: &'static str,
    /// Executable searched for in PATH and known installer directories.
    pub executable: &'static str,
    /// Arguments selecting the harness' protocol mode.
    pub args: &'static [&'static str],
    /// Output protocol used by the runner.
    pub protocol: Protocol,
    /// Models advertised in the picker for this harness.
    pub models: &'static [&'static str],
    /// Friendly installation command or documentation hint.
    pub install_hint: &'static str,
}

pub const HARNESSES: &[Harness] = &[
    Harness {
        id: "claude",
        label: "Claude Code",
        executable: "claude",
        args: &[],
        protocol: Protocol::ClaudeStream,
        models: &["sonnet", "opus", "haiku"],
        install_hint: "npm i -g @anthropic-ai/claude-code",
    },
    Harness {
        id: "codex",
        label: "Codex",
        executable: "codex",
        args: &[],
        protocol: Protocol::CodexExec,
        models: &["gpt-5-codex", "gpt-5"],
        install_hint: "npm i -g @openai/codex",
    },
    Harness {
        id: "cursor",
        label: "Cursor",
        executable: "cursor-agent",
        args: &["acp"],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "curl https://cursor.com/install -fsS | bash",
    },
    Harness {
        id: "devin",
        label: "Devin",
        executable: "devin",
        args: &["acp"],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "install the Devin CLI (docs.devin.ai)",
    },
    Harness {
        id: "grok",
        label: "Grok",
        executable: "grok",
        args: &["--no-auto-update", "agent", "--no-leader", "stdio"],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "npm i -g @xai-official/grok",
    },
    Harness {
        id: "hermes",
        label: "Hermes",
        executable: "hermes",
        args: &["acp"],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "install Hermes Agent (hermes-agent.nousresearch.com)",
    },
    Harness {
        id: "pi",
        label: "Pi",
        executable: "pi-acp",
        args: &[],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "npm i -g pi-acp",
    },
    Harness {
        id: "opencode",
        label: "OpenCode",
        executable: "opencode",
        args: &[],
        protocol: Protocol::Opencode,
        models: &[
            "opencode/big-pickle",
            "opencode/mimo-v2.6-flash-free",
            "opencode/nemotron-3-ultra-free",
            "opencode/nemotron-3.5-lightning-free",
            "opencode/longcat-2.5-preview-free",
            "opencode/ling-3.0-flash-fin-free",
            "opencode/muse-spark-1.3-contributor-free",
            "anthropic/claude-sonnet-4-5",
            "openai/gpt-5",
        ],
        install_hint: "curl -fsSL https://opencode.ai/install | bash",
    },
    Harness {
        id: "antigravity",
        label: "Antigravity",
        executable: "agy_acp_server",
        args: &[],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "install Antigravity's agy_acp_server",
    },
    Harness {
        id: "amp",
        label: "Amp",
        executable: "amp",
        args: &[],
        protocol: Protocol::ClaudeStream,
        models: &["default"],
        install_hint: "npm i -g @sourcegraph/amp",
    },
    Harness {
        id: "kimi",
        label: "Kimi Code",
        executable: "kimi",
        args: &["acp"],
        protocol: Protocol::Acp,
        models: &["default"],
        install_hint: "npm i -g @moonshot-ai/kimi-cli",
    },
];

pub fn harness(id: &str) -> &'static Harness {
    HARNESSES
        .iter()
        .find(|h| h.id == id)
        .unwrap_or_else(|| HARNESSES.iter().find(|h| h.id == "opencode").unwrap())
}

pub fn model_short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Harness session id — pass back as `session` to continue the thread.
    Session(String),
    /// A harness tool invocation has started.
    ToolStart {
        id: String,
        tool: String,
        title: String,
    },
    /// A harness tool invocation has completed.
    ToolDone {
        id: String,
        tool: String,
        title: String,
        output: String,
        error: Option<String>,
    },
    /// Assistant text for a part; replaces earlier text with the same id.
    Text { id: String, text: String },
    /// Harness exited. `error` is the stderr tail when it failed.
    Done { error: Option<String> },
}

#[derive(Clone)]
pub struct RunConfig {
    pub harness: String,
    pub model: String,
    pub cwd: String,
    pub guard: Guard,
    pub session: String,
    pub prompt: String,
    /// Owning chat tab: browser tools open tabs under it.
    pub tab_id: String,
    pub first_turn: bool,
}

pub struct Run {
    pub(crate) child: Arc<Mutex<Option<Child>>>,
    pub(crate) stop_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Run {
    pub fn stop(&self) {
        if let Some(hook) = &self.stop_hook {
            hook();
        }
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut child) = guard.take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Directories harness installers drop binaries into, on top of `PATH`.
fn extra_bin_dirs() -> Vec<PathBuf> {
    let h = home();
    let mut dirs = vec![
        h.join(".opencode/bin"),
        h.join(".claude/local"),
        h.join(".cursor/bin"),
        h.join(".grok/bin"),
        h.join(".hermes/bin"),
        h.join(".kimi-code/bin"),
        h.join(".local/bin"),
        h.join(".cargo/bin"),
        h.join(".bun/bin"),
        h.join(".npm-global/bin"),
        h.join("scoop/shims"),
        h.join(".fnm/aliases/default/bin"),
        h.join(".nvm/current/bin"),
        h.join(".volta/bin"),
        h.join(".config/yarn/global/node_modules/.bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    for variable in ["NVM_SYMLINK", "VOLTA_HOME", "PNPM_HOME"] {
        if let Some(path) = std::env::var_os(variable) {
            dirs.push(PathBuf::from(path));
        }
    }
    if let Ok(fnm_dir) = std::env::var("FNM_DIR") {
        dirs.push(PathBuf::from(fnm_dir).join("aliases/default/bin"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.extend([
            local.join("Programs/cursor"),
            local.join("cursor-agent"),
            local.join("Programs/Devin/bin"),
            local.join("Programs/Devin/resources/app/extensions/windsurf/devin/bin"),
            local.join("hermes/bin"),
        ]);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.insert(0, dir.to_path_buf());
    }
    dirs
}

/// `PATH` with the extra bin dirs prepended so harnesses and `hifi` resolve
/// even when the app was launched from Finder (no shell profile).
pub(crate) fn search_path() -> String {
    let mut parts: Vec<String> = extra_bin_dirs()
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if let Ok(path) = std::env::var("PATH") {
        parts.push(path);
    }
    parts.join(if cfg!(windows) { ";" } else { ":" })
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    let path_dirs = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    for dir in extra_bin_dirs().into_iter().chain(path_dirs) {
        let exact = dir.join(name);
        if exact.is_file() {
            return Some(exact);
        }
        if cfg!(windows) {
            for suffix in ["exe", "cmd"] {
                let candidate = dir.join(format!("{name}.{suffix}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn installed(h: &Harness) -> bool {
    find_binary(h.executable).is_some() || (h.id == "pi" && find_binary("pi").is_some())
}

pub fn resolve(h: &Harness) -> Result<(PathBuf, Vec<String>), String> {
    if let Some(path) = find_binary(h.executable) {
        let mut args: Vec<String> = h.args.iter().map(|arg| (*arg).to_string()).collect();
        if h.id == "antigravity" && cfg!(target_os = "linux") {
            args.push("--uid=".into());
        }
        return Ok((path, args));
    }
    if h.id == "pi"
        && find_binary("pi").is_some()
        && let Some(npx) = find_binary("npx")
    {
        return Ok((npx, vec!["-y".into(), "pi-acp@0.0.33".into()]));
    }
    Err(format!("{} isn't installed ({})", h.label, h.install_hint))
}

pub const DESTRUCTIVE_PATTERNS: &[&str] = &[
    "rm -r",
    "rm -f",
    "sudo ",
    "git push",
    "git reset --hard",
    "git clean",
    "mkfs",
    "dd ",
    "chmod -R",
    "> /dev/",
];

/// Return whether a shell-like command contains a destructive operation.
///
/// Matching command segments instead of arbitrary substrings keeps `dd ` from
/// matching words such as `add `.
pub fn is_destructive(text: &str) -> bool {
    text.split(['\n', ';', '&', '|'])
        .map(str::trim)
        .any(|segment| {
            DESTRUCTIVE_PATTERNS
                .iter()
                .any(|pattern| segment.starts_with(pattern))
                || segment.contains("> /dev/")
        })
}

/// OpenCode config for a guard level: browser tools via `hifi mcp`, file and
/// shell access per level. `--auto` approves anything not denied here.
pub fn opencode_config(guard: Guard, hifi_bin: &Path, tab_id: &str) -> Value {
    let permission = match guard {
        Guard::Strict => json!({
            // The bash tool stays registered (OpenCode's free tier rejects
            // requests without it) but nothing except a bare `pwd` may run;
            // reads go through the read/glob/grep tools.
            "*": "allow",
            "edit": "deny",
            "write": "deny",
            "patch": "deny",
            "bash": { "*": "deny", "pwd": "allow" },
        }),
        Guard::Balanced => {
            let mut bash = serde_json::Map::new();
            bash.insert("*".into(), json!("allow"));
            for pattern in DESTRUCTIVE_PATTERNS {
                bash.insert(format!("{pattern}*"), json!("deny"));
            }
            json!({ "*": "allow", "bash": bash })
        }
        Guard::Full => json!({ "*": "allow" }),
    };
    json!({
        "$schema": "https://opencode.ai/config.json",
        "permission": permission,
        "mcp": {
            "hifi": {
                "type": "local",
                "command": [hifi_bin, "mcp"],
                "enabled": true,
                "environment": { "HIFI_AGENT_TAB": tab_id }
            }
        }
    })
}

fn write_opencode_config(config: &Value, tab_id: &str) -> Result<PathBuf, String> {
    let dir = HifiPaths::detect().root.join("opencode").join(tab_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let path = dir.join("opencode.json");
    std::fs::write(
        &path,
        serde_json::to_vec(config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
}

fn mcp_json(hifi: &Path, tab: &str) -> String {
    json!({"mcpServers":{"hifi":{"command":hifi,"args":["mcp"],"env":{"HIFI_AGENT_TAB":tab}}}})
        .to_string()
}

pub fn claude_args(cfg: &RunConfig, hifi: &Path) -> Vec<String> {
    let mut args = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--include-partial-messages".into(),
    ];
    if !cfg.model.is_empty() {
        args.extend(["--model".into(), cfg.model.clone()]);
    }
    if !cfg.session.is_empty() {
        args.extend(["--resume".into(), cfg.session.clone()]);
    }
    match cfg.guard {
        Guard::Full => args.push("--dangerously-skip-permissions".into()),
        Guard::Balanced => {
            args.extend(["--permission-mode".into(), "acceptEdits".into()]);
            for pattern in DESTRUCTIVE_PATTERNS {
                args.extend(["--disallowedTools".into(), format!("Bash({pattern}*)")]);
            }
        }
        Guard::Strict => {
            args.extend(["--permission-mode".into(), "plan".into()]);
            for tool in ["Edit", "Write", "MultiEdit", "Bash"] {
                args.extend(["--disallowedTools".into(), tool.into()]);
            }
        }
    }
    args.extend([
        "--mcp-config".into(),
        mcp_json(hifi, &cfg.tab_id),
        cfg.prompt.clone(),
    ]);
    args
}

pub fn amp_args(cfg: &RunConfig, hifi: &Path) -> Vec<String> {
    let mut args = vec![
        "--execute".into(),
        "--stream-json".into(),
        "--mcp-config".into(),
        mcp_json(hifi, &cfg.tab_id),
    ];
    if cfg.guard == Guard::Full {
        args.push("--dangerously-allow-all".into());
    }
    args.push(cfg.prompt.clone());
    args
}

pub fn codex_args(cfg: &RunConfig, hifi: &Path) -> Vec<String> {
    let sandbox = match cfg.guard {
        Guard::Strict => "read-only",
        Guard::Balanced => "workspace-write",
        Guard::Full => "danger-full-access",
    };
    let mut args = vec![
        "exec".into(),
        "--json".into(),
        "--sandbox".into(),
        sandbox.into(),
        "-a".into(),
        "never".into(),
        "--skip-git-repo-check".into(),
    ];
    if !cfg.model.is_empty() && cfg.model != "default" {
        args.extend(["-m".into(), cfg.model.clone()]);
    }
    args.extend([
        "-c".into(),
        format!("mcp_servers.hifi.command=\"{}\"", hifi.display()),
        "-c".into(),
        "mcp_servers.hifi.args=[\"mcp\"]".into(),
        "-c".into(),
        format!(
            "mcp_servers.hifi.env={{ HIFI_AGENT_TAB=\"{}\" }}",
            cfg.tab_id
        ),
    ]);
    if !cfg.session.is_empty() {
        args.extend(["resume".into(), cfg.session.clone()]);
    }
    args.push(cfg.prompt.clone());
    args
}

/// What the harness is told about where it runs, once per chat.
fn preamble(cwd: &str, guard: Guard) -> String {
    format!(
        "You are running inside the Hi-Fi browser as its agent. Working folder: {cwd}. \
Permission level: {} ({}). To use the web, call the hifi browser_* tools: browser_open to \
open a page in your own tab (the user's tabs are never touched), then browser_read for the \
text, browser_snapshot for clickable elements, browser_click / browser_type / browser_scroll \
to interact. Keep answers concise.\n\nTask: ",
        guard.label(),
        guard.describe()
    )
}

/// Spawn the harness. Events arrive on `tx` from a reader thread; the returned
/// [`Run`] kills the process on stop.
pub fn spawn(mut cfg: RunConfig, tx: UnboundedSender<AgentEvent>) -> Result<Run, String> {
    let selected = harness(&cfg.harness);
    let (bin, protocol_args) = resolve(selected)?;
    let cwd = if cfg.cwd.is_empty() {
        home()
    } else {
        PathBuf::from(&cfg.cwd)
    };
    if cfg.first_turn {
        cfg.prompt = format!(
            "{}{}",
            preamble(&cwd.display().to_string(), cfg.guard),
            cfg.prompt
        );
    }
    if selected.protocol == Protocol::Acp {
        return crate::acp::spawn(cfg, bin, protocol_args, tx);
    }
    let hifi = find_binary("hifi").unwrap_or_else(|| PathBuf::from("hifi"));
    let mut command = Command::new(&bin);
    command
        .current_dir(&cwd)
        .env("PATH", search_path())
        .env("HIFI_AGENT_TAB", &cfg.tab_id)
        .env_remove("CLOUDFLARE_API_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if selected.protocol == Protocol::Opencode {
        command.arg("run").arg("--format").arg("json");
        if !cfg.model.is_empty() {
            command.arg("-m").arg(&cfg.model);
        }
        if !cfg.session.is_empty() {
            command.arg("--session").arg(&cfg.session);
        }
        command.arg("--auto");
        let path =
            write_opencode_config(&opencode_config(cfg.guard, &hifi, &cfg.tab_id), &cfg.tab_id)?;
        command.env("OPENCODE_CONFIG", path).arg(&cfg.prompt);
    } else if selected.id == "amp" {
        command.args(amp_args(&cfg, &hifi));
    } else if selected.protocol == Protocol::ClaudeStream {
        command.args(claude_args(&cfg, &hifi));
    } else if selected.protocol == Protocol::CodexExec {
        command.args(codex_args(&cfg, &hifi));
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", bin.display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let wait_child = child.clone();
    let stderr_tail = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = stderr {
        let tail = stderr_tail.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut value) = tail.lock() {
                    value.push_str(&line);
                    value.push('\n');
                    if value.len() > 4000 {
                        let start = value.len() - 4000;
                        value.drain(..start);
                    }
                }
            }
        });
    }
    let protocol = selected.protocol;
    thread::spawn(move || {
        if let Some(stdout) = stdout {
            let mut plain_id = 0;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let events = match protocol {
                    Protocol::Opencode => serde_json::from_str::<Value>(&line)
                        .map(|v| events_from_opencode(&v))
                        .unwrap_or_default(),
                    Protocol::ClaudeStream => serde_json::from_str::<Value>(&line)
                        .map(|v| events_from_claude(&v))
                        .unwrap_or_default(),
                    Protocol::CodexExec => serde_json::from_str::<Value>(&line)
                        .map(|v| events_from_codex(&v))
                        .unwrap_or_default(),
                    Protocol::Acp => Vec::new(),
                };
                if events.is_empty() && protocol == Protocol::Opencode && !line.trim().is_empty() {
                    plain_id += 1;
                    if tx
                        .unbounded_send(AgentEvent::Text {
                            id: format!("line-{plain_id}"),
                            text: line,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                for event in events {
                    if tx.unbounded_send(event).is_err() {
                        return;
                    }
                }
            }
        }
        let status = wait_child
            .lock()
            .ok()
            .and_then(|mut guard| guard.take())
            .and_then(|mut c| c.wait().ok());
        let error = if status.is_some_and(|s| !s.success()) {
            let tail = stderr_tail
                .lock()
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            Some(if tail.is_empty() {
                "harness exited with an error".into()
            } else {
                tail
            })
        } else {
            None
        };
        let _ = tx.unbounded_send(AgentEvent::Done { error });
    });
    Ok(Run {
        child,
        stop_hook: None,
    })
}

pub fn events_from_opencode(v: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    if let Some(session) = v.get("sessionID").and_then(Value::as_str) {
        out.push(AgentEvent::Session(session.into()));
    }
    if v.get("type").and_then(Value::as_str) == Some("error") {
        let e = v.get("error");
        let message = e
            .and_then(|v| v.pointer("/data/message"))
            .or_else(|| e.and_then(|v| v.get("message")))
            .or_else(|| e.and_then(|v| v.get("name")))
            .and_then(Value::as_str)
            .unwrap_or("harness reported an error");
        out.push(AgentEvent::Done {
            error: Some(message.into()),
        });
        return out;
    }
    let Some(part) = v.get("part") else {
        return out;
    };
    let string = |key: &str| {
        part.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match v.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" => {
            let text = string("text");
            if !text.is_empty() {
                out.push(AgentEvent::Text {
                    id: string("id"),
                    text,
                });
            }
        }
        "tool_use" => {
            let tool = string("tool");
            let id = if part.get("callID").is_some() {
                string("callID")
            } else {
                string("id")
            };
            let state = part.get("state").cloned().unwrap_or(Value::Null);
            let title = state
                .get("title")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| tool_title(&tool, state.get("input")));
            match state.get("status").and_then(Value::as_str) {
                Some("completed") => out.push(AgentEvent::ToolDone {
                    id,
                    tool,
                    title,
                    output: state
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    error: None,
                }),
                Some("error") => out.push(AgentEvent::ToolDone {
                    id,
                    tool,
                    title,
                    output: String::new(),
                    error: Some(
                        state
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("tool failed")
                            .into(),
                    ),
                }),
                _ => out.push(AgentEvent::ToolStart { id, tool, title }),
            }
        }
        _ => {}
    }
    out
}

pub fn events_from_claude(v: &Value) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    if v.get("type").and_then(Value::as_str) == Some("system")
        && v.get("subtype").and_then(Value::as_str) == Some("init")
        && let Some(session) = v.get("session_id").and_then(Value::as_str)
    {
        events.push(AgentEvent::Session(session.into()));
    }
    if v.get("type").and_then(Value::as_str) == Some("assistant") {
        let message = v.get("message").unwrap_or(&Value::Null);
        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("assistant");
        if let Some(content) = message.get("content").and_then(Value::as_array) {
            for (index, block) in content.iter().enumerate() {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            events.push(AgentEvent::Text {
                                id: format!("{message_id}-{index}"),
                                text: text.into(),
                            });
                        }
                    }
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .into();
                        let tool = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .to_string();
                        events.push(AgentEvent::ToolStart {
                            id,
                            title: tool_title(&tool, block.get("input")),
                            tool,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    if v.get("type").and_then(Value::as_str) == Some("user")
        && let Some(content) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
    {
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                events.push(AgentEvent::ToolDone {
                    id: block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .into(),
                    tool: "tool".into(),
                    title: "tool result".into(),
                    output: block.get("content").map(value_text).unwrap_or_default(),
                    error: (block.get("is_error") == Some(&Value::Bool(true)))
                        .then(|| "tool failed".into()),
                });
            }
        }
    }
    if v.get("type").and_then(Value::as_str) == Some("stream_event") {
        let delta = v
            .pointer("/event/delta")
            .or_else(|| v.get("content_block_delta"));
        if delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta")
            && let Some(text) = delta.and_then(|d| d.get("text")).and_then(Value::as_str)
        {
            let id = v
                .pointer("/event/message/id")
                .or_else(|| v.pointer("/message/id"))
                .and_then(Value::as_str)
                .unwrap_or("stream");
            events.push(AgentEvent::Text {
                id: id.into(),
                text: text.into(),
            });
        }
    }
    if v.get("type").and_then(Value::as_str) == Some("result")
        && v.get("is_error").and_then(Value::as_bool) == Some(true)
    {
        events.push(AgentEvent::Done {
            error: Some(
                v.get("result")
                    .map(value_text)
                    .unwrap_or_else(|| "harness reported an error".into()),
            ),
        });
    }
    events
}

pub fn events_from_codex(v: &Value) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    match v.get("type").and_then(Value::as_str).unwrap_or_default() {
        "thread.started" => {
            if let Some(id) = v.get("thread_id").and_then(Value::as_str) {
                events.push(AgentEvent::Session(id.into()));
            }
        }
        "turn.failed" => events.push(AgentEvent::Done {
            error: Some(
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("turn failed")
                    .into(),
            ),
        }),
        "error" => events.push(AgentEvent::Done {
            error: Some(
                v.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("harness reported an error")
                    .into(),
            ),
        }),
        "item.started" | "item.completed" => {
            let item = v.get("item").unwrap_or(&Value::Null);
            let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("item")
                .into();
            let completed = v.get("type").and_then(Value::as_str) == Some("item.completed");
            match kind {
                "agent_message" => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        events.push(AgentEvent::Text {
                            id,
                            text: text.into(),
                        });
                    }
                }
                "command_execution" => {
                    let title = item
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or("command")
                        .into();
                    if completed {
                        let code = item.get("exit_code").and_then(Value::as_i64);
                        events.push(AgentEvent::ToolDone {
                            id,
                            tool: "command_execution".into(),
                            title,
                            output: item
                                .get("aggregated_output")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .into(),
                            error: code
                                .filter(|c| *c != 0)
                                .map(|c| format!("command exited with status {c}")),
                        });
                    } else {
                        events.push(AgentEvent::ToolStart {
                            id,
                            tool: "command_execution".into(),
                            title,
                        });
                    }
                }
                "mcp_tool_call" => {
                    let title = format!(
                        "{} / {}",
                        item.get("server").and_then(Value::as_str).unwrap_or("mcp"),
                        item.get("tool")
                            .or_else(|| item.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                    );
                    if completed {
                        events.push(AgentEvent::ToolDone {
                            id,
                            tool: "mcp_tool_call".into(),
                            title,
                            output: value_text(item.get("result").unwrap_or(&Value::Null)),
                            error: item.get("error").map(value_text),
                        });
                    } else {
                        events.push(AgentEvent::ToolStart {
                            id,
                            tool: "mcp_tool_call".into(),
                            title,
                        });
                    }
                }
                "file_change" if completed => {
                    let paths = item
                        .get("changes")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(|item| {
                                    item.get("path")
                                        .and_then(Value::as_str)
                                        .or_else(|| item.as_str())
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    events.push(AgentEvent::ToolDone {
                        id,
                        tool: "file_change".into(),
                        title: format!("edit {paths}"),
                        output: String::new(),
                        error: None,
                    });
                }
                _ => {}
            }
        }
        _ => {}
    }
    events
}

pub(crate) fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(value_text).collect::<Vec<_>>().join(""),
        Value::Object(o) => o
            .get("text")
            .map(value_text)
            .unwrap_or_else(|| value.to_string()),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

pub(crate) fn tool_title(tool: &str, input: Option<&Value>) -> String {
    let name = tool.trim_start_matches("hifi_");
    if name == "browser_click" || name == "browser_type" {
        let verb = if name == "browser_click" {
            "Click"
        } else {
            "Type into"
        };
        if let Some(label) = input
            .and_then(|i| i.get("label").or_else(|| i.get("name")))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return format!("{verb} \"{label}\"");
        }
        if let Some(target) = input.and_then(|i| i.get("target")) {
            let index = target.as_u64().map(|n| n.to_string()).or_else(|| {
                target.as_str().and_then(|s| {
                    s.strip_prefix("[data-hifi=\"")
                        .and_then(|s| s.strip_suffix("\"]"))
                        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
                        .map(str::to_string)
                })
            });
            if let Some(index) = index {
                return format!("{verb} element #{index}");
            }
        }
        return format!("{verb} element");
    }
    [
        "command",
        "url",
        "filePath",
        "path",
        "pattern",
        "target",
        "text",
        "description",
    ]
    .iter()
    .find_map(|key| input.and_then(|i| i.get(*key)).and_then(Value::as_str))
    .map(str::to_string)
    .unwrap_or_else(|| name.replace('_', " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::time::Duration;

    #[test]
    fn production_harnesses_are_exactly_cosmos_order() {
        let ids: Vec<_> = HARNESSES.iter().map(|h| h.id).collect();
        assert_eq!(
            ids,
            [
                "claude",
                "codex",
                "cursor",
                "devin",
                "grok",
                "hermes",
                "pi",
                "opencode",
                "antigravity",
                "amp",
                "kimi"
            ]
        );
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            11
        );
        assert!(!ids.contains(&"mock"));
    }

    #[test]
    fn command_builders_apply_permissions_and_mcp() {
        let base = RunConfig {
            harness: "claude".into(),
            model: "sonnet".into(),
            cwd: ".".into(),
            guard: Guard::Strict,
            session: "s".into(),
            prompt: "hello".into(),
            tab_id: "tab-1".into(),
            first_turn: false,
        };
        let strict = claude_args(&base, Path::new("/bin/hifi"));
        assert!(
            strict
                .windows(2)
                .any(|x| x == ["--permission-mode", "plan"])
        );
        assert!(strict.iter().any(|x| x == "Edit"));
        assert!(strict.iter().any(|x| x.contains("HIFI_AGENT_TAB")));
        let mut balanced = base.clone();
        balanced.guard = Guard::Balanced;
        assert!(
            claude_args(&balanced, Path::new("/bin/hifi"))
                .iter()
                .any(|x| x == "acceptEdits")
        );
        let mut full = base;
        full.guard = Guard::Full;
        assert!(
            claude_args(&full, Path::new("/bin/hifi"))
                .iter()
                .any(|x| x == "--dangerously-skip-permissions")
        );
        let codex = codex_args(&balanced, Path::new("/bin/hifi"));
        assert!(
            codex
                .windows(2)
                .any(|x| x == ["--sandbox", "workspace-write"])
        );
        let strict_codex = codex_args(
            &RunConfig {
                guard: Guard::Strict,
                ..balanced.clone()
            },
            Path::new("/bin/hifi"),
        );
        assert!(
            strict_codex
                .windows(2)
                .any(|x| x == ["--sandbox", "read-only"])
        );
        let full_codex = codex_args(
            &RunConfig {
                guard: Guard::Full,
                ..balanced.clone()
            },
            Path::new("/bin/hifi"),
        );
        assert!(
            full_codex
                .windows(2)
                .any(|x| x == ["--sandbox", "danger-full-access"])
        );
        assert_eq!(amp_args(&full, Path::new("/bin/hifi"))[0], "--execute");
        assert!(
            !amp_args(&balanced, Path::new("/bin/hifi"))
                .iter()
                .any(|x| x == "--dangerously-allow-all")
        );
    }

    #[test]
    fn parses_claude_fixture() {
        assert!(
            matches!(&events_from_claude(&json!({"type":"system","subtype":"init","session_id":"s1"}))[0], AgentEvent::Session(id) if id == "s1")
        );
        let events = events_from_claude(
            &json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"t1","name":"browser_click","input":{"label":"Learn more"}}]}}),
        );
        assert!(matches!(&events[0], AgentEvent::Text { text, .. } if text == "hello"));
        assert!(
            matches!(&events[1], AgentEvent::ToolStart { title, .. } if title == "Click \"Learn more\"")
        );
        let done = events_from_claude(
            &json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]}}),
        );
        assert!(matches!(&done[0], AgentEvent::ToolDone { output, .. } if output == "ok"));
        let delta = events_from_claude(
            &json!({"type":"stream_event","event":{"message":{"id":"m1"},"delta":{"type":"text_delta","text":" world"}}}),
        );
        assert!(
            matches!(&delta[0], AgentEvent::Text { id, text } if id == "m1" && text == " world")
        );
        assert!(
            matches!(&events_from_claude(&json!({"type":"result","is_error":true,"result":"bad"}))[0], AgentEvent::Done { error: Some(e) } if e == "bad")
        );
    }

    #[test]
    fn parses_codex_fixture() {
        assert!(
            matches!(&events_from_codex(&json!({"type":"thread.started","thread_id":"t"}))[0], AgentEvent::Session(id) if id == "t")
        );
        assert!(
            matches!(&events_from_codex(&json!({"type":"item.started","item":{"id":"i","type":"command_execution","command":"ls"}}))[0], AgentEvent::ToolStart { title, .. } if title == "ls")
        );
        assert!(
            matches!(&events_from_codex(&json!({"type":"item.completed","item":{"id":"i","type":"command_execution","command":"ls","aggregated_output":"ok","exit_code":0}}))[0], AgentEvent::ToolDone { output, .. } if output == "ok")
        );
        assert!(
            matches!(&events_from_codex(&json!({"type":"item.started","item":{"id":"m","type":"mcp_tool_call","server":"hifi","name":"browser_read"}}))[0], AgentEvent::ToolStart { title, .. } if title == "hifi / browser_read")
        );
        assert!(
            matches!(&events_from_codex(&json!({"type":"item.completed","item":{"id":"f","type":"file_change","changes":[{"path":"src/main.rs"}]}}))[0], AgentEvent::ToolDone { title, .. } if title == "edit src/main.rs")
        );
        assert!(
            matches!(&events_from_codex(&json!({"type":"turn.failed","error":{"message":"nope"}}))[0], AgentEvent::Done { error: Some(e) } if e == "nope")
        );
    }

    #[test]
    fn missing_harness_reports_install_hint() {
        let bogus = Harness {
            id: "bogus",
            label: "Bogus",
            executable: "definitely-not-a-hifi-harness",
            args: &[],
            protocol: Protocol::Acp,
            models: &["default"],
            install_hint: "install bogus",
        };
        assert!(
            resolve(&bogus)
                .expect_err("missing")
                .contains("install bogus")
        );
    }

    #[test]
    fn model_short_strips_provider() {
        assert_eq!(model_short("opencode/big-pickle"), "big-pickle");
    }

    #[test]
    fn unknown_harnesses_fall_back_to_opencode() {
        assert_eq!(harness("nope").id, "opencode");
    }

    #[test]
    fn destructive_detection_is_command_aware() {
        assert!(!is_destructive("git add ."));
        assert!(is_destructive("echo hi && rm -rf x"));
        assert!(is_destructive("sudo ls"));
        assert!(is_destructive("dd if=/dev/zero"));
    }

    #[test]
    #[ignore = "requires a configured OpenCode account and network access"]
    fn opencode_live_turn() {
        let cwd = std::env::temp_dir().join(format!("hifi-opencode-live-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).expect("create temporary OpenCode cwd");
        let cfg = RunConfig {
            harness: "opencode".into(),
            model: "opencode/big-pickle".into(),
            cwd: cwd.display().to_string(),
            guard: Guard::Strict,
            session: String::new(),
            prompt: "Reply with exactly the word PONG and nothing else.".into(),
            tab_id: "test-tab".into(),
            first_turn: false,
        };
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let run = spawn(cfg, tx).expect("OpenCode must be installed");
        let (events_tx, events_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let events = futures::executor::block_on(async move {
                let mut events = Vec::new();
                while let Some(event) = rx.next().await {
                    let done = matches!(event, AgentEvent::Done { .. });
                    events.push(event);
                    if done {
                        break;
                    }
                }
                events
            });
            let _ = events_tx.send(events);
        });
        let result = events_rx.recv_timeout(Duration::from_secs(120));
        run.stop();
        let _ = std::fs::remove_dir_all(&cwd);
        let events = result.expect("OpenCode did not finish within 120 seconds");
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Text { text, .. } if text.contains("PONG")
            )),
            "OpenCode events did not contain PONG: {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::Done { error: None })),
            "OpenCode did not complete successfully: {events:?}"
        );
    }
}
