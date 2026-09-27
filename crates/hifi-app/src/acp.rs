//! Minimal newline-delimited ACP client used by the production harnesses.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use futures::channel::mpsc::UnboundedSender;
use hifi_core::Guard;
use serde_json::{Value, json};

use crate::agent_runner::{AgentEvent, Run, RunConfig, value_text};

#[derive(Default)]
pub(crate) struct UpdateState {
    next_message: u32,
    current_id: Option<String>,
}

fn mcp_server(tab: &str, hifi: &std::path::Path) -> Value {
    json!([{
        "name": "hifi",
        "command": hifi,
        "args": ["mcp"],
        "env": [{"name": "HIFI_AGENT_TAB", "value": tab}]
    }])
}

fn rpc(id: u64, method: &str, params: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string()
}

fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc":"2.0","method":method,"params":params}).to_string()
}

fn response(id: &Value, result: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"result":result}).to_string()
}

fn error_response(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
}

/// Select a permission option according to Hi-Fi's guard policy.
pub fn permission_response(params: &Value, guard: Guard) -> Value {
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tool = params.get("toolCall").unwrap_or(&Value::Null);
    let kind = tool.get("kind").and_then(Value::as_str).unwrap_or_default();
    let title = tool
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let raw = tool
        .get("rawInput")
        .or_else(|| params.get("rawInput"))
        .map(value_text)
        .unwrap_or_default();
    let destructive = crate::agent_runner::is_destructive(&format!("{title} {raw}"));
    let wanted = match guard {
        Guard::Strict if matches!(kind, "read" | "fetch" | "search" | "think") => "allow_once",
        Guard::Strict => "reject_once",
        Guard::Balanced if destructive => "reject_once",
        Guard::Balanced => "allow_once",
        Guard::Full => "allow_always",
    };
    options
        .iter()
        .find(|option| option.get("kind").and_then(Value::as_str) == Some(wanted))
        .or_else(|| {
            (guard == Guard::Full)
                .then(|| options.iter().find(|option| option.get("kind").and_then(Value::as_str) == Some("allow_once")))
                .flatten()
        })
        .map(|option| {
            json!({"outcome":{"outcome":"selected","optionId":option.get("optionId").or_else(|| option.get("id")).cloned().unwrap_or(Value::Null)}})
        })
        .unwrap_or_else(|| json!({"outcome":{"outcome":"cancelled"}}))
}

pub(crate) fn events_from_update(update: &Value, state: &mut UpdateState) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    let update = update
        .pointer("/params/update")
        .or_else(|| update.get("update"))
        .unwrap_or(update);
    match update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "agent_message_chunk" => {
            let text = update
                .get("content")
                .map(value_text)
                .or_else(|| update.pointer("/messageChunk/text").map(value_text))
                .unwrap_or_default();
            if !text.is_empty() {
                let id = state
                    .current_id
                    .get_or_insert_with(|| {
                        state.next_message += 1;
                        format!("msg-{}", state.next_message)
                    })
                    .clone();
                events.push(AgentEvent::Text { id, text });
            }
        }
        "tool_call" => {
            state.next_message += 1;
            state.current_id = None;
            let id = update
                .get("toolCallId")
                .or_else(|| update.get("callId"))
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let tool = update
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let title = update
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(&tool)
                .to_string();
            events.push(AgentEvent::ToolStart { id, tool, title });
        }
        "tool_call_update" => {
            let status = update
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(status, "completed" | "failed") {
                let id = update
                    .get("toolCallId")
                    .or_else(|| update.get("callId"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let tool = update
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let title = update
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(&tool)
                    .to_string();
                let output = update
                    .get("content")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| item.get("content").map(value_text).unwrap_or_default())
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default();
                events.push(AgentEvent::ToolDone {
                    id,
                    tool,
                    title,
                    output,
                    error: (status == "failed").then(|| {
                        update
                            .get("error")
                            .map(value_text)
                            .unwrap_or_else(|| "tool failed".into())
                    }),
                });
                state.current_id = None;
            }
        }
        _ => {}
    }
    events
}

