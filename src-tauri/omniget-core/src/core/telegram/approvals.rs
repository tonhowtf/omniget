//! Human approval for the `tg_*` tools that write to the person's account
//! (send, forward, delete). An agent never writes by itself: the tool asks
//! here, the OmniGet window shows `telegram-approval-request`
//! `{id, tool, summary, details, client, created, expires_at}`, and the
//! person allows or refuses. The tool waits at most [`APPROVAL_TTL_SECS`].
//! When a request ends (answered or expired) the app gets
//! `telegram-approval-resolved {id, status}` so the dialog can close.
//!
//! Requests live in memory only: a request is worth nothing once the tool
//! call that waits on it is gone.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::oneshot;

/// A request waits this long for the person before it expires.
pub const APPROVAL_TTL_SECS: i64 = 300;

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: String,
    /// The tool that asks (`tg_send_message`, ...).
    pub tool: String,
    /// One line for the dialog title.
    pub summary: String,
    /// What exactly will happen: chat, text, files, message ids.
    pub details: Value,
    /// Who asks: `agent`, an external MCP client name, `cli`.
    pub client: String,
    pub created: i64,
    pub expires_at: i64,
}

struct Pending {
    req: Request,
    tx: oneshot::Sender<bool>,
}

fn table() -> &'static Mutex<HashMap<String, Pending>> {
    static T: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn emit(event: &str, payload: Value) {
    if let Some(h) = super::host::get() {
        h.emit(event, payload);
    }
}

/// Ask the person and wait for the answer. `Ok(())` only when the person
/// allowed it; a refusal, an expiry or a missing window is an error the
/// tool returns as is.
pub async fn request(
    tool: &str,
    summary: String,
    details: Value,
    client: &str,
) -> Result<(), String> {
    request_with_ttl(tool, summary, details, client, APPROVAL_TTL_SECS).await
}

async fn request_with_ttl(
    tool: &str,
    summary: String,
    details: Value,
    client: &str,
    ttl_secs: i64,
) -> Result<(), String> {
    if super::host::get().is_none() {
        return Err(
            "APPROVAL_UNAVAILABLE: writing to the Telegram account needs the OmniGet window open to approve it"
                .into(),
        );
    }
    let created = now();
    let req = Request {
        id: format!("tgappr_{}", uuid::Uuid::new_v4().simple()),
        tool: tool.to_string(),
        summary,
        details,
        client: client.to_string(),
        created,
        expires_at: created + ttl_secs,
    };
    let rx = insert(req.clone());
    emit(
        "telegram-approval-request",
        serde_json::to_value(&req).unwrap_or(Value::Null),
    );
    let outcome = tokio::time::timeout(Duration::from_secs(ttl_secs.max(1) as u64), rx).await;
    match outcome {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err(format!(
            "APPROVAL_DENIED: the person refused {tool} in the OmniGet window. Do not retry unless they ask."
        )),
        // Timed out, or the sender was dropped without an answer.
        _ => {
            if remove(&req.id).is_some() {
                emit(
                    "telegram-approval-resolved",
                    json!({ "id": req.id, "status": "expired" }),
                );
            }
            Err(format!(
                "APPROVAL_EXPIRED: nobody answered the approval for {tool} within {} minutes",
                ttl_secs / 60
            ))
        }
    }
}

fn insert(req: Request) -> oneshot::Receiver<bool> {
    let (tx, rx) = oneshot::channel();
    if let Ok(mut t) = table().lock() {
        t.insert(req.id.clone(), Pending { req, tx });
    }
    rx
}

fn remove(id: &str) -> Option<Pending> {
    table().lock().ok()?.remove(id)
}

/// Requests still waiting, oldest first (for a window that opens late).
pub fn pending() -> Vec<Request> {
    let cutoff = now();
    let mut list: Vec<Request> = table()
        .lock()
        .map(|t| {
            t.values()
                .filter(|p| p.req.expires_at > cutoff)
                .map(|p| p.req.clone())
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|r| r.created);
    list
}

/// The person's answer. The waiting tool resumes right away.
pub fn answer(id: &str, approve: bool) -> Result<Value, String> {
    let p = remove(id)
        .ok_or_else(|| format!("APPROVAL_NOT_FOUND: {id} (already answered or expired)"))?;
    let status = if approve { "approved" } else { "denied" };
    // A dropped receiver means the tool call already gave up.
    let delivered = p.tx.send(approve).is_ok();
    emit(
        "telegram-approval-resolved",
        json!({ "id": id, "status": status }),
    );
    if !delivered {
        return Err("APPROVAL_EXPIRED: the tool stopped waiting".into());
    }
    Ok(json!({ "id": id, "status": status, "tool": p.req.tool }))
}
