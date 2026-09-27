use anyhow::{Result, bail};
use hifi_core::ipc::{IpcResponse, client, methods};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::thread;
use std::time::Duration;

const PROTOCOL_VERSION: &str = "2025-06-18";
const IPC_TIMEOUT_SECS: u64 = 30;

type IpcCall = dyn Fn(&str, Value) -> Result<IpcResponse> + Send + Sync;

pub struct Ctx {
    pub agent_tab: Option<String>,
    pub wait_after_navigation: bool,
    pub call: Box<IpcCall>,
}

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

pub fn run(socket: &Path) -> Result<()> {
    let agent_tab = std::env::var("HIFI_AGENT_TAB")
        .ok()
        .filter(|value| !value.is_empty());
    let socket = socket.to_path_buf();
    let ctx = Ctx {
        agent_tab,
        wait_after_navigation: true,
        call: Box::new(move |method, params| {
            client::call(&socket, method, params, IPC_TIMEOUT_SECS)
        }),
    };
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if let Some(response) = handle_message(&line, &ctx) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

pub fn handle_message(message: &str, ctx: &Ctx) -> Option<String> {
    let request: Request = match serde_json::from_str(message) {
        Ok(request) => request,
        Err(error) => return Some(response_error(Value::Null, -32700, error.to_string())),
    };
    let id = request.id?;
    let response = match request.method.as_str() {
        "initialize" => response_result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "hifi",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        ),
        "ping" => response_result(id, json!({})),
        "tools/list" => response_result(id, json!({"tools": tools()})),
        "tools/call" => handle_tool_call(id, request.params, ctx),
        _ => response_error(id, -32601, "Method not found"),
    };
    Some(response)
}

fn handle_tool_call(id: Value, params: Value, ctx: &Ctx) -> String {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) => name,
        None => return tool_response(id, "tools/call requires a tool name", true),
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name {
        "browser_open" => browser_open(&arguments, ctx),
        "browser_tabs" => browser_tabs(ctx),
        "browser_navigate" => browser_navigate(&arguments, ctx),
        "browser_read" => browser_read(&arguments, ctx),
        "browser_snapshot" => browser_snapshot(&arguments, ctx),
        "browser_click" => browser_click(&arguments, ctx),
        "browser_type" => browser_type(&arguments, ctx),
        "browser_scroll" => browser_scroll(&arguments, ctx),
        "browser_screenshot" => browser_screenshot(&arguments, ctx),
        "browser_back" => browser_back(&arguments, ctx),
        "browser_close" => browser_close(&arguments, ctx),
        _ => Err(anyhow::anyhow!("Unknown tool: {name}")),
    };
    match result {
        Ok(text) => tool_response(id, text, false),
        Err(error) => tool_response(id, error.to_string(), true),
    }
}

fn browser_open(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let url = required_string(arguments, "url")?;
    let mut params = json!({"url": url});
    if let Some(agent_tab) = &ctx.agent_tab {
        params["agentOf"] = json!(agent_tab);
    }
    let result = call_ipc(ctx, methods::TAB_OPEN, params)?;
    let id = result
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("tab.open returned no tab id"))?;
    wait_after_navigation(ctx);
    Ok(format!(
        "Opened tab {id} at {url}. Wait briefly, then use browser_read or browser_snapshot."
    ))
}

fn browser_tabs(ctx: &Ctx) -> Result<String> {
    let result = call_ipc(ctx, methods::TAB_LIST, json!({}))?;
    let tabs = result
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("tab.list returned a non-array result"))?;
    let lines = tabs
        .iter()
        .filter(|tab| {
            ctx.agent_tab
                .as_ref()
                .is_none_or(|owner| tab.get("agentOf").and_then(Value::as_str) == Some(owner))
        })
        .map(|tab| {
            let id = tab.get("id").and_then(Value::as_str).unwrap_or("?");
            let title = tab.get("title").and_then(Value::as_str).unwrap_or("");
            let url = tab.get("url").and_then(Value::as_str).unwrap_or("");
            let loading = tab.get("loading").and_then(Value::as_bool).unwrap_or(false);
            if loading {
                format!("{id}  {title}  {url}  (loading)")
            } else {
                format!("{id}  {title}  {url}")
            }
        })
        .collect::<Vec<_>>();
    Ok(lines.join("\n"))
}