/// Spawn an ACP harness and drive its initialize/session/prompt lifecycle.
pub fn spawn(
    cfg: RunConfig,
    bin: PathBuf,
    args: Vec<String>,
    tx: UnboundedSender<AgentEvent>,
) -> Result<Run, String> {
    let cwd = if cfg.cwd.is_empty() {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    } else {
        PathBuf::from(&cfg.cwd)
    };
    let hifi = crate::agent_runner::find_binary("hifi").unwrap_or_else(|| PathBuf::from("hifi"));
    let mut command = Command::new(&bin);
    command
        .current_dir(cwd)
        .args(args)
        .env("PATH", crate::agent_runner::search_path())
        .env("HIFI_AGENT_TAB", &cfg.tab_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start {}: {error}", bin.display()))?;
    let stdin = child.stdin.take().ok_or("ACP stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("ACP stdout was not piped")?;
    let stderr = child.stderr.take();
    let child_arc = Arc::new(Mutex::new(Some(child)));
    let wait_child = child_arc.clone();
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
    let (write_tx, write_rx) = mpsc::channel::<String>();
    thread::spawn(move || writer(stdin, write_rx));
    let session = Arc::new(Mutex::new(None::<String>));
    let cancel_tx = write_tx.clone();
    let cancel_session = session.clone();
    let cancel = Arc::new(move || {
        if let Ok(session) = cancel_session.lock()
            && let Some(id) = session.as_deref()
        {
            let _ = cancel_tx.send(notification("session/cancel", json!({"sessionId": id})));
        }
    });
    let initial = rpc(
        1,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},
            "clientInfo": {"name":"Hi-Fi","version":env!("CARGO_PKG_VERSION")}
        }),
    );
    write_tx.send(initial).map_err(|_| "ACP writer stopped")?;
    let tab = cfg.tab_id.clone();
    let cwd = if cfg.cwd.is_empty() {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    } else {
        PathBuf::from(&cfg.cwd)
    };
    let prompt = cfg.prompt.clone();
    let session_name = cfg.session.clone();
    let guard = cfg.guard;
    thread::spawn(move || {
        let mut next_id = 2u64;
        let mut phase = 0u8;
        let mut loading = false;
        let mut prompt_id = None::<u64>;
        let mut auth_required = false;
        let mut done = false;
        let mut updates = UpdateState::default();
        let reader = BufReader::new(stdout);
        let send_request = |id: u64, method: &str, params: Value| {
            let _ = write_tx.send(rpc(id, method, params));
        };
        for line in reader.lines().map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("method").and_then(Value::as_str).is_some() {
                let method = value
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let id = value.get("id");
                if method == "session/update" {
                    if !loading {
                        for event in events_from_update(&value, &mut updates) {
                            if tx.unbounded_send(event).is_err() {
                                return;
                            }
                        }
                    }
                    continue;
                }
                if let Some(id) = id {
                    let result = if method == "session/request_permission" {
                        permission_response(value.get("params").unwrap_or(&Value::Null), guard)
                    } else {
                        let message =
                            if method.starts_with("fs/") || method.starts_with("terminal/") {
                                "Hi-Fi does not expose filesystem or terminal requests"
                            } else {
                                "Hi-Fi does not support this ACP request"
                            };
                        let _ = write_tx.send(error_response(id, -32601, message));
                        continue;
                    };
                    let _ = write_tx.send(response(id, result));
                }
                continue;
            }
            let Some(id) = value.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let result = value.get("result").cloned().unwrap_or(Value::Null);
            let error = value.get("error");
            if phase == 0 && id == 1 {
                if let Some(error) = error {
                    let message = value_text(error);
                    let _ = tx.unbounded_send(AgentEvent::Done {
                        error: Some(message),
                    });
                    done = true;
                    break;
                }
                auth_required = result
                    .get("authMethods")
                    .and_then(Value::as_array)
                    .is_some_and(|methods| !methods.is_empty());
                let load_session = result
                    .pointer("/agentCapabilities/loadSession")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if !session_name.is_empty() && load_session {
                    loading = true;
                    send_request(
                        next_id,
                        "session/load",
                        json!({"sessionId":session_name,"cwd":cwd,"mcpServers":mcp_server(&tab, &hifi)}),
                    );
                } else {
                    send_request(
                        next_id,
                        "session/new",
                        json!({"cwd":cwd,"mcpServers":mcp_server(&tab, &hifi)}),
                    );
                }
                phase = 1;
                next_id += 1;
                continue;
            }
            if phase == 1 && id == next_id - 1 {
                if error.is_some() {
                    let message = error.map(value_text).unwrap_or_default();
                    let message = if auth_required
                        || message.to_lowercase().contains("auth")
                        || message.to_lowercase().contains("login")
                    {
                        format!("Run `{}` login first", bin.display())
                    } else {
                        message
                    };
                    let _ = tx.unbounded_send(AgentEvent::Done {
                        error: Some(message),
                    });
                    done = true;
                    break;
                }
                loading = false;
                let sid = result
                    .get("sessionId")
                    .or_else(|| result.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or(&session_name)
                    .to_string();
                if let Ok(mut current) = session.lock() {
                    *current = Some(sid.clone());
                }
                let _ = tx.unbounded_send(AgentEvent::Session(sid.clone()));
                let id = next_id;
                prompt_id = Some(id);
                send_request(
                    id,
                    "session/prompt",
                    json!({"sessionId":sid,"prompt":[{"type":"text","text":prompt}]}),
                );
                next_id += 1;
                phase = 2;
                continue;
            }
            if phase == 2 && Some(id) == prompt_id {
                if let Some(error) = error {
                    let _ = tx.unbounded_send(AgentEvent::Done {
                        error: Some(value_text(error)),
                    });
                    done = true;
                    break;
                }
                let refusal = result.get("stopReason").and_then(Value::as_str) == Some("refusal");
                let _ = tx.unbounded_send(AgentEvent::Done {
                    error: refusal.then(|| "The harness refused the prompt".into()),
                });
                done = true;
                break;
            }
        }
        if !done {
            let tail = stderr_tail
                .lock()
                .map(|tail| tail.trim().to_string())
                .unwrap_or_default();
            let _ = tx.unbounded_send(AgentEvent::Done {
                error: Some(if tail.is_empty() {
                    "ACP harness exited before completing the prompt".into()
                } else {
                    tail
                }),
            });
        }
        if let Ok(mut child) = wait_child.lock()
            && let Some(mut child) = child.take()
        {
            let _ = child.wait();
        }
    });
    Ok(Run {
        child: child_arc,
        stop_hook: Some(cancel),
    })
}

