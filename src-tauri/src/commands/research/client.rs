use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Semaphore;

use super::catalog::{self, Operation, Request};
use super::fail;

const ACCOUNT: &str = "scrapecreators_api_key";
const TIMEOUT: Duration = Duration::from_secs(60);
const BACKOFF: Duration = Duration::from_millis(1500);
const CONCURRENCY: usize = 4;
const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;
const LOG_LIMIT: usize = 200;
pub const DEFAULT_PREVIEW_CHARS: usize = 8000;
pub const MAX_PREVIEW_CHARS: usize = 100_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub body: Value,
    pub credits_used: Option<f64>,
    pub credits_remaining: Option<f64>,
}

#[derive(Default)]
struct Session {
    calls: Vec<Value>,
    total_calls: u64,
    credits_used: f64,
    credits_remaining: Option<f64>,
}

fn session() -> &'static Mutex<Session> {
    static SESSION: OnceLock<Mutex<Session>> = OnceLock::new();
    SESSION.get_or_init(Default::default)
}

fn gate() -> &'static Semaphore {
    static GATE: OnceLock<Semaphore> = OnceLock::new();
    GATE.get_or_init(|| Semaphore::new(CONCURRENCY))
}

fn http() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .connect_timeout(Duration::from_secs(15))
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| {
            fail(
                "NETWORK_ERROR",
                "the HTTP client could not be created",
                false,
                "restart the app",
                Value::Null,
            )
        })
}

async fn stored<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| {
            fail(
                "STORE_FAILED",
                &e.to_string(),
                true,
                "try again",
                Value::Null,
            )
        })?
        .map_err(|e| fail("STORE_FAILED", &e, true, "try again", Value::Null))
}

pub async fn key_configured() -> Result<bool, String> {
    Ok(load_key().await?.is_some())
}

async fn load_key() -> Result<Option<String>, String> {
    stored(|| crate::secrets::load_secret(crate::secrets::RESEARCH, ACCOUNT))
        .await
        .map(|key| key.filter(|k| !k.trim().is_empty()))
}

pub async fn set_key(key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() || key.len() > 512 || key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(fail(
            "INVALID_API_KEY",
            "the key is empty or has spaces",
            false,
            "paste the key exactly as shown in the ScrapeCreators dashboard",
            Value::Null,
        ));
    }
    stored(move || crate::secrets::save_secret(crate::secrets::RESEARCH, ACCOUNT, &key)).await
}

pub async fn clear_key() -> Result<(), String> {
    stored(|| crate::secrets::delete_secret(crate::secrets::RESEARCH, ACCOUNT)).await
}

pub fn no_key() -> String {
    fail(
        "NO_API_KEY",
        "no ScrapeCreators API key is saved in OmniGet",
        false,
        "ask the person to open OmniGet, go to Research and paste their ScrapeCreators API key; it cannot be passed through a tool",
        Value::Null,
    )
}

fn upstream_message(body: &Value, raw: &str) -> String {
    let found = ["message", "error", "detail"]
        .iter()
        .find_map(|key| body[*key].as_str())
        .unwrap_or(raw);
    found.chars().take(300).collect()
}

