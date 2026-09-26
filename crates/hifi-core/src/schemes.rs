//! `hifi://` internal scheme routing.

use crate::models::TabKind;
use serde::{Deserialize, Serialize};

pub const SCHEME: &str = "hifi";

/// Internal destinations plus the Radius-compatible aliases (`chat`, `diffs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutedUrl {
    External(String),
    Internal(TabKind),
    /// hifi://preview/<path> — render a local file (markdown/html/image).
    Preview(String),
}

pub fn route(input: &str) -> RoutedUrl {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix(&format!("{SCHEME}://")) {
        let (head, tail) = rest
            .split_once('/')
            .map(|(h, t)| (h, t.to_string()))
            .unwrap_or((rest, String::new()));
        return match head {
            "newtab" | "new-tab" | "home" => RoutedUrl::Internal(TabKind::NewTab),
            "terminal" => RoutedUrl::Internal(TabKind::Terminal),
            "agent" | "chat" => RoutedUrl::Internal(TabKind::Agent),
            "diff" | "diffs" => RoutedUrl::Internal(TabKind::Diff),
            "settings" | "preferences" => RoutedUrl::Internal(TabKind::Settings),
            "notes" | "note" | "docs" => RoutedUrl::Internal(TabKind::Notes),
            "preview" => RoutedUrl::Preview(tail),
            _ => RoutedUrl::Internal(TabKind::NewTab),
        };
    }
    RoutedUrl::External(normalize_url(trimmed))
}

/// Bare input → http(s) URL or search query via the engine template.
pub fn normalize_url(input: &str) -> String {
    if input.starts_with("http://") || input.starts_with("https://") || input.starts_with("file://")
    {
        return input.to_string();
    }
    if url::Url::parse(&format!("https://{input}"))
        .map(|u| u.host_str().map(|h| h.contains('.')).unwrap_or(false))
        .unwrap_or(false)
    {
        return format!("https://{input}");
    }
    // Search — caller substitutes {q} into the configured engine.
    format!("search:{input}")
}

pub fn resolve_search(template: &str, query: &str) -> String {
    template.replace("{q}", &urlencoding(query))
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
