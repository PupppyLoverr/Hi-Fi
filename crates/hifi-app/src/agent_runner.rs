//! Real agent execution: spawns a CLI harness (OpenCode first-class, any other
//! harness as a plain-text stream) in the chat's working folder and turns its
//! output into transcript events.
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
use hifi_core::Guard;
use serde_json::{Value, json};

/// Harnesses the composer offers: command, label, models (provider/model).
pub struct Harness {
    pub command: &'static str,
    pub label: &'static str,
    pub models: &'static [&'static str],
}

pub const HARNESSES: &[Harness] = &[
    Harness {
        command: "opencode",
        label: "OpenCode",
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
    },
    Harness {
        command: "claude",
        label: "Claude Code",
        models: &["sonnet", "opus", "haiku"],
    },
    Harness {
        command: "codex",
        label: "Codex",
        models: &["gpt-5-codex", "gpt-5"],
    },
    Harness {
        command: "amp",
        label: "Amp",
        models: &["default"],
    },
];

pub fn harness(command: &str) -> &'static Harness {
    HARNESSES
        .iter()
        .find(|h| h.command == command)
        .unwrap_or(&HARNESSES[0])
}

/// "opencode/big-pickle" → "big-pickle"; what the picker shows.
pub fn model_short(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Harness session id — pass back as `session` to continue the thread.
    Session(String),
    ToolStart {
        id: String,
        tool: String,
        title: String,
    },
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
    child: Arc<Mutex<Option<Child>>>,
}

impl Run {
    pub fn stop(&self) {
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut c) = guard.take()
        {
            let _ = c.kill();
            let _ = c.wait();
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
        h.join(".local/bin"),
        h.join(".cargo/bin"),
        h.join(".bun/bin"),
        h.join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.insert(0, dir.to_path_buf());
    }
    dirs
}

/// `PATH` with the extra bin dirs prepended so harnesses and `hifi` resolve
/// even when the app was launched from Finder (no shell profile).
fn search_path() -> String {
    let mut parts: Vec<String> = extra_bin_dirs()
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    if let Ok(p) = std::env::var("PATH") {
        parts.push(p);
    }
    let sep = if cfg!(windows) { ";" } else { ":" };
    parts.join(sep)
}

pub fn find_binary(name: &str) -> Option<PathBuf> {
    let candidates = extra_bin_dirs().into_iter().chain(
        std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default(),
    );
    for dir in candidates {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
            let cmd = dir.join(format!("{name}.cmd"));
            if cmd.is_file() {
                return Some(cmd);
            }
        }
    }
    None
}

/// OpenCode config for a guard level: browser tools via `hifi mcp`, file and
/// shell access per level. `--auto` approves anything not denied here.
pub fn opencode_config(guard: Guard, hifi_bin: &Path, tab_id: &str) -> Value {
    let permission = match guard {
        Guard::Strict => json!({
            "*": "allow",
            "edit": "deny",
            "write": "deny",
            "patch": "deny",
            // The bash tool stays registered (OpenCode's free tier rejects
            // requests without it) but nothing except a bare `pwd` may run;
            // reads go through the read/glob/grep tools.
            "bash": { "*": "deny", "pwd": "allow" },
        }),
        Guard::Balanced => json!({
            "*": "allow",
            "bash": {
                "*": "allow",
                "rm -r*": "deny",
                "rm -f*": "deny",
                "sudo *": "deny",
                "git push*": "deny",
                "git reset --hard*": "deny",
                "git clean*": "deny",
                "mkfs*": "deny",
                "dd *": "deny",
                "chmod -R*": "deny",
                "> /dev/*": "deny",
            }
        }),
        Guard::Full => json!({ "*": "allow" }),
    };
    json!({
        "$schema": "https://opencode.ai/config.json",
        "permission": permission,
        "mcp": {
            "hifi": {
                "type": "local",
                "command": [hifi_bin.display().to_string(), "mcp"],
                "enabled": true,
                "environment": { "HIFI_AGENT_TAB": tab_id }
            }
        }
    })
}

/// What the harness is told about where it runs, once per chat.
fn preamble(cwd: &str, guard: Guard) -> String {
    format!(
        "You are running inside the Hi-Fi browser as its agent. Working folder: {cwd}. \
Permission level: {} ({}). To use the web, call the hifi browser_* tools: browser_open to \
open a page in your own tab (the user's tabs are never touched), then browser_read for the \
text, browser_snapshot for clickable elements, browser_click / browser_type to interact, \
browser_scroll to see more. Keep answers concise.\n\nTask: ",
        guard.label(),
        guard.describe()
    )
}