pub fn classify(status: u16, raw: &str) -> Result<Value, String> {
    let parsed: Option<Value> = serde_json::from_str(raw).ok();
    let body = parsed.clone().unwrap_or(Value::Null);
    let message = upstream_message(&body, raw);
    let said = |fallback: &str| {
        if message.trim().is_empty() {
            fallback.to_string()
        } else {
            message.clone()
        }
    };
    let extra = json!({ "status": status });
    match status {
        200..=299 => match parsed {
            Some(body) if body["success"] == json!(false) => Err(fail(
                "UPSTREAM_FAILED",
                &said("the API answered success=false"),
                false,
                "read the message; the resource may be private, removed or unsupported",
                extra,
            )),
            Some(body) if body.is_object() || body.is_array() => Ok(body),
            _ => Err(fail(
                "UPSTREAM_CHANGED",
                "the API answered something that is not JSON",
                true,
                "try again later; if it persists the API changed",
                extra,
            )),
        },
        400 | 422 => Err(fail(
            "INVALID_PARAMS",
            &said("the API refused the parameters"),
            false,
            "check the parameters of this operation with research_endpoints",
            extra,
        )),
        401 => Err(fail(
            "INVALID_API_KEY",
            "ScrapeCreators refused the saved API key",
            false,
            "ask the person to paste a valid key in OmniGet, Research",
            extra,
        )),
        402 => Err(fail(
            "OUT_OF_CREDITS",
            &said("the ScrapeCreators account has no credits left"),
            false,
            "ask the person to add credits to their ScrapeCreators account; nothing else will work until then",
            extra,
        )),
        403 => Err(fail(
            "SOURCE_BLOCKED",
            &said("the source blocks this resource"),
            false,
            "the profile or post is private, restricted or blocked for scraping; try another one or another region",
            extra,
        )),
        404 => Err(fail(
            "NOT_FOUND",
            &said("nothing was found for these parameters"),
            false,
            "check the handle or URL",
            extra,
        )),
        429 => Err(fail(
            "RATE_LIMITED",
            &said("too many requests"),
            true,
            "wait a minute and make fewer calls at once",
            extra,
        )),
        _ => Err(fail(
            "UPSTREAM_ERROR",
            &said("the API failed"),
            status >= 500,
            "try again in a moment",
            extra,
        )),
    }
}

fn retry_worthy(error: &str) -> bool {
    error.starts_with("UPSTREAM_ERROR") && error.contains("\"retryable\":true")
        || error.starts_with("TIMEOUT")
}

fn number(body: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| body[*key].as_f64())
}

pub fn reply(body: Value) -> Reply {
    Reply {
        credits_used: number(&body, &["credits_charged", "credits_used"]),
        credits_remaining: number(&body, &["credits_remaining"]),
        body,
    }
}

async fn once(request: &Request, key: &str) -> Result<Value, String> {
    let client = http()?;
    let mut builder = if request.post {
        client.post(&request.url)
    } else {
        client.get(&request.url)
    };
    if let Some(body) = &request.body {
        builder = builder.json(body);
    }
    let network = |e: reqwest::Error| {
        if e.is_timeout() {
            fail(
                "TIMEOUT",
                &format!("no answer after {} s", TIMEOUT.as_secs()),
                true,
                "try again; some sources are slow",
                Value::Null,
            )
        } else {
            fail(
                "NETWORK_ERROR",
                "the request could not reach api.scrapecreators.com",
                true,
                "check the internet connection and try again",
                Value::Null,
            )
        }
    };
    let response = builder
        .header("x-api-key", key)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(network)?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.map_err(network)?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(fail(
            "RESPONSE_TOO_LARGE",
            &format!("the answer has {} bytes", bytes.len()),
            false,
            "ask for less: use trim=true or a smaller page",
            Value::Null,
        ));
    }
    classify(status, &String::from_utf8_lossy(&bytes))
}

fn record(op: &Operation, started: Instant, outcome: &Result<Reply, String>) {
    let Ok(mut session) = session().lock() else {
        return;
    };
    let mut row = json!({
        "endpoint": op.id,
        "at_ms": crate::commands::social::store::now_ms(),
        "ms": started.elapsed().as_millis() as u64,
    });
    match outcome {
        Ok(reply) => {
            row["ok"] = json!(true);
            row["credits_used"] = json!(reply.credits_used);
            session.credits_used += reply.credits_used.unwrap_or(0.0);
            if reply.credits_remaining.is_some() {
                session.credits_remaining = reply.credits_remaining;
            }
        }
        Err(error) => {
            row["ok"] = json!(false);
            row["code"] = json!(error.split(':').next().unwrap_or_default());
        }
    }
    session.total_calls += 1;
    session.calls.push(row);
    if session.calls.len() > LOG_LIMIT {
        let extra = session.calls.len() - LOG_LIMIT;
        session.calls.drain(..extra);
    }
}

pub async fn send(op: &Operation, params: &Value) -> Result<Reply, String> {
    let request = catalog::request(op, params)?;
    let key = load_key().await?.ok_or_else(no_key)?;
    let _permit = gate().acquire().await.map_err(|_| {
        fail(
            "UNAVAILABLE",
            "the request queue is closed",
            true,
            "try again",
            Value::Null,
        )
    })?;
    let started = Instant::now();
    let mut outcome = once(&request, &key).await;
    if outcome.as_ref().is_err_and(|e| retry_worthy(e)) {
        tokio::time::sleep(BACKOFF).await;
        outcome = once(&request, &key).await;
    }
    let outcome = outcome.map(reply);
    record(op, started, &outcome);
    outcome
}