fn writer(mut stdin: ChildStdin, rx: mpsc::Receiver<String>) {
    for line in rx {
        if writeln!(stdin, "{line}")
            .and_then(|_| stdin.flush())
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_updates_decode_text_and_tools() {
        let mut state = UpdateState::default();
        let text = events_from_update(
            &json!({"params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}}}),
            &mut state,
        );
        assert!(matches!(&text[0], AgentEvent::Text { id, text } if id == "msg-1" && text == "hi"));
        let start = events_from_update(
            &json!({"params":{"update":{"sessionUpdate":"tool_call","toolCallId":"t","kind":"read","title":"Read file"}}}),
            &mut state,
        );
        assert!(
            matches!(&start[0], AgentEvent::ToolStart { id, title, .. } if id == "t" && title == "Read file")
        );
        let done = events_from_update(
            &json!({"params":{"update":{"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed","content":[{"content":{"type":"text","text":"ok"}}]}}}),
            &mut state,
        );
        assert!(matches!(&done[0], AgentEvent::ToolDone { output, .. } if output == "ok"));
    }

    #[test]
    fn permission_choices_follow_guard() {
        let params = json!({"toolCall":{"kind":"read","title":"read"},"options":[
            {"optionId":"allow","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"},{"optionId":"always","kind":"allow_always"}
        ]});
        assert_eq!(
            permission_response(&params, Guard::Strict)["outcome"]["optionId"],
            "allow"
        );
        assert_eq!(
            permission_response(&params, Guard::Balanced)["outcome"]["optionId"],
            "allow"
        );
        assert_eq!(
            permission_response(&params, Guard::Full)["outcome"]["optionId"],
            "always"
        );
        let destructive = json!({"toolCall":{"kind":"execute","title":"sudo rm -rf /"},"options":[
            {"optionId":"allow","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"}
        ]});
        assert_eq!(
            permission_response(&destructive, Guard::Balanced)["outcome"]["optionId"],
            "reject"
        );
        let nested_raw = json!({"toolCall":{"kind":"execute","rawInput":"git push"},"options":[
            {"optionId":"allow","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"}
        ]});
        assert_eq!(
            permission_response(&nested_raw, Guard::Balanced)["outcome"]["optionId"],
            "reject"
        );
        for (guard, expected) in [
            (Guard::Strict, "allow"),
            (Guard::Balanced, "allow"),
            (Guard::Full, "always"),
        ] {
            assert_eq!(
                permission_response(&params, guard)["outcome"]["optionId"],
                expected
            );
        }
        let non_read = json!({"toolCall":{"kind":"execute","title":"run command"},"options":[
            {"optionId":"allow","kind":"allow_once"},{"optionId":"reject","kind":"reject_once"}
        ]});
        assert_eq!(
            permission_response(&non_read, Guard::Strict)["outcome"]["optionId"],
            "reject"
        );
        let no_match = json!({"toolCall":{"kind":"execute"},"options":[{"optionId":"reject","kind":"reject_once"}]});
        assert_eq!(
            permission_response(&no_match, Guard::Full)["outcome"]["outcome"],
            "cancelled"
        );
    }

    #[test]
    fn mcp_environment_uses_name_value_pairs() {
        let mcp = mcp_server("tab-1", PathBuf::from("/bin/hifi").as_path());
        assert_eq!(mcp[0]["env"][0]["name"], "HIFI_AGENT_TAB");
        assert_eq!(mcp[0]["env"][0]["value"], "tab-1");
    }
}