/// Spawn the harness. Events arrive on `tx` from a reader thread; the returned
/// [`Run`] kills the process on stop.
pub fn spawn(cfg: RunConfig, tx: UnboundedSender<AgentEvent>) -> Result<Run, String> {
    let bin = find_binary(&cfg.harness).ok_or_else(|| match cfg.harness.as_str() {
        "opencode" => "OpenCode isn't installed. Install it with `curl -fsSL https://opencode.ai/install | bash`, then send again.".to_string(),
        h => format!("`{h}` isn't installed or not on PATH."),
    })?;
    let cwd = if cfg.cwd.is_empty() {
        home()
    } else {
        PathBuf::from(&cfg.cwd)
    };
    let prompt = if cfg.first_turn {
        format!(
            "{}{}",
            preamble(&cwd.display().to_string(), cfg.guard),
            cfg.prompt
        )
    } else {
        cfg.prompt.clone()
    };
    let json_events = cfg.harness == "opencode";
    let mut cmd = Command::new(&bin);
    cmd.current_dir(&cwd)
        .env("PATH", search_path())
        .env("HIFI_AGENT_TAB", &cfg.tab_id)
        .env_remove("CLOUDFLARE_API_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match cfg.harness.as_str() {
        "opencode" => {
            let hifi = find_binary("hifi").unwrap_or_else(|| PathBuf::from("hifi"));
            cmd.arg("run").arg("--format").arg("json");
            if !cfg.model.is_empty() {
                cmd.arg("-m").arg(&cfg.model);
            }
            if !cfg.session.is_empty() {
                cmd.arg("--session").arg(&cfg.session);
            }
            cmd.arg("--auto");
            cmd.env(
                "OPENCODE_CONFIG_CONTENT",
                opencode_config(cfg.guard, &hifi, &cfg.tab_id).to_string(),
            );
            cmd.arg(&prompt);
        }
        "claude" => {
            cmd.arg("-p");
            if !cfg.model.is_empty() {
                cmd.arg("--model").arg(&cfg.model);
            }
            if !cfg.session.is_empty() {
                cmd.arg("--resume").arg(&cfg.session);
            }
            cmd.arg(&prompt);
        }
        "codex" => {
            cmd.arg("exec");
            if !cfg.model.is_empty() && cfg.model != "default" {
                cmd.arg("-m").arg(&cfg.model);
            }
            cmd.arg(&prompt);
        }
        "amp" => {
            cmd.arg("-x").arg(&prompt);
        }
        _ => {
            cmd.arg(&prompt);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to start {}: {e}", bin.display()))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(Some(child)));
    let handle = child.clone();

    let err_tail: Arc<Mutex<String>> = Arc::default();
    if let Some(stderr) = stderr {
        let tail = err_tail.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut t) = tail.lock() {
                    t.push_str(&line);
                    t.push('\n');
                    if t.len() > 4000 {
                        let cut = t.len() - 4000;
                        t.drain(..cut);
                    }
                }
            }
        });
    }

    thread::spawn(move || {
        if let Some(stdout) = stdout {
            let mut plain_id = 0u32;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if json_events {
                    if let Ok(v) = serde_json::from_str::<Value>(&line) {
                        for ev in events_from_opencode(&v) {
                            if tx.unbounded_send(ev).is_err() {
                                return;
                            }
                        }
                    }
                } else if !line.trim().is_empty() {
                    plain_id += 1;
                    let ev = AgentEvent::Text {
                        id: format!("line-{plain_id}"),
                        text: line,
                    };
                    if tx.unbounded_send(ev).is_err() {
                        return;
                    }
                }
            }
        }
        let status = handle
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .and_then(|mut c| c.wait().ok());
        let failed = status.is_some_and(|s| !s.success());
        let error = if failed {
            let tail = err_tail
                .lock()
                .map(|t| t.trim().to_string())
                .unwrap_or_default();
            Some(if tail.is_empty() {
                "harness exited with an error".to_string()
            } else {
                tail
            })
        } else {
            None
        };
        let _ = tx.unbounded_send(AgentEvent::Done { error });
    });

    Ok(Run { child })
}