pub async fn get(endpoint: &str, params: Value) -> Result<Value, String> {
    send(catalog::find(endpoint)?, &params)
        .await
        .map(|reply| reply.body)
}

pub fn usage(limit: usize) -> Value {
    let Ok(session) = session().lock() else {
        return json!({"calls": [], "total": 0, "returned": 0, "truncated": false});
    };
    let recent: Vec<Value> = session.calls.iter().rev().take(limit).cloned().collect();
    json!({
        "credits_remaining": session.credits_remaining,
        "session_credits_used": session.credits_used,
        "total": session.total_calls,
        "returned": recent.len(),
        "truncated": (session.total_calls as usize) > recent.len(),
        "calls": recent,
    })
}

pub fn credits_spent() -> f64 {
    session().lock().map(|s| s.credits_used).unwrap_or(0.0)
}

pub struct Preview {
    pub text: String,
    pub total: usize,
    pub truncated: bool,
}

pub fn preview(body: &Value, max_chars: usize) -> Preview {
    let full = serde_json::to_string_pretty(body).unwrap_or_default();
    let total = full.chars().count();
    if total <= max_chars {
        return Preview {
            text: full,
            total,
            truncated: false,
        };
    }
    Preview {
        text: full.chars().take(max_chars).collect(),
        total,
        truncated: true,
    }
}