fn browser_navigate(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    let url = required_string(arguments, "url")?;
    call_ipc(ctx, methods::TAB_NAVIGATE, json!({"id": tab, "url": url}))?;
    wait_after_navigation(ctx);
    Ok(format!("Navigated tab {tab} to {url}."))
}

fn browser_read(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    result_text(call_ipc(ctx, methods::TAB_READ, json!({"id": tab}))?)
}

fn browser_snapshot(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    result_text(call_ipc(ctx, methods::TAB_SNAPSHOT, json!({"id": tab}))?)
}

fn browser_click(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    let target = required_string(arguments, "target")?;
    result_text(call_ipc(
        ctx,
        methods::TAB_CLICK,
        json!({"id": tab, "target": target}),
    )?)
}

fn browser_type(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    let target = required_string(arguments, "target")?;
    let text = required_string(arguments, "text")?;
    let submit = arguments
        .get("submit")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    result_text(call_ipc(
        ctx,
        methods::TAB_TYPE,
        json!({"id": tab, "target": target, "text": text, "submit": submit}),
    )?)
}

fn browser_scroll(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    let dy = arguments.get("dy").and_then(Value::as_f64).unwrap_or(600.0);
    result_text(call_ipc(
        ctx,
        methods::TAB_SCROLL,
        json!({"id": tab, "dy": dy}),
    )?)
}

fn browser_screenshot(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    result_text(call_ipc(ctx, methods::TAB_SCREENSHOT, json!({"id": tab}))?)
}

fn browser_back(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    result_text(call_ipc(ctx, methods::TAB_BACK, json!({"id": tab}))?)
}

fn browser_close(arguments: &Value, ctx: &Ctx) -> Result<String> {
    let tab = required_string(arguments, "tab")?;
    result_text(call_ipc(ctx, methods::TAB_CLOSE, json!({"id": tab}))?)
}

fn required_string<'a>(arguments: &'a Value, name: &str) -> Result<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string argument: {name}"))
}

fn call_ipc(ctx: &Ctx, method: &str, params: Value) -> Result<Value> {
    let response = (ctx.call)(method, params)?;
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "unknown Hi-Fi IPC error".to_string())
        );
    }
    Ok(response.result.unwrap_or(Value::Null))
}

/// Page scripts hand back JSON as a string (sometimes twice-encoded); unwrap
/// it and render page reads / snapshots as plain lines the model can scan.
fn result_text(mut result: Value) -> Result<String> {
    for _ in 0..2 {
        if let Value::String(text) = &result
            && let Ok(inner) = serde_json::from_str::<Value>(text)
            && !inner.is_number()
        {
            result = inner;
            continue;
        }
        break;
    }
    match result {
        Value::String(text) => Ok(text),
        Value::Null => Ok(String::new()),
        Value::Object(map) if map.contains_key("text") || map.contains_key("elements") => {
            let s = |k: &str| map.get(k).and_then(Value::as_str).unwrap_or("");
            let mut out = format!("{}\n{}\n", s("title"), s("url"));
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                out.push('\n');
                out.push_str(text);
            }
            if let Some(elements) = map.get("elements").and_then(Value::as_array) {
                out.push_str(&format!("\n{} interactive elements:\n", elements.len()));
                for el in elements {
                    let g = |k: &str| el.get(k).and_then(Value::as_str).unwrap_or("");
                    out.push_str(&format!(
                        "[{}] {} \"{}\"  target={}\n",
                        el.get("i").and_then(Value::as_u64).unwrap_or(0),
                        g("role"),
                        g("label"),
                        g("sel")
                    ));
                }
            }
            Ok(out)
        }
        value => Ok(serde_json::to_string(&value)?),
    }
}

fn wait_after_navigation(ctx: &Ctx) {
    if ctx.wait_after_navigation {
        thread::sleep(Duration::from_millis(1500));
    }
}

fn response_result(id: Value, result: Value) -> String {
    serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
        .expect("JSON-RPC response is serializable")
}

fn response_error(id: Value, code: i64, message: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message.into()},
    }))
    .expect("JSON-RPC error response is serializable")
}

