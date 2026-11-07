use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use super::model::Platform;

pub const TTL_SECS: i64 = 300;
const WINDOW: Duration = Duration::from_secs(600);
const MAX_PER_WINDOW: usize = 20;
const MIN_GAP: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub id: String,
    pub tool: String,
    pub platform: Platform,
    pub summary: String,
    pub details: Value,
    pub client: String,
    pub created: i64,
    pub expires_at: i64,
}

struct Pending {
    req: Request,
    tx: oneshot::Sender<bool>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();

pub fn set_app(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

fn table() -> &'static Mutex<HashMap<String, Pending>> {
    static T: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

fn writes() -> &'static Mutex<HashMap<Platform, VecDeque<Instant>>> {
    static W: OnceLock<Mutex<HashMap<Platform, VecDeque<Instant>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn pace(p: Platform) -> Result<(), String> {
    let mut w = writes().lock().unwrap_or_else(|e| e.into_inner());
    let q = w.entry(p).or_default();
    while q.front().is_some_and(|t| t.elapsed() > WINDOW) {
        q.pop_front();
    }
    if q.len() >= MAX_PER_WINDOW {
        return Err(format!(
            "RATE_LIMITED: OmniGet allows at most {MAX_PER_WINDOW} agent actions on {} every 10 minutes, to keep the account safe; wait and try again",
            p.label()
        ));
    }
    if q.back().is_some_and(|t| t.elapsed() < MIN_GAP) {
        return Err(format!(
            "RATE_LIMITED: wait a few seconds between actions on {}",
            p.label()
        ));
    }
    Ok(())
}

pub fn record(p: Platform) {
    writes()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(p)
        .or_default()
        .push_back(Instant::now());
}

pub async fn request(
    tool: &str,
    platform: Platform,
    summary: String,
    details: Value,
    client: &str,
) -> Result<(), String> {
    let Some(app) = APP.get() else {
        return Err(format!(
            "APPROVAL_UNAVAILABLE: acting on the {} account needs the OmniGet window open to approve it",
            platform.label()
        ));
    };
    let created = now();
    let req = Request {
        id: format!("soappr_{}", uuid::Uuid::new_v4().simple()),
        tool: tool.to_string(),
        platform,
        summary,
        details,
        client: client.to_string(),
        created,
        expires_at: created + TTL_SECS,
    };
    let (tx, rx) = oneshot::channel();
    table().lock().unwrap_or_else(|e| e.into_inner()).insert(
        req.id.clone(),
        Pending {
            req: req.clone(),
            tx,
        },
    );
    let _ = app.emit("social-approval-request", &req);
    let outcome = tokio::time::timeout(Duration::from_secs(TTL_SECS as u64), rx).await;
    let id = req.id.clone();
    table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    let _ = app.emit("social-approval-resolved", json!({ "id": id }));
    match outcome {
        Ok(Ok(true)) => Ok(()),
        Ok(Ok(false)) => Err(format!(
            "APPROVAL_DENIED: the person refused {tool} in the OmniGet window. Do not retry unless they ask."
        )),
        _ => Err(format!(
            "APPROVAL_TIMEOUT: nobody approved {tool} in the OmniGet window within 5 minutes"
        )),
    }
}

pub fn list() -> Vec<Request> {
    let t = now();
    let mut v: Vec<Request> = table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .map(|p| p.req.clone())
        .filter(|r| r.expires_at > t)
        .collect();
    v.sort_by_key(|r| r.created);
    v
}

pub fn answer(id: &str, approve: bool) -> Result<(), String> {
    let p = table()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id)
        .ok_or("this request already expired or was answered")?;
    let _ = p.tx.send(approve);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pace_blocks_bursts() {
        let p = Platform::Tiktok;
        writes().lock().unwrap().remove(&p);
        assert!(pace(p).is_ok());
        record(p);
        assert!(pace(p).unwrap_err().starts_with("RATE_LIMITED"));
        writes().lock().unwrap().remove(&p);
    }

    #[test]
    fn answering_an_unknown_request_fails() {
        assert!(answer("soappr_missing", true).is_err());
    }
}