pub fn save(body: &Value, wanted: Option<&str>, label: &str) -> Result<PathBuf, String> {
    let path = match wanted.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => {
            let dir = std::env::temp_dir().join("omniget-research");
            std::fs::create_dir_all(&dir).map_err(|e| {
                fail(
                    "WRITE_FAILED",
                    &e.to_string(),
                    false,
                    "pass saveTo with a writable path",
                    Value::Null,
                )
            })?;
            dir.join(format!(
                "{label}-{}.json",
                crate::commands::social::store::now_ms()
            ))
        }
    };
    if !path.is_absolute() {
        return Err(fail(
            "INVALID_PARAMS",
            "saveTo must be an absolute path",
            false,
            "pass an absolute path of a file that does not exist yet",
            Value::Null,
        ));
    }
    let text = serde_json::to_string_pretty(body).unwrap_or_default();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| {
            let exists = e.kind() == std::io::ErrorKind::AlreadyExists;
            fail(
                if exists {
                    "OUTPUT_EXISTS"
                } else {
                    "WRITE_FAILED"
                },
                &format!("cannot create {}: {e}", path.display()),
                false,
                "pass saveTo with a new file name in an existing folder",
                Value::Null,
            )
        })?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| {
        fail(
            "WRITE_FAILED",
            &e.to_string(),
            false,
            "check the disk and try again",
            Value::Null,
        )
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(error: &str) -> (&str, Value) {
        let (code, detail) = error.split_once(": ").unwrap();
        (code, serde_json::from_str(detail).unwrap())
    }

    #[test]
    fn status_codes_map_to_clear_codes_with_recovery_fields() {
        for (status, body, code, retryable) in [
            (
                400,
                r#"{"success":false,"message":"handle is required"}"#,
                "INVALID_PARAMS",
                false,
            ),
            (
                401,
                r#"{"success":false,"message":"Invalid API key"}"#,
                "INVALID_API_KEY",
                false,
            ),
            (
                402,
                r#"{"success":false,"message":"Out of credits"}"#,
                "OUT_OF_CREDITS",
                false,
            ),
            (403, r#"{"success":false}"#, "SOURCE_BLOCKED", false),
            (
                404,
                r#"{"success":false,"message":"User not found"}"#,
                "NOT_FOUND",
                false,
            ),
            (429, "Too Many Requests", "RATE_LIMITED", true),
            (500, "<html>oops</html>", "UPSTREAM_ERROR", true),
            (503, "", "UPSTREAM_ERROR", true),
        ] {
            let error = classify(status, body).unwrap_err();
            let (found, detail) = parts(&error);
            assert_eq!(found, code, "{status}");
            assert_eq!(detail["retryable"], retryable, "{status}");
            assert_eq!(detail["status"], status);
            assert!(
                !detail["next_step"].as_str().unwrap().is_empty(),
                "{status}"
            );
            assert!(!detail["message"].as_str().unwrap().is_empty(), "{status}");
        }
        let (_, detail) = parts(&classify(400, r#"{"message":"handle is required"}"#).unwrap_err());
        assert_eq!(detail["message"], "handle is required");
    }

    #[test]
    fn a_success_body_is_returned_and_soft_failures_are_errors() {
        let ok = classify(
            200,
            r#"{"success":true,"credits_remaining":9,"credits_charged":1,"user":{}}"#,
        )
        .unwrap();
        let reply = reply(ok);
        assert_eq!(reply.credits_used, Some(1.0));
        assert_eq!(reply.credits_remaining, Some(9.0));
        let soft = classify(200, r#"{"success":false,"error":"Video is private"}"#).unwrap_err();
        let (code, detail) = parts(&soft);
        assert_eq!(
            (code, detail["message"].as_str()),
            ("UPSTREAM_FAILED", Some("Video is private"))
        );
        assert!(classify(200, "<html>")
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
        assert!(classify(200, "\"text\"")
            .unwrap_err()
            .starts_with("UPSTREAM_CHANGED"));
    }

    #[test]
    fn only_server_errors_and_timeouts_are_retried() {
        assert!(retry_worthy(&classify(502, "").unwrap_err()));
        assert!(retry_worthy(&fail("TIMEOUT", "x", true, "y", Value::Null)));
        assert!(!retry_worthy(&classify(429, "").unwrap_err()));
        assert!(!retry_worthy(&classify(402, "").unwrap_err()));
        assert!(!retry_worthy(&classify(400, "").unwrap_err()));
    }

    #[test]
    fn a_long_upstream_message_is_cut() {
        let long = "x".repeat(2000);
        let (_, detail) = parts(&classify(404, &long).unwrap_err());
        assert_eq!(detail["message"].as_str().unwrap().len(), 300);
    }

    #[test]
    fn the_preview_is_capped_by_characters_and_reports_the_total() {
        let body = json!({"items": ["ação", "b", "c"], "n": 1});
        let whole = preview(&body, 1000);
        assert!(!whole.truncated);
        assert_eq!(whole.total, whole.text.chars().count());
        let cut = preview(&body, 10);
        assert!(cut.truncated);
        assert_eq!(cut.text.chars().count(), 10);
        assert_eq!(cut.total, whole.total);
    }

    #[test]
    fn the_full_body_is_saved_once_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        let body = json!({"a": 1});
        let saved = save(&body, path.to_str(), "x").unwrap();
        assert_eq!(saved, path);
        let read: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(read, body);
        assert!(save(&body, path.to_str(), "x")
            .unwrap_err()
            .starts_with("OUTPUT_EXISTS"));
        assert!(save(&body, Some("relative.json"), "x")
            .unwrap_err()
            .starts_with("INVALID_PARAMS"));
    }

    #[test]
    fn the_missing_key_error_tells_the_agent_what_the_person_must_do() {
        let error = no_key();
        let (code, detail) = parts(&error);
        assert_eq!(code, "NO_API_KEY");
        assert!(detail["next_step"].as_str().unwrap().contains("Research"));
    }

    #[tokio::test]
    async fn a_key_is_stored_checked_and_cleared_without_being_readable_back() {
        let dir = tempfile::tempdir().unwrap();
        let env = crate::secrets::test_env::EnvGuard::acquire();
        env.set(
            omniget_core::core::secrets::SECRETS_DIR_ENV,
            dir.path().to_str(),
        );
        assert!(!key_configured().await.unwrap());
        assert!(set_key("  ".into())
            .await
            .unwrap_err()
            .starts_with("INVALID_API_KEY"));
        assert!(set_key("has space".into()).await.is_err());
        set_key(" abc123 ".into()).await.unwrap();
        assert!(key_configured().await.unwrap());
        clear_key().await.unwrap();
        assert!(!key_configured().await.unwrap());
        let error = send(
            catalog::find("account_credit_balance").unwrap(),
            &Value::Null,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("NO_API_KEY"));
    }
}