fn tool_response(id: Value, text: impl Into<String>, is_error: bool) -> String {
    response_result(
        id,
        json!({
            "content": [{"type": "text", "text": text.into()}],
            "isError": is_error,
        }),
    )
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "browser_open",
            "Open a URL in a browser tab owned by this agent; it does not disturb the user's tabs.",
            schema(&[("url", "string")], &["url"]),
        ),
        tool(
            "browser_tabs",
            "List browser tabs owned by this agent, or all tabs when no owner is configured.",
            schema(&[], &[]),
        ),
        tool(
            "browser_navigate",
            "Navigate an agent browser tab to a URL.",
            schema(&[("tab", "string"), ("url", "string")], &["tab", "url"]),
        ),
        tool(
            "browser_read",
            "Read the visible text, title, and URL of a browser tab.",
            schema(&[("tab", "string")], &["tab"]),
        ),
        tool(
            "browser_snapshot",
            "Get interactive elements from a tab; use each element's sel value as the target for click/type.",
            schema(&[("tab", "string")], &["tab"]),
        ),
        tool(
            "browser_click",
            "Click a CSS selector from a browser snapshot.",
            schema(
                &[("tab", "string"), ("target", "string")],
                &["tab", "target"],
            ),
        ),
        tool(
            "browser_type",
            "Type text into a CSS selector from a browser snapshot, optionally submitting it.",
            schema(
                &[
                    ("tab", "string"),
                    ("target", "string"),
                    ("text", "string"),
                    ("submit", "boolean"),
                ],
                &["tab", "target", "text"],
            ),
        ),
        tool(
            "browser_scroll",
            "Scroll a browser tab vertically; dy defaults to 600.",
            schema_with_default(
                &[("tab", "string"), ("dy", "number")],
                &["tab"],
                json!({"dy": 600}),
            ),
        ),
        tool(
            "browser_screenshot",
            "Capture a PNG screenshot of a browser tab and return its path.",
            schema(&[("tab", "string")], &["tab"]),
        ),
        tool(
            "browser_back",
            "Navigate a browser tab back.",
            schema(&[("tab", "string")], &["tab"]),
        ),
        tool(
            "browser_close",
            "Close an agent browser tab.",
            schema(&[("tab", "string")], &["tab"]),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn schema(properties: &[(&str, &str)], required: &[&str]) -> Value {
    let mut property_map = serde_json::Map::new();
    for (name, kind) in properties {
        property_map.insert((*name).to_string(), json!({"type": kind}));
    }
    json!({
        "type": "object",
        "properties": property_map,
        "required": required,
        "additionalProperties": false,
    })
}

fn schema_with_default(properties: &[(&str, &str)], required: &[&str], defaults: Value) -> Value {
    let mut input_schema = schema(properties, required);
    if let Some(properties) = input_schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        && let Some(dy) = properties.get_mut("dy")
    {
        dy["default"] = defaults["dy"].clone();
    }
    input_schema
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(response: IpcResponse) -> Ctx {
        Ctx {
            agent_tab: None,
            wait_after_navigation: false,
            call: Box::new(move |_, _| Ok(response.clone())),
        }
    }

    #[test]
    fn initialize_returns_protocol_version() {
        let ctx = test_ctx(IpcResponse::ok("test", Value::Null));
        let response: Value = serde_json::from_str(
            &handle_message(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"old"}}"#,
                &ctx,
            )
            .expect("response"),
        )
        .expect("valid JSON");
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn tools_list_has_eleven_tools() {
        let ctx = test_ctx(IpcResponse::ok("test", Value::Null));
        let response: Value = serde_json::from_str(
            &handle_message(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#,
                &ctx,
            )
            .expect("response"),
        )
        .expect("valid JSON");
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 11);
    }

    #[test]
    fn browser_open_returns_opened_tab_text() {
        let ctx = test_ctx(IpcResponse::ok("test", json!({"id": "t1"})));
        let response: Value = serde_json::from_str(
            &handle_message(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"browser_open","arguments":{"url":"https://example.com"}}}"#,
                &ctx,
            )
            .expect("response"),
        )
        .expect("valid JSON");
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("t1")
        );
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let ctx = test_ctx(IpcResponse::ok("test", Value::Null));
        let response: Value = serde_json::from_str(
            &handle_message(
                r#"{"jsonrpc":"2.0","id":1,"method":"not-a-method","params":{}}"#,
                &ctx,
            )
            .expect("response"),
        )
        .expect("valid JSON");
        assert_eq!(response["error"]["code"], -32601);
    }
}