/// Map one `opencode run --format json` line to transcript events.
pub fn events_from_opencode(v: &Value) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    if let Some(sid) = v.get("sessionID").and_then(Value::as_str) {
        out.push(AgentEvent::Session(sid.to_string()));
    }
    if v.get("type").and_then(Value::as_str) == Some("error") {
        let err = v.get("error");
        let msg = err
            .and_then(|e| e.pointer("/data/message"))
            .or_else(|| err.and_then(|e| e.get("message")))
            .or_else(|| err.and_then(|e| e.get("name")))
            .and_then(Value::as_str)
            .unwrap_or("harness reported an error");
        out.push(AgentEvent::Done {
            error: Some(msg.to_string()),
        });
        return out;
    }
    let Some(part) = v.get("part") else {
        return out;
    };
    let s = |k: &str| {
        part.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match v.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" => {
            let text = s("text");
            if !text.is_empty() {
                out.push(AgentEvent::Text { id: s("id"), text });
            }
        }
        "tool_use" => {
            let tool = s("tool");
            let id = if part.get("callID").is_some() {
                s("callID")
            } else {
                s("id")
            };
            let state = part.get("state").cloned().unwrap_or(Value::Null);
            let title = state
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|t| !t.is_empty())
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
                        .to_string(),
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
                            .to_string(),
                    ),
                }),
                _ => out.push(AgentEvent::ToolStart { id, tool, title }),
            }
        }
        _ => {}
    }
    out
}

/// Fallback title when the harness gives none: the most telling input field.
fn tool_title(tool: &str, input: Option<&Value>) -> String {
    let pick = |keys: &[&str]| {
        input.and_then(|i| {
            keys.iter()
                .find_map(|k| i.get(*k).and_then(Value::as_str))
                .map(str::to_string)
        })
    };
    pick(&[
        "command",
        "url",
        "filePath",
        "path",
        "pattern",
        "target",
        "text",
        "description",
    ])
    .unwrap_or_else(|| {
        tool.trim_start_matches("hifi_")
            .replace('_', " ")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tool_and_text_events() {
        let line = json!({
            "type":"tool_use","sessionID":"ses_1",
            "part":{"type":"tool","tool":"bash","callID":"c1",
                "state":{"status":"completed","input":{"command":"echo hi"},"output":"hi\n","title":"echo hi"}}
        });
        let evs = events_from_opencode(&line);
        assert!(matches!(&evs[0], AgentEvent::Session(s) if s == "ses_1"));
        assert!(
            matches!(&evs[1], AgentEvent::ToolDone { id, tool, title, output, error: None } if id == "c1" && tool == "bash" && title == "echo hi" && output == "hi\n")
        );
        let text = json!({"type":"text","sessionID":"ses_1","part":{"id":"p1","type":"text","text":"`hi`"}});
        let evs = events_from_opencode(&text);
        assert!(matches!(&evs[1], AgentEvent::Text { id, text } if id == "p1" && text == "`hi`"));
    }

    #[test]
    fn provider_error_surfaces_message() {
        let line = json!({"type":"error","sessionID":"s1",
            "error":{"name":"APIError","data":{"message":"Rate limit exceeded","statusCode":429}}});
        let evs = events_from_opencode(&line);
        assert!(
            matches!(&evs[1], AgentEvent::Done { error: Some(e) } if e == "Rate limit exceeded")
        );
    }

    #[test]
    fn rejected_tool_is_an_error() {
        let line = json!({"type":"tool_use","part":{"tool":"edit","callID":"c2",
            "state":{"status":"error","input":{"filePath":"a.rs"},"error":"The user rejected permission to use this specific tool call."}}});
        let evs = events_from_opencode(&line);
        assert!(
            matches!(&evs[0], AgentEvent::ToolDone { title, error: Some(e), .. } if title == "a.rs" && e.contains("rejected"))
        );
    }

    #[test]
    fn guard_levels_shape_permissions() {
        let bin = Path::new("/usr/local/bin/hifi");
        let strict = opencode_config(Guard::Strict, bin, "tab1");
        assert_eq!(strict["permission"]["bash"]["*"], "deny");
        assert_eq!(strict["permission"]["write"], "deny");
        assert_eq!(strict["permission"]["edit"], "deny");
        assert_eq!(strict["mcp"]["hifi"]["command"][1], "mcp");
        assert_eq!(
            strict["mcp"]["hifi"]["environment"]["HIFI_AGENT_TAB"],
            "tab1"
        );
        let balanced = opencode_config(Guard::Balanced, bin, "tab1");
        assert_eq!(balanced["permission"]["bash"]["sudo *"], "deny");
        assert_eq!(balanced["permission"]["bash"]["*"], "allow");
        let full = opencode_config(Guard::Full, bin, "tab1");
        assert_eq!(full["permission"]["*"], "allow");
    }

    #[test]
    fn model_short_strips_provider() {
        assert_eq!(model_short("opencode/big-pickle"), "big-pickle");
        assert_eq!(model_short("sonnet"), "sonnet");
    }
}
