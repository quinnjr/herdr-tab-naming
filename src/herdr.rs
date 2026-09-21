//! Thin client for the Herdr socket API (newline-delimited JSON-RPC).
//!
//! Request/response calls open a short-lived connection: the server closes the
//! stream after a non-subscription response, so connections are never reused.
//! Event subscriptions use a separate long-lived connection.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

const CALL_TIMEOUT: Duration = Duration::from_secs(5);

pub fn socket_path() -> Result<PathBuf, String> {
    std::env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .ok_or_else(|| "HERDR_SOCKET_PATH is not set".to_string())
}

pub fn connect() -> Result<UnixStream, String> {
    let path = socket_path()?;
    UnixStream::connect(&path).map_err(|e| format!("connect {}: {e}", path.display()))
}

/// One request/response round-trip on a fresh connection.
pub fn call(method: &str, params: Value) -> Result<Value, String> {
    let stream = connect()?;
    stream
        .set_read_timeout(Some(CALL_TIMEOUT))
        .map_err(|e| format!("{method}: set timeout: {e}"))?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);

    let req = json!({ "id": "req", "method": method, "params": params });
    writer
        .write_all(req.to_string().as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|e| format!("{method}: write: {e}"))?;

    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .map_err(|e| format!("{method}: read: {e}"))?;
    if n == 0 {
        return Err(format!("{method}: connection closed"));
    }
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("{method}: malformed response: {e}"))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("{method}: {msg}"));
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub focused: bool,
}

impl Pane {
    /// Directory the pane is actually working in, preferring the resolved
    /// foreground process cwd over the (stickier) pane cwd.
    pub fn workdir(&self) -> Option<&str> {
        self.foreground_cwd
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.cwd.as_deref().filter(|s| !s.is_empty()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: u32,
}

pub fn pane_list() -> Result<Vec<Pane>, String> {
    let result = call("pane.list", json!({}))?;
    let panes = result.get("panes").cloned().unwrap_or(Value::Array(vec![]));
    serde_json::from_value(panes).map_err(|e| format!("pane.list: bad payload: {e}"))
}

pub fn tab_list() -> Result<Vec<Tab>, String> {
    let result = call("tab.list", json!({}))?;
    let tabs = result.get("tabs").cloned().unwrap_or(Value::Array(vec![]));
    serde_json::from_value(tabs).map_err(|e| format!("tab.list: bad payload: {e}"))
}

pub fn tab_rename(tab_id: &str, label: &str) -> Result<(), String> {
    call("tab.rename", json!({ "tab_id": tab_id, "label": label })).map(|_| ())
}

/// Events the watcher listens to so structural changes land immediately; the
/// poll timer is what catches a plain `cd`, which Herdr does not emit.
///
/// `pane.agent_status_changed`, `pane.output_matched`, and
/// `pane.scroll_changed` are intentionally absent: Herdr requires a `pane_id`
/// for those, which a single global watcher cannot supply.
pub const SUBSCRIPTIONS: &[&str] = &[
    "tab.created",
    "tab.closed",
    "tab.focused",
    "tab.moved",
    "tab.renamed",
    "pane.created",
    "pane.closed",
    "pane.exited",
    "pane.focused",
    "pane.moved",
    "pane.updated",
    "workspace.created",
    "workspace.closed",
    "workspace.focused",
    "workspace.moved",
    "workspace.renamed",
    "worktree.created",
    "worktree.opened",
    "worktree.removed",
];

/// Open the long-lived event subscription and consume the ack. The returned
/// stream has not been over-read, so the caller can buffer it freely.
pub fn subscribe() -> Result<UnixStream, String> {
    let stream = connect()?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let subs: Vec<Value> = SUBSCRIPTIONS.iter().map(|t| json!({ "type": t })).collect();
    let req = json!({
        "id": "sub",
        "method": "events.subscribe",
        "params": { "subscriptions": subs },
    });
    writer
        .write_all(req.to_string().as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|e| format!("subscribe: write: {e}"))?;

    let ack = read_line_raw(&stream)?;
    let value: Value =
        serde_json::from_str(ack.trim()).map_err(|e| format!("subscribe: malformed ack: {e}"))?;
    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("subscribe: {msg}"));
    }
    Ok(stream)
}

/// Read a single newline-terminated line one byte at a time so nothing beyond
/// the terminator is consumed.
fn read_line_raw(stream: &UnixStream) -> Result<String, String> {
    let mut reader = stream.try_clone().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                out.push(byte[0]);
                if out.len() > 1 << 20 {
                    return Err("subscribe: oversized ack".to_string());
                }
            }
            Err(e) => return Err(format!("subscribe: read ack: {e}")),
        }
    }
    String::from_utf8(out).map_err(|e| format!("subscribe: non-utf8 ack: {e}"))
}
