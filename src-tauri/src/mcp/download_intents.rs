//! Durable admission journal for the existing queue. This is not a scheduler.
//! Missing evidence of a completed effect never licenses automatic replay.
use super::policy::{self, Principal};
use omniget_core::core::secure_files::{Identity, Root};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    /// Persisted form is the redacted display URL; `sealed` means the
    /// executable URL lives in the encrypted secret store (see [`hydrate`]).
    pub url: String,
    #[serde(default)]
    pub sealed: bool,
    pub mode: Option<String>,
    pub quality: Option<String>,
    pub format_id: Option<String>,
    pub audio_format: Option<String>,
    pub subtitles: bool,
    pub auto_subtitles: bool,
    pub fragments: u32,
    pub parent: String,
    pub parent_identity: Identity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Intent {
    pub job_id: u64,
    pub principal: String,
    pub key: String,
    pub stage: String,
    pub destination: String,
    pub destination_identity: Option<Identity>,
    pub options: Options,
    #[serde(default)]
    pub attempt: u32,
}
/// Worker path ceiling for parallel fragments. The worker runs yt-dlp with
/// no fragment retries through the broker; its default (4) is the proven
/// value, the desktop's 8 broke a long YouTube DASH download (bench D3).
pub const WORKER_MAX_FRAGMENTS: u32 = 4;
/// Explicit retries per job (the shared retry ceiling).
pub const MAX_RETRIES: u32 = 2;
/// At most this many produced files are exposed per job.
pub const MAX_JOB_ARTIFACTS: usize = 20;
/// A job folder untouched for this long has no live writer: reconciliation
/// after a crash may judge its contents.
pub const RECONCILE_QUIET_MS: u64 = 10_000;
/// Executable URLs that carry credentials (signed queries, tokens) never go
/// into the journal in clear; only while the job may still run.
const EXEC_URLS: crate::secrets::Namespace = crate::secrets::Namespace {
    service: "wtf.tonho.omniget.mcp-downloads",
    dir_name: "mcp_downloads",
    dir_env: "OMNIGET_MCP_DOWNLOADS_SECRET_DIR",
};
fn exec_account(id: u64) -> String {
    format!("download-url-{id}")
}
/// Redacted display URL for persistence; the raw one when it differs.
fn seal(mut options: Options) -> (Options, Option<String>) {
    let display = crate::core::flight_recorder::redact_url(&options.url);
    if display == options.url {
        return (options, None);
    }
    let raw = std::mem::replace(&mut options.url, display);
    options.sealed = true;
    (options, Some(raw))
}
/// The intent with the executable URL, for execution only.
pub fn hydrate(mut i: Intent) -> Result<Intent, String> {
    if i.options.sealed {
        i.options.url = crate::secrets::load_secret(EXEC_URLS, &exec_account(i.job_id))
            .map_err(|_| "EXECUTION_URL_UNAVAILABLE")?
            .ok_or("EXECUTION_URL_UNAVAILABLE")?;
        i.options.sealed = false;
    }
    Ok(i)
}
fn forget_exec_url(i: &Intent) {
    if i.options.sealed {
        let _ = crate::secrets::delete_secret(EXEC_URLS, &exec_account(i.job_id));
    }
}

fn error(_: rusqlite::Error) -> String {
    "DOWNLOAD_INTENT_STORE_UNAVAILABLE".into()
}
fn schema(c: &Connection) -> Result<(), String> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS download_intents(job_id INTEGER PRIMARY KEY,principal TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,body TEXT NOT NULL,stage TEXT NOT NULL,UNIQUE(principal,key));
CREATE TABLE IF NOT EXISTS download_attempts(job_id INTEGER NOT NULL,number INTEGER NOT NULL,operation TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,snapshot TEXT NOT NULL,terminal TEXT,settled INTEGER NOT NULL DEFAULT 0,next_allowed INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(job_id,number),UNIQUE(job_id,operation,key));
CREATE TABLE IF NOT EXISTS download_host_cooldowns(host_hash TEXT PRIMARY KEY,next_allowed INTEGER NOT NULL);
INSERT OR IGNORE INTO download_attempts(job_id,number,operation,key,fingerprint,snapshot,settled) SELECT job_id,0,'download_enqueue',key,fingerprint,body,CASE WHEN stage IN ('prepared','admitting','enqueued') THEN 1 ELSE 0 END FROM download_intents;").map_err(error)
}
fn connection() -> Result<Connection, String> {
    let c = policy::db()?;
    schema(&c)?;
    Ok(c)
}
fn hash(args: &Value) -> String {
    format!("{:x}", Sha256::digest(args.to_string().as_bytes()))
}
fn decode(body: String, stage: String) -> Result<Intent, String> {
    let mut i: Intent = serde_json::from_str(&body).map_err(|_| "DOWNLOAD_INTENT_INVALID")?;
    i.stage = stage;
    Ok(i)
}
fn load_in(c: &Connection, id: u64) -> Result<Option<Intent>, String> {
    let row: Option<(String, String)> = c
        .query_row(
            "SELECT body,stage FROM download_intents WHERE job_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(error)?;
    row.map(|(b, s)| decode(b, s)).transpose()
}
pub fn load(id: u64) -> Result<Option<Intent>, String> {
    load_in(&connection()?, id)
}
pub fn by_key(p: &Principal, key: &str, args: &Value) -> Result<Option<Intent>, String> {
    policy::active(p)?;
    let c = connection()?;
    let row: Option<(String, String, String)> = c
        .query_row(
            "SELECT body,stage,fingerprint FROM download_intents WHERE principal=?1 AND key=?2",
            params![p.id, key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    match row {
        Some((b, s, h)) if h == hash(args) => Ok(Some(decode(b, s)?)),
        Some(_) => Err("IDEMPOTENCY_CONFLICT".into()),
        None => Ok(None),
    }
}
pub fn reserve(p: &Principal, key: &str, args: &Value, options: Options) -> Result<Intent, String> {
    policy::active(p)?;
    if !policy::allowed(p, "download_enqueue") {
        return Err("ENQUEUE_NOT_GRANTED".into());
    }
    let raw = options.url.clone();
    let i = reserve_in(&mut connection()?, p, key, args, options)?;
    // Same key + fingerprint means the same raw URL: rewriting the secret is
    // idempotent and repairs a crash between the commit and this write.
    if i.options.sealed && i.stage != "terminal" {
        crate::secrets::save_secret(EXEC_URLS, &exec_account(i.job_id), &raw)
            .map_err(|_| "EXECUTION_URL_STORE_UNAVAILABLE")?;
    }
    Ok(i)
}
fn reserve_in(
    c: &mut Connection,
    p: &Principal,
    key: &str,
    args: &Value,
    options: Options,
) -> Result<Intent, String> {
    if key.is_empty() || key.len() > 100 {
        return Err("INVALID_IDEMPOTENCY_KEY".into());
    }
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let previous: Option<(String, String, String)> = tx
        .query_row(
            "SELECT body,stage,fingerprint FROM download_intents WHERE principal=?1 AND key=?2",
            params![p.id, key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(error)?;
    if let Some((b, s, h)) = previous {
        if h != hash(args) {
            return Err("IDEMPOTENCY_CONFLICT".into());
        }
        return decode(b, s);
    }
    // A legacy reservation without our original options cannot be reconciled.
    let legacy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM receipts WHERE principal=?1 AND operation='download_enqueue' AND key=?2)",params![p.id,key],|r|r.get(0)).map_err(error)?;
    if legacy {
        return Err("OUTCOME_UNKNOWN: legacy intent has no durable execution snapshot".into());
    }
    let id = ((uuid::Uuid::new_v4().as_u128() as u64) & ((1u64 << 52) - 1)).max(1);
    let (options, _) = seal(options);
    let destination = Path::new(&options.parent).join(format!("omniget-mcp-{id}"));
    let i = Intent {
        job_id: id,
        principal: p.id.clone(),
        key: key.into(),
        stage: "prepared".into(),
        attempt: 0,
        destination: destination.to_str().ok_or("INVALID_DESTINATION")?.into(),
        destination_identity: None,
        options,
    };
    tx.execute(
        "INSERT INTO owners(download,principal) VALUES(?1,?2)",
        params![id, p.id],
    )
    .map_err(error)?;
    tx.execute(
        "INSERT INTO receipts VALUES(?1,'download_enqueue',?2,?3,NULL)",
        params![p.id, key, hash(args)],
    )
    .map_err(error)?;
    tx.execute(
        "INSERT INTO download_intents VALUES(?1,?2,?3,?4,?5,'prepared')",
        params![
            id,
            p.id,
            key,
            hash(args),
            serde_json::to_string(&i).map_err(|_| "DOWNLOAD_INTENT_INVALID")?
        ],
    )
    .map_err(error)?;
    tx.execute("INSERT INTO download_attempts(job_id,number,operation,key,fingerprint,snapshot,settled) VALUES(?1,0,'download_enqueue',?2,?3,?4,1)",params![id,key,hash(args),serde_json::to_string(&i).map_err(|_|"DOWNLOAD_INTENT_INVALID")?]).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(i)
}
/// Create the private destination once. Crash between mkdir and identity
/// persistence is ambiguous: never adopt an existing directory by pathname.
pub fn prepare_destination(p: &Principal, id: u64) -> Result<Intent, String> {
    policy::active(p)?;
    let mut c = connection()?;
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let mut i = load_in(&tx, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if i.principal != p.id {
        return Err("DOWNLOAD_NOT_FOUND".into());
    }
    let parent = Root::open(
        Path::new(&i.options.parent),
        Some(&i.options.parent_identity),
    )
    .map_err(|_| "DESTINATION_ROOT_CHANGED")?;
    if let Some(identity) = &i.destination_identity {
        Root::open(Path::new(&i.destination), Some(identity)).map_err(|_| "DESTINATION_CHANGED")?;
    } else {
        let leaf = Path::new(&i.destination)
            .file_name()
            .ok_or("DESTINATION_INVALID")?;
        parent
            .create_directory(Path::new(leaf))
            .map_err(|_| "OUTCOME_UNKNOWN: destination exists without an identity receipt")?;
        let root = Root::open(Path::new(&i.destination), None).map_err(|_| "DESTINATION_UNSAFE")?;
        i.destination_identity = Some(root.identity().clone());
        tx.execute(
            "UPDATE download_intents SET body=?2 WHERE job_id=?1",
            params![
                id,
                serde_json::to_string(&i).map_err(|_| "DOWNLOAD_INTENT_INVALID")?
            ],
        )
        .map_err(error)?;
    }
    tx.commit().map_err(error)?;
    Ok(i)
}
/// Called under the existing queue lock immediately before constructing item.
/// It must precede q.enqueue and is safe to replay: actual effect admission is
/// a separate durable gate, before the first worker invocation.
pub fn admitting(id: u64) -> Result<(), String> {
    let c = connection()?;
    let n=c.execute("UPDATE download_intents SET stage='admitting' WHERE job_id=?1 AND stage IN ('prepared','admitting','enqueued')",[id]).map_err(error)?;
    if n == 1 {
        Ok(())
    } else {
        Err("OUTCOME_UNKNOWN: execution cannot be replayed".into())
    }
}
pub fn enqueued(id: u64) -> Result<(), String> {
    let c = connection()?;
    let n = c
        .execute(
            "UPDATE download_intents SET stage='enqueued' WHERE job_id=?1 AND stage='admitting'",
            [id],
        )
        .map_err(error)?;
    if n == 1 {
        Ok(())
    } else {
        Err("DOWNLOAD_ADMISSION_CONFLICT".into())
    }
}
/// Durable effect gate. A second dispatcher or recovered executing record is
/// denied. Queue item and principal must agree, even when called by local UI.
pub fn before_execute(id: u64) -> Result<Intent, String> {
    // Resolve the executable URL before the durable gate: a missing secret
    // must fail while the attempt is still replayable, not after `executing`.
    let url = hydrate(load(id)?.ok_or("DOWNLOAD_INTENT_MISSING")?)?
        .options
        .url;
    let mut i = before_execute_in(&mut connection()?, id)?;
    i.options.url = url;
    i.options.sealed = false;
    Ok(i)
}
/// The current attempt is a `download_resume` of a paused attempt: a user
/// action, not a failure recovery, so failure cooldowns and the retry budget
/// do not apply to it.
fn resumes_pause(c: &Connection, id: u64, attempt: u32) -> Result<bool, String> {
    if attempt == 0 {
        return Ok(false);
    }
    let row: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT a.operation,p.terminal FROM download_attempts a LEFT JOIN download_attempts p ON p.job_id=a.job_id AND p.number=a.number-1 WHERE a.job_id=?1 AND a.number=?2",
            params![id, attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(error)?;
    Ok(row.is_some_and(|(op, prior)| {
        op == "download_resume"
            && prior
                .as_deref()
                .and_then(|t| serde_json::from_str::<TerminalReceipt>(t).ok())
                .is_some_and(|t| t.outcome == "paused")
    }))
}
/// Explicit failure retries spent by this job; resumes of a pause are free.
fn retries_used_in(c: &Connection, id: u64) -> Result<u32, String> {
    let attempts: Vec<u32> = {
        let mut q = c
            .prepare("SELECT number FROM download_attempts WHERE job_id=?1 AND number>0")
            .map_err(error)?;
        let rows = q.query_map([id], |r| r.get(0)).map_err(error)?;
        rows.collect::<Result<_, _>>().map_err(error)?
    };
    let mut used = 0;
    for n in attempts {
        if !resumes_pause(c, id, n)? {
            used += 1;
        }
    }
    Ok(used)
}
pub fn retries_used(id: u64) -> Result<u32, String> {
    retries_used_in(&connection()?, id)
}
fn before_execute_in(c: &mut Connection, id: u64) -> Result<Intent, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let i = load_in(&tx, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    let p = tx
        .query_row(
            "SELECT id,name,scopes FROM clients WHERE id=?1 AND revoked=0",
            [&i.principal],
            |r| {
                Ok(Principal {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    scopes: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                })
            },
        )
        .map_err(|_| "PRINCIPAL_INACTIVE")?;
    if !policy::allowed(&p, "download_enqueue") {
        return Err("ENQUEUE_NOT_GRANTED".into());
    }
    Root::open(
        Path::new(&i.options.parent),
        Some(&i.options.parent_identity),
    )
    .map_err(|_| "DESTINATION_ROOT_CHANGED")?;
    Root::open(
        Path::new(&i.destination),
        Some(
            i.destination_identity
                .as_ref()
                .ok_or("DESTINATION_UNPREPARED")?,
        ),
    )
    .map_err(|_| "DESTINATION_CHANGED")?;
    if !resumes_pause(&tx, id, i.attempt)? {
        let host = host_hash(&i.options.url)?;
        let cooldown: u64 = tx
            .query_row(
                "SELECT COALESCE(MAX(next_allowed),0) FROM download_host_cooldowns WHERE host_hash=?1",
                [host],
                |r| r.get(0),
            )
            .map_err(error)?;
        if now_ms() < cooldown {
            return Err(format!("RETRY_COOLDOWN_UNTIL:{cooldown}"));
        }
    }
    let n = tx
        .execute(
            "UPDATE download_intents SET stage='executing' WHERE job_id=?1 AND stage='enqueued'",
            [id],
        )
        .map_err(error)?;
    if n != 1 {
        return Err("OUTCOME_UNKNOWN: prior execution requires reconciliation".into());
    }
    tx.execute(
        "UPDATE download_attempts SET settled=0 WHERE job_id=?1 AND number=?2",
        params![id, i.attempt],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(i)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalReceipt {
    pub outcome: String,
    pub error: Option<String>,
    pub file_path: Option<String>,
    pub bytes: Option<u64>,
    pub ended_at: u64,
    pub next_allowed_at: u64,
    pub retryable: bool,
    /// Every produced file of a successful attempt; `bytes` is their sum
    /// (the engine result names only the last file of a carousel).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ReceiptFile>,
    /// Settled by reconciliation after a crash, not by the worker's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciled: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReceiptFile {
    pub name: String,
    pub bytes: u64,
}
/// A user action or a crash, not a server answer: never a retry cooldown.
fn is_user_or_crash_outcome(outcome: &str, message: &str) -> bool {
    matches!(outcome, "paused" | "cancelled") || message.starts_with("INTERRUPTED")
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn host_hash(url: &str) -> Result<String, String> {
    let u = url::Url::parse(url).map_err(|_| "INVALID_URL")?;
    Ok(format!(
        "{:x}",
        Sha256::digest(
            u.host_str()
                .ok_or("INVALID_URL")?
                .to_ascii_lowercase()
                .as_bytes()
        )
    ))
}
fn retry_after(error: &str) -> Option<u64> {
    // WorkerFailure exposes only numeric seconds; never parse a provider URL
    // or invent a shorter interval when an explicit value is very large.
    error
        .split("retry_after_seconds=")
        .nth(1)
        .and_then(|v| v.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|v| v.parse().ok())
}
pub fn terminal_receipt(
    id: u64,
    success: bool,
    error_message: Option<String>,
    file_path: Option<String>,
    bytes: Option<u64>,
    retryable: bool,
) -> Result<(), String> {
    let outcome = if success { "success" } else { "failed" };
    let files = if success {
        load(id)?.map(|i| sized_job_files(&i)).unwrap_or_default()
    } else {
        vec![]
    };
    let mut c = connection()?;
    record_terminal_full_in(
        &mut c,
        id,
        outcome,
        error_message,
        file_path,
        bytes,
        retryable,
        now_ms(),
        files,
        None,
    )?;
    forget_if_final(&c, id);
    Ok(())
}
/// Produced files of the job with their sizes, best effort (the receipt is
/// evidence, the artifact listing re-validates every file).
fn sized_job_files(i: &Intent) -> Vec<ReceiptFile> {
    let Ok((names, _)) = job_files(i) else {
        return vec![];
    };
    names
        .into_iter()
        .filter_map(|name| {
            let meta = std::fs::symlink_metadata(Path::new(&i.destination).join(&name)).ok()?;
            meta.is_file().then(|| ReceiptFile {
                name,
                bytes: meta.len(),
            })
        })
        .collect()
}
/// Drop the protected executable URL once no attempt can use it again.
fn forget_if_final(c: &Connection, id: u64) {
    let Ok(Some(i)) = load_in(c, id) else { return };
    let Ok(Some(r)) = receipt_in_conn(c, &i) else {
        return;
    };
    let spent = retries_used_in(c, id).unwrap_or(MAX_RETRIES) >= MAX_RETRIES;
    if r.outcome == "success" || !r.retryable || (r.outcome != "paused" && spent) {
        forget_exec_url(&i);
    }
}
#[allow(clippy::too_many_arguments)]
fn record_terminal_in(
    c: &mut Connection,
    id: u64,
    outcome: &str,
    error_message: Option<String>,
    file_path: Option<String>,
    bytes: Option<u64>,
    retryable: bool,
    now: u64,
) -> Result<(), String> {
    record_terminal_full_in(
        c,
        id,
        outcome,
        error_message,
        file_path,
        bytes,
        retryable,
        now,
        vec![],
        None,
    )
}
#[allow(clippy::too_many_arguments)]
fn record_terminal_full_in(
    c: &mut Connection,
    id: u64,
    outcome: &str,
    error_message: Option<String>,
    file_path: Option<String>,
    bytes: Option<u64>,
    retryable: bool,
    now: u64,
    files: Vec<ReceiptFile>,
    reconciled: Option<String>,
) -> Result<(), String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let i = load_in(&tx, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if !matches!(
        i.stage.as_str(),
        "executing" | "enqueued" | "admitting" | "prepared"
    ) {
        return Err("OUTCOME_UNKNOWN: no current effect to settle".into());
    }
    write_terminal(
        &tx,
        &i,
        outcome,
        error_message,
        file_path,
        bytes,
        retryable,
        now,
        files,
        reconciled,
    )?;
    tx.commit().map_err(error)
}
#[allow(clippy::too_many_arguments)]
fn write_terminal(
    tx: &Connection,
    i: &Intent,
    outcome: &str,
    error_message: Option<String>,
    file_path: Option<String>,
    bytes: Option<u64>,
    retryable: bool,
    now: u64,
    files: Vec<ReceiptFile>,
    reconciled: Option<String>,
) -> Result<(), String> {
    let id = i.job_id;
    let message = error_message.as_deref().unwrap_or("");
    if message.contains("OUTCOME_UNKNOWN")
        || message == "Download interrupted"
        || message.contains("WORKER_TERMINATION_UNCONFIRMED")
    {
        return Err("OUTCOME_UNKNOWN: interrupted effect".into());
    }
    // The class decides the minimum wait (rate limit 60 s, platform block
    // 15 min, otherwise 5 s); an explicit Retry-After can only lengthen it.
    // A pause, a cancel or a crash is no answer from the server: no wait.
    let diagnosis = crate::core::root_cause::machine_diagnose(message);
    let delay = if outcome == "success" || is_user_or_crash_outcome(outcome, message) {
        0
    } else {
        retry_after(message)
            .unwrap_or(0)
            .max(diagnosis.cooldown_seconds())
    };
    // A platform block names this post/job, not the host: other jobs on the
    // same host keep the ordinary short spacing.
    let host_delay = if diagnosis.code == "BLOCKED_BY_PLATFORM" {
        delay.min(5)
    } else {
        delay
    };
    let next = now
        .saturating_add(delay.saturating_mul(1000))
        .min(i64::MAX as u64);
    let bytes = if files.is_empty() {
        bytes
    } else {
        Some(files.iter().map(|f| f.bytes).sum())
    };
    let receipt = TerminalReceipt {
        outcome: outcome.into(),
        error: error_message,
        file_path,
        bytes,
        ended_at: now,
        next_allowed_at: next,
        retryable,
        files,
        reconciled,
    };
    let body = serde_json::to_string(&receipt).map_err(|_| "INVALID_TERMINAL_RECEIPT")?;
    let n=tx.execute("UPDATE download_attempts SET terminal=?3,next_allowed=?4 WHERE job_id=?1 AND number=?2 AND terminal IS NULL",params![id,i.attempt,body,next]).map_err(error)?;
    if n != 1 {
        return Err("TERMINAL_RECEIPT_CONFLICT".into());
    }
    tx.execute(
        "UPDATE download_intents SET stage='terminal' WHERE job_id=?1",
        [id],
    )
    .map_err(error)?;
    if host_delay > 0 {
        let next = now
            .saturating_add(host_delay.saturating_mul(1000))
            .min(i64::MAX as u64);
        tx.execute("INSERT INTO download_host_cooldowns VALUES(?1,?2) ON CONFLICT(host_hash) DO UPDATE SET next_allowed=MAX(next_allowed,excluded.next_allowed)",params![host_hash(&i.options.url)?,next]).map_err(error)?;
    }
    Ok(())
}
fn receipt_in_conn(c: &Connection, i: &Intent) -> Result<Option<TerminalReceipt>, String> {
    let body: Option<String> = c
        .query_row(
            "SELECT terminal FROM download_attempts WHERE job_id=?1 AND number=?2",
            params![i.job_id, i.attempt],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?
        .flatten();
    body.map(|b| serde_json::from_str(&b).map_err(|_| "INVALID_TERMINAL_RECEIPT".into()))
        .transpose()
}
pub fn receipt(id: u64) -> Result<Option<TerminalReceipt>, String> {
    let c = connection()?;
    let i = load_in(&c, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    receipt_in_conn(&c, &i)
}
/// A normal wrapper return is not enough after a dropped worker future.
/// Call only after the worker acknowledges kill/wait and broker shutdown.
pub fn settled(id: u64, attempt: u32, cancelled: Option<&str>) -> Result<(), String> {
    let mut c = connection()?;
    let i = load_in(&c, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if i.attempt != attempt {
        return Err("STALE_ATTEMPT".into());
    }
    if let Some(outcome) = cancelled {
        if i.stage != "terminal" {
            record_terminal_in(
                &mut c,
                id,
                outcome,
                Some(outcome.into()),
                None,
                None,
                // A cancelled attempt is not advertised as retryable (status
                // says so too); the URL can be enqueued again. A pause is
                // resumable through download_resume.
                outcome == "paused",
                now_ms(),
            )?;
        }
    }
    c.execute(
        "UPDATE download_attempts SET settled=1 WHERE job_id=?1 AND number=?2",
        params![id, attempt],
    )
    .map_err(error)?;
    forget_if_final(&c, id);
    Ok(())
}
/// A pause settles the attempt as `paused` (resumable) and keeps its partial
/// files. Cancelling it afterwards turns that receipt into `cancelled`, so the
/// job folder is cleaned like any cancelled attempt (D-06). No-op otherwise.
pub fn cancel_paused(id: u64) -> Result<Vec<String>, String> {
    let mut c = connection()?;
    if !cancel_paused_in(&mut c, id)? {
        return Ok(vec![]);
    }
    forget_if_final(&c, id);
    cleanup_failed_attempt(id)
}
fn cancel_paused_in(c: &mut Connection, id: u64) -> Result<bool, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let Some(i) = load_in(&tx, id)? else {
        return Ok(false);
    };
    if i.stage != "terminal" {
        return Ok(false);
    }
    let settled: bool = tx
        .query_row(
            "SELECT settled FROM download_attempts WHERE job_id=?1 AND number=?2",
            params![id, i.attempt],
            |r| r.get(0),
        )
        .map_err(error)?;
    let Some(mut r) = receipt_in_conn(&tx, &i)? else {
        return Ok(false);
    };
    if !settled || r.outcome != "paused" {
        return Ok(false);
    }
    r.outcome = "cancelled".into();
    r.error = Some("cancelled".into());
    r.retryable = false;
    let body = serde_json::to_string(&r).map_err(|_| "INVALID_TERMINAL_RECEIPT")?;
    tx.execute(
        "UPDATE download_attempts SET terminal=?3 WHERE job_id=?1 AND number=?2",
        params![id, i.attempt, body],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(true)
}
pub fn replay_receipt(
    p: &Principal,
    operation: &str,
    key: &str,
    args: &Value,
) -> Result<Option<Value>, String> {
    policy::active(p)?;
    let c = connection()?;
    let row:Option<(String,Option<String>)>=c.query_row("SELECT fingerprint,result FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3",params![p.id,operation,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(error)?;
    match row {
        Some((fingerprint, _)) if fingerprint != hash(args) => Err("IDEMPOTENCY_CONFLICT".into()),
        Some((_, Some(result))) => serde_json::from_str(&result)
            .map(Some)
            .map_err(|_| "INVALID_OPERATION_RECEIPT".into()),
        _ => Ok(None),
    }
}
pub fn prepare_retry(
    p: &Principal,
    id: u64,
    key: &str,
    operation: &str,
    args: &Value,
) -> Result<Intent, String> {
    policy::active(p)?;
    if !policy::allowed(p, "download_enqueue") || !policy::allowed(p, operation) {
        return Err("RETRY_NOT_GRANTED".into());
    }
    prepare_retry_in(&mut connection()?, p, id, key, operation, args, now_ms())
}
fn prepare_retry_in(
    c: &mut Connection,
    p: &Principal,
    id: u64,
    key: &str,
    operation: &str,
    args: &Value,
    now: u64,
) -> Result<Intent, String> {
    if key.is_empty()
        || key.len() > 100
        || !matches!(operation, "download_retry" | "download_resume")
    {
        return Err("INVALID_RETRY_INTENT".into());
    }
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let mut i = load_in(&tx, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if i.principal != p.id {
        return Err("DOWNLOAD_NOT_FOUND".into());
    }
    let prior:Option<(u32,String)>=tx.query_row("SELECT number,fingerprint FROM download_attempts WHERE job_id=?1 AND operation=?2 AND key=?3",params![id,operation,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(error)?;
    if let Some((number, fingerprint)) = prior {
        if fingerprint != hash(args) {
            return Err("IDEMPOTENCY_CONFLICT".into());
        }
        if number != i.attempt {
            return Err("ATTEMPT_ALREADY_SUPERSEDED".into());
        }
        return Ok(i);
    }
    if i.stage != "terminal" {
        return Err("OUTCOME_UNKNOWN_OR_ATTEMPT_ACTIVE".into());
    }
    let (terminal,settled,next):(Option<String>,bool,u64)=tx.query_row("SELECT terminal,settled,next_allowed FROM download_attempts WHERE job_id=?1 AND number=?2",params![id,i.attempt],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(error)?;
    if !settled {
        return Err("WORKER_TERMINATION_UNCONFIRMED".into());
    }
    let terminal: TerminalReceipt = serde_json::from_str(&terminal.ok_or("OUTCOME_UNKNOWN")?)
        .map_err(|_| "INVALID_TERMINAL_RECEIPT")?;
    if !matches!(terminal.outcome.as_str(), "failed" | "cancelled" | "paused")
        || !terminal.retryable
    {
        return Err("RETRY_REQUIRES_LOCAL_ACTION_OR_MORE_EVIDENCE".into());
    }
    // Resuming a pause is the user continuing their own job: no failure
    // cooldown and no retry budget (D-03). Everything else is a retry.
    let resume = operation == "download_resume" && terminal.outcome == "paused";
    if !resume {
        if retries_used_in(&tx, id)? >= MAX_RETRIES {
            return Err("RETRY_LIMIT_REACHED".into());
        }
        let host = host_hash(&i.options.url)?;
        let host_next: u64 = tx
            .query_row(
                "SELECT COALESCE(MAX(next_allowed),0) FROM download_host_cooldowns WHERE host_hash=?1",
                [&host],
                |r| r.get(0),
            )
            .map_err(error)?;
        if now < next.max(host_next) {
            return Err(format!("RETRY_COOLDOWN_UNTIL:{}", next.max(host_next)));
        }
    }
    let legacy: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3)",
            params![p.id, operation, key],
            |r| r.get(0),
        )
        .map_err(error)?;
    if legacy {
        return Err("OUTCOME_UNKNOWN: legacy retry has no attempt snapshot".into());
    }
    i.attempt += 1;
    i.stage = "prepared".into();
    // Parallel fragments over the mediated egress are the likely cause of a
    // fragment failure: the one bounded retry uses a single connection.
    if terminal
        .error
        .as_deref()
        .is_some_and(|e| e.contains("BROKEN_SOURCE"))
    {
        i.options.fragments = 1;
    }
    let snapshot = serde_json::to_string(&i).map_err(|_| "DOWNLOAD_INTENT_INVALID")?;
    tx.execute("INSERT INTO download_attempts(job_id,number,operation,key,fingerprint,snapshot,settled,next_allowed) VALUES(?1,?2,?3,?4,?5,?6,1,?7)",params![id,i.attempt,operation,key,hash(args),snapshot,now]).map_err(error)?;
    tx.execute(
        "UPDATE download_intents SET body=?2,stage='prepared' WHERE job_id=?1",
        params![id, snapshot],
    )
    .map_err(error)?;
    tx.execute(
        "INSERT INTO receipts VALUES(?1,?2,?3,?4,NULL)",
        params![p.id, operation, key, hash(args)],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(i)
}
/// Engine leftovers and OS bookkeeping that are never a produced artifact.
/// A leading dot alone is not one: the engine may name a real output
/// `......name.mp4` from a hostile URL, and hiding it turned a completed job
/// into success without artifacts (D-07).
pub fn is_partial_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    is_os_junk(&lower)
        || lower.ends_with(".part")
        || lower.ends_with(".part.resume.json")
        || lower.contains(".part-frag")
        || lower.ends_with(".ytdl")
        || lower.ends_with(".temp")
        || lower.ends_with(".tmp")
        || is_intermediate_stream(&lower)
}
fn is_os_junk(lower: &str) -> bool {
    matches!(
        lower,
        ".ds_store" | ".localized" | "thumbs.db" | "desktop.ini"
    ) || lower.starts_with("._")
        || lower.starts_with(".nfs")
        || lower.starts_with(".fuse_hidden")
}
/// yt-dlp per-format intermediates such as `name.f251.webm` / `name.f398.mp4`.
fn is_intermediate_stream(lower: &str) -> bool {
    let mut parts = lower.rsplit('.');
    let (_ext, format) = (parts.next(), parts.next());
    format.is_some_and(|f| {
        f.len() > 1 && f.starts_with('f') && f[1..].bytes().all(|b| b.is_ascii_digit())
    })
}
/// Produced files of a completed job, read from its exclusive folder (the
/// engine result names only the last file of a carousel). Sorted, bounded.
pub fn job_files(i: &Intent) -> Result<(Vec<String>, bool), String> {
    let identity = i
        .destination_identity
        .as_ref()
        .ok_or("ARTIFACT_ROOT_IDENTITY_MISSING")?;
    list_job_files(Path::new(&i.destination), identity)
}
fn list_job_files(dir: &Path, identity: &Identity) -> Result<(Vec<String>, bool), String> {
    Root::open(dir, Some(identity)).map_err(|_| "DESTINATION_CHANGED")?;
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|_| "ARTIFACT_UNAVAILABLE")?
        .take(1024)
    {
        let entry = entry.map_err(|_| "ARTIFACT_UNAVAILABLE")?;
        // file_type() does not follow symlinks: links and directories are skipped.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_partial_name(&name) {
            names.push(name);
        }
    }
    names.sort();
    let truncated = names.len() > MAX_JOB_ARTIFACTS;
    names.truncate(MAX_JOB_ARTIFACTS);
    Ok((names, truncated))
}
/// After a failed or cancelled attempt, remove what it left in the job's own
/// exclusive folder (partials, fragments, .ytdl, thumbnails, intermediates and
/// any rejected output). A failed job exposes no artifact, and a later
/// successful attempt in the same folder must not expose stale files. Only
/// runs once the attempt is durably terminal (worker teardown acknowledged).
/// Returns the removed file names for the job journal.
pub fn cleanup_failed_attempt(id: u64) -> Result<Vec<String>, String> {
    let i = load(id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if i.stage != "terminal" {
        return Err("CLEANUP_REQUIRES_TERMINAL_ATTEMPT".into());
    }
    let r = receipt(id)?.ok_or("CLEANUP_REQUIRES_TERMINAL_ATTEMPT")?;
    if !matches!(r.outcome.as_str(), "failed" | "cancelled") {
        return Ok(vec![]);
    }
    let identity = i
        .destination_identity
        .as_ref()
        .ok_or("DESTINATION_UNPREPARED")?;
    cleanup_dir(Path::new(&i.destination), identity)
}
fn cleanup_dir(dir: &Path, identity: &Identity) -> Result<Vec<String>, String> {
    Root::open(dir, Some(identity)).map_err(|_| "DESTINATION_CHANGED")?;
    let mut removed = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|_| "CLEANUP_UNAVAILABLE")?
        .take(4096)
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if std::fs::remove_file(dir.join(&name)).is_ok() {
            removed.push(name);
        }
    }
    removed.sort();
    Ok(removed)
}

/// Local UI retry cannot bypass explicit durable attempt admission.
pub fn retry_is_prepared(id: u64) -> bool {
    load(id).ok().flatten().is_some_and(|i| {
        i.attempt > 0 && matches!(i.stage.as_str(), "prepared" | "admitting" | "enqueued")
    })
}

/// Reconciliation cannot prove whether an admitted effect completed.
pub fn unknown(id: u64) -> Result<(), String> {
    connection()?
        .execute(
            "UPDATE download_intents SET stage='unknown' WHERE job_id=?1 AND stage='executing'",
            [id],
        )
        .map_err(error)?;
    Ok(())
}
/// Owned intents whose effect may have run and nobody settled (a crash):
/// the URL stays managed by that job until it is reconciled. A job still live
/// in the queue (`live`) is running, not interrupted. URLs are compared
/// exactly: two signed links that differ only in a secret redact to the same
/// display string, so a sealed intent is compared through its stored URL.
pub fn unresolved_for_url(p: &Principal, url: &str, live: &[u64]) -> Result<Option<u64>, String> {
    let display = crate::core::flight_recorder::redact_url(url);
    let c = connection()?;
    let mut q = c
        .prepare("SELECT body,stage FROM download_intents WHERE principal=?1 AND stage IN ('unknown','executing')")
        .map_err(error)?;
    let rows = q
        .query_map([&p.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(error)?;
    for row in rows {
        let (b, s) = row.map_err(error)?;
        let i = decode(b, s)?;
        if live.contains(&i.job_id) || i.options.url != display {
            continue;
        }
        let same = if i.options.sealed {
            // Unreadable stored URL: keep the conservative answer.
            hydrate(i.clone())
                .map(|h| h.options.url == url)
                .unwrap_or(true)
        } else {
            true
        };
        if same {
            return Ok(Some(i.job_id));
        }
    }
    Ok(None)
}
/// Every attempt of a job, oldest first, as bounded evidence (D-10).
pub fn attempts(id: u64) -> Result<Vec<Value>, String> {
    let c = connection()?;
    let mut q = c
        .prepare("SELECT number,operation,terminal,settled FROM download_attempts WHERE job_id=?1 ORDER BY number LIMIT 16")
        .map_err(error)?;
    let rows = q
        .query_map([id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })
        .map_err(error)?;
    let mut out = vec![];
    for row in rows {
        let (number, operation, terminal, settled) = row.map_err(error)?;
        let r = terminal.and_then(|t| serde_json::from_str::<TerminalReceipt>(&t).ok());
        out.push(serde_json::json!({
            "attempt": number,
            "operation": operation,
            "outcome": r.as_ref().map(|r| r.outcome.clone()),
            "error": r.as_ref().and_then(|r| r.error.as_deref()).map(|e| crate::core::flight_recorder::redact(&e.chars().take(240).collect::<String>())),
            "endedAt": r.as_ref().map(|r| r.ended_at),
            "retryable": r.as_ref().map(|r| r.retryable),
            "reconciled": r.as_ref().and_then(|r| r.reconciled.clone()),
            "settled": settled,
        }));
    }
    Ok(out)
}
/// A resume restarts from zero for every engine: what the paused attempt left
/// is removed first, so the claim made to the client is always true (yt-dlp
/// would continue some formats and the direct path none). Only before the
/// new attempt is admitted. Returns (files, bytes) discarded.
pub fn discard_for_restart(i: &Intent) -> Result<(usize, u64), String> {
    let Some(identity) = i
        .destination_identity
        .as_ref()
        .filter(|_| i.stage == "prepared")
    else {
        return Ok((0, 0));
    };
    let dir = Path::new(&i.destination);
    let scan = scan_folder(dir, identity)?;
    let bytes = scan.partial_bytes + scan.complete.iter().map(|f| f.bytes).sum::<u64>();
    let removed = cleanup_dir(dir, identity)?;
    Ok((removed.len(), bytes))
}
#[derive(Debug, Default, Clone)]
pub struct FolderScan {
    pub complete: Vec<ReceiptFile>,
    pub partial_files: usize,
    pub partial_bytes: u64,
    pub newest_ms: u64,
}
fn scan_folder(dir: &Path, identity: &Identity) -> Result<FolderScan, String> {
    Root::open(dir, Some(identity)).map_err(|_| "DESTINATION_CHANGED")?;
    let mtime = |m: &std::fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64)
    };
    // A live writer touches the file it writes; that is the signal.
    let mut scan = FolderScan::default();
    for entry in std::fs::read_dir(dir)
        .map_err(|_| "ARTIFACT_UNAVAILABLE")?
        .take(1024)
    {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        scan.newest_ms = scan.newest_ms.max(mtime(&meta));
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_os_junk(&name.to_ascii_lowercase()) {
            continue;
        }
        if is_partial_name(&name) {
            scan.partial_files += 1;
            scan.partial_bytes += meta.len();
        } else if meta.len() > 0 {
            scan.complete.push(ReceiptFile {
                name,
                bytes: meta.len(),
            });
        }
    }
    scan.complete.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(scan)
}
/// Settle a job whose attempt was running when the app died (D-08). The
/// caller has established that no live queue item runs it in this process;
/// a folder written to recently is refused (a writer may still be alive).
/// Complete output with no partial leftovers confirms the effect; anything
/// else is an interrupted, retryable attempt. Never starts a download.
pub fn reconcile(p: Option<&Principal>, id: u64) -> Result<(String, Vec<String>), String> {
    let i = load(id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    if p.is_some_and(|p| p.id != i.principal) {
        return Err("DOWNLOAD_NOT_FOUND".into());
    }
    if i.stage == "terminal" {
        return Ok(("already_terminal".into(), vec![]));
    }
    let identity = i
        .destination_identity
        .clone()
        .ok_or("RECONCILE_NOT_NEEDED: the attempt never started; replay its enqueue key")?;
    let dir = Path::new(&i.destination);
    let scan = scan_folder(dir, &identity)?;
    let mut c = connection()?;
    let outcome = reconcile_in(&mut c, id, &scan, now_ms())?;
    let removed = if outcome == "interrupted" {
        cleanup_dir(dir, &identity)?
    } else {
        vec![]
    };
    forget_if_final(&c, id);
    Ok((outcome.into(), removed))
}
fn reconcile_in(
    c: &mut Connection,
    id: u64,
    scan: &FolderScan,
    now: u64,
) -> Result<&'static str, String> {
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let i = load_in(&tx, id)?.ok_or("DOWNLOAD_INTENT_MISSING")?;
    match i.stage.as_str() {
        "terminal" => return Ok("already_terminal"),
        "unknown" | "executing" => {}
        _ => {
            return Err(
                "RECONCILE_NOT_NEEDED: the attempt never started; replay its enqueue key".into(),
            )
        }
    }
    let quiet_until = scan.newest_ms.saturating_add(RECONCILE_QUIET_MS);
    if now < quiet_until {
        return Err(format!("RECONCILE_WRITER_ACTIVE:{quiet_until}"));
    }
    let outcome = if !scan.complete.is_empty() && scan.partial_files == 0 {
        let first = Path::new(&i.destination).join(&scan.complete[0].name);
        write_terminal(
            &tx,
            &i,
            "success",
            None,
            first.to_str().map(str::to_owned),
            None,
            false,
            now,
            scan.complete.clone(),
            Some("artifact_present_without_partials".into()),
        )?;
        "completed"
    } else {
        write_terminal(&tx, &i, "failed", Some("INTERRUPTED: the app stopped while this download was running; no complete artifact was confirmed".into()), None, None, true, now, vec![], Some("no_complete_artifact".into()))?;
        "interrupted"
    };
    // The folder is quiet and no process of this app runs the attempt.
    tx.execute(
        "UPDATE download_attempts SET settled=1 WHERE job_id=?1 AND number=?2",
        params![id, i.attempt],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(outcome)
}
pub fn all() -> Result<Vec<Intent>, String> {
    let c = connection()?;
    let mut q = c
        .prepare("SELECT body,stage FROM download_intents WHERE stage!='terminal'")
        .map_err(error)?;
    let rows = q
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(error)?;
    rows.map(|r| {
        let (b, s) = r.map_err(error)?;
        decode(b, s)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE receipts(principal TEXT,operation TEXT,key TEXT,fingerprint TEXT,result TEXT,PRIMARY KEY(principal,operation,key)); CREATE TABLE owners(download INTEGER PRIMARY KEY,principal TEXT);").unwrap();
        schema(&c).unwrap();
        c
    }
    fn opts() -> Options {
        Options {
            url: "https://example.com/a".into(),
            sealed: false,
            mode: Some("video".into()),
            quality: Some("720".into()),
            format_id: None,
            audio_format: None,
            subtitles: false,
            auto_subtitles: false,
            fragments: 4,
            parent: "/tmp/fixture".into(),
            parent_identity: Identity {
                device: 1,
                inode: 2,
            },
        }
    }
    fn p() -> Principal {
        Principal {
            id: "A".into(),
            name: "A".into(),
            scopes: vec!["enqueue".into()],
        }
    }
    #[test]
    fn same_intent_same_id_and_options_after_effect_window() {
        let mut c = db();
        let a = serde_json::json!({"url":"https://example.com/a"});
        let first = reserve_in(&mut c, &p(), "key", &a, opts()).unwrap();
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        let mut changed = opts();
        changed.quality = Some("1080".into());
        let replay = reserve_in(&mut c, &p(), "key", &a, changed).unwrap();
        assert_eq!(first.job_id, replay.job_id);
        assert_eq!(replay.stage, "executing");
        assert_eq!(replay.options.quality, Some("720".into()));
        assert_eq!(
            c.query_row("SELECT count(*) FROM owners", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert!(reserve_in(
            &mut c,
            &p(),
            "key",
            &serde_json::json!({"url":"different"}),
            opts()
        )
        .is_err());
    }
    #[test]
    fn legacy_unknown_is_not_reissued() {
        let mut c = db();
        c.execute(
            "INSERT INTO receipts VALUES('A','download_enqueue','legacy','hash',NULL)",
            [],
        )
        .unwrap();
        assert!(
            reserve_in(&mut c, &p(), "legacy", &serde_json::json!({}), opts())
                .unwrap_err()
                .starts_with("OUTCOME_UNKNOWN")
        );
    }
    #[test]
    fn transaction_rolls_back_owner_on_failed_reservation() {
        let mut c = db();
        c.execute_batch("CREATE TRIGGER fail_intent BEFORE INSERT ON download_intents BEGIN SELECT RAISE(ABORT,'disk fixture'); END;").unwrap();
        assert!(reserve_in(&mut c, &p(), "key", &serde_json::json!({}), opts()).is_err());
        assert_eq!(
            c.query_row("SELECT count(*) FROM owners", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn crash_boundaries_reopen_same_id_and_effect_is_admitted_once() {
        let temp =
            std::env::temp_dir().join(format!("omniget-intent-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        let temp = std::fs::canonicalize(temp).unwrap();
        let path = temp.join("journal.sqlite");
        let mut c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE receipts(principal TEXT,operation TEXT,key TEXT,fingerprint TEXT,result TEXT,PRIMARY KEY(principal,operation,key)); CREATE TABLE owners(download INTEGER PRIMARY KEY,principal TEXT); CREATE TABLE clients(id TEXT,name TEXT,scopes TEXT,revoked INTEGER); INSERT INTO clients VALUES('A','A','[\"enqueue\"]',0);").unwrap();
        schema(&c).unwrap();
        let root = Root::open(&temp, None).unwrap();
        let mut options = opts();
        options.parent = temp.to_str().unwrap().into();
        options.parent_identity = root.identity().clone();
        let a = serde_json::json!({"url":"https://example.com/a"});
        let mut i = reserve_in(&mut c, &p(), "crash", &a, options.clone()).unwrap();
        let id = i.job_id;
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        assert_eq!(
            reserve_in(&mut c, &p(), "crash", &a, options.clone())
                .unwrap()
                .job_id,
            id
        );
        root.create_directory(Path::new(&format!("omniget-mcp-{id}")))
            .unwrap();
        i.destination_identity = Some(
            Root::open(Path::new(&i.destination), None)
                .unwrap()
                .identity()
                .clone(),
        );
        c.execute(
            "UPDATE download_intents SET body=?2,stage='admitting' WHERE job_id=?1",
            params![id, serde_json::to_string(&i).unwrap()],
        )
        .unwrap();
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        assert_eq!(load_in(&c, id).unwrap().unwrap().stage, "admitting");
        assert!(before_execute_in(&mut c, id).is_err());
        c.execute(
            "UPDATE download_intents SET stage='enqueued' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        assert!(before_execute_in(&mut c, id).is_ok());
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        assert_eq!(
            reserve_in(&mut c, &p(), "crash", &a, options)
                .unwrap()
                .stage,
            "executing"
        );
        assert!(before_execute_in(&mut c, id)
            .unwrap_err()
            .starts_with("OUTCOME_UNKNOWN"));
        c.execute("UPDATE clients SET revoked=1", []).unwrap();
        c.execute("UPDATE download_intents SET stage='enqueued'", [])
            .unwrap();
        assert_eq!(
            before_execute_in(&mut c, id).unwrap_err(),
            "PRINCIPAL_INACTIVE"
        );
        drop(c);
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn terminal_receipt_then_explicit_attempt_is_bounded_and_idempotent() {
        let mut c = db();
        let args = serde_json::json!({"url":"https://example.com/a"});
        let initial = reserve_in(&mut c, &p(), "initial", &args, opts()).unwrap();
        let id = initial.job_id;
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        c.execute("UPDATE download_attempts SET settled=0", [])
            .unwrap();
        record_terminal_in(
            &mut c,
            id,
            "failed",
            Some("HTTP 429; retry_after_seconds=120".into()),
            None,
            None,
            true,
            1000,
        )
        .unwrap();
        let request = serde_json::json!({"download_id":id,"idempotencyKey":"retry-1"});
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                id,
                "retry-1",
                "download_retry",
                &request,
                200000
            )
            .unwrap_err(),
            "WORKER_TERMINATION_UNCONFIRMED"
        );
        c.execute("UPDATE download_attempts SET settled=1", [])
            .unwrap();
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            id,
            "retry-1",
            "download_retry",
            &request,
            120999
        )
        .unwrap_err()
        .starts_with("RETRY_COOLDOWN_UNTIL:121000"));
        let retry = prepare_retry_in(
            &mut c,
            &p(),
            id,
            "retry-1",
            "download_retry",
            &request,
            121000,
        )
        .unwrap();
        assert_eq!(retry.job_id, id);
        assert_eq!(retry.attempt, 1);
        assert_eq!(retry.options.quality, initial.options.quality);
        assert_eq!(retry.destination, initial.destination);
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                id,
                "retry-1",
                "download_retry",
                &request,
                121001
            )
            .unwrap()
            .attempt,
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM download_attempts WHERE job_id=?1",
                [id],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                id,
                "retry-1",
                "download_retry",
                &serde_json::json!({"changed":true}),
                121001
            )
            .unwrap_err(),
            "IDEMPOTENCY_CONFLICT"
        );
        record_terminal_in(&mut c, id, "cancelled", None, None, None, true, 130000).unwrap();
        let next = prepare_retry_in(
            &mut c,
            &p(),
            id,
            "retry-2",
            "download_resume",
            &serde_json::json!({"id":id}),
            135000,
        )
        .unwrap();
        assert_eq!(next.attempt, 2);
        record_terminal_in(
            &mut c,
            id,
            "failed",
            Some("network timeout".into()),
            None,
            None,
            true,
            140000,
        )
        .unwrap();
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                id,
                "retry-3",
                "download_retry",
                &serde_json::json!({"id":id}),
                150000
            )
            .unwrap_err(),
            "RETRY_LIMIT_REACHED"
        );
    }
    #[test]
    fn cooldown_shared_by_host_and_429_without_header_waits_sixty_seconds() {
        let mut c = db();
        let one = reserve_in(&mut c, &p(), "one", &serde_json::json!({"one":1}), opts()).unwrap();
        let two = reserve_in(&mut c, &p(), "two", &serde_json::json!({"two":2}), opts()).unwrap();
        record_terminal_in(
            &mut c,
            one.job_id,
            "failed",
            Some("HTTP 429 rate limited".into()),
            None,
            None,
            true,
            1000,
        )
        .unwrap();
        record_terminal_in(
            &mut c,
            two.job_id,
            "failed",
            Some("network timeout".into()),
            None,
            None,
            true,
            2000,
        )
        .unwrap();
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            two.job_id,
            "retry",
            "download_retry",
            &serde_json::json!({}),
            7000
        )
        .unwrap_err()
        .starts_with("RETRY_COOLDOWN_UNTIL:61000"));
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            two.job_id,
            "retry",
            "download_retry",
            &serde_json::json!({}),
            61000
        )
        .is_ok());
    }
    fn terminal(c: &mut Connection, id: u64, message: &str, now: u64) {
        c.execute(
            "UPDATE download_intents SET stage='executing' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        let retryable = crate::core::queue::external_retryable(message);
        record_terminal_in(
            c,
            id,
            "failed",
            Some(message.into()),
            None,
            None,
            retryable,
            now,
        )
        .unwrap();
    }
    #[test]
    fn platform_block_waits_for_its_own_cooldown_without_blocking_the_host() {
        let mut c = db();
        let blocked = reserve_in(&mut c, &p(), "b", &serde_json::json!({"b":1}), opts()).unwrap();
        let other = reserve_in(&mut c, &p(), "o", &serde_json::json!({"o":1}), opts()).unwrap();
        terminal(
            &mut c,
            blocked.job_id,
            "BLOCKED_BY_PLATFORM: the platform blocked access from this network",
            1000,
        );
        let err = prepare_retry_in(
            &mut c,
            &p(),
            blocked.job_id,
            "r",
            "download_retry",
            &serde_json::json!({}),
            2000,
        )
        .unwrap_err();
        assert_eq!(err, "RETRY_COOLDOWN_UNTIL:901000");
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            blocked.job_id,
            "r",
            "download_retry",
            &serde_json::json!({}),
            901000
        )
        .is_ok());
        // Same host, different post: only the ordinary short spacing applies.
        terminal(&mut c, other.job_id, "network timeout", 1000);
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            other.job_id,
            "r2",
            "download_retry",
            &serde_json::json!({}),
            7000
        )
        .is_ok());
    }
    #[test]
    fn broken_source_is_retryable_once_with_a_single_fragment_and_format_refusal_is_not() {
        let mut c = db();
        let i = reserve_in(&mut c, &p(), "yt", &serde_json::json!({"y":1}), opts()).unwrap();
        assert_eq!(i.options.fragments, 4);
        terminal(
            &mut c,
            i.job_id,
            "BROKEN_SOURCE: the source stopped serving part of the media (fragment failure)",
            1000,
        );
        assert!(receipt_in(&c, i.job_id).retryable);
        let retry = prepare_retry_in(
            &mut c,
            &p(),
            i.job_id,
            "r",
            "download_retry",
            &serde_json::json!({}),
            10000,
        )
        .unwrap();
        assert_eq!((retry.attempt, retry.options.fragments), (1, 1));
        let f = reserve_in(&mut c, &p(), "fmt", &serde_json::json!({"f":1}), opts()).unwrap();
        terminal(
            &mut c,
            f.job_id,
            "FORMAT_UNAVAILABLE: requested format is not available within the requested options",
            1000,
        );
        assert!(!receipt_in(&c, f.job_id).retryable);
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                f.job_id,
                "r",
                "download_retry",
                &serde_json::json!({}),
                99999999
            )
            .unwrap_err(),
            "RETRY_REQUIRES_LOCAL_ACTION_OR_MORE_EVIDENCE"
        );
        let big = reserve_in(&mut c, &p(), "big", &serde_json::json!({"g":1}), opts()).unwrap();
        terminal(
            &mut c,
            big.job_id,
            "OUTPUT_TOO_LARGE: output exceeds the artifact limit",
            1000,
        );
        assert!(!receipt_in(&c, big.job_id).retryable);
    }
    fn receipt_in(c: &Connection, id: u64) -> TerminalReceipt {
        let body: String = c.query_row("SELECT terminal FROM download_attempts WHERE job_id=?1 ORDER BY number DESC LIMIT 1", [id], |r| r.get(0)).unwrap();
        serde_json::from_str(&body).unwrap()
    }
    #[test]
    fn job_folder_lists_every_produced_file_and_cleanup_removes_leftovers() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-job-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let identity = Root::open(&dir, None).unwrap().identity().clone();
        for name in [
            "post_1.mp4",
            "post_2.mp4",
            "post_3.mp4",
            "v.f398.mp4.part",
            "v.f398.mp4.part-Frag3",
            "v.f398.mp4.ytdl",
            "v.f251.webm",
            "._post_1.mp4",
        ] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::create_dir(dir.join("sub")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", dir.join("link.mp4")).unwrap();
        let (names, truncated) = list_job_files(&dir, &identity).unwrap();
        assert_eq!(names, vec!["post_1.mp4", "post_2.mp4", "post_3.mp4"]);
        assert!(!truncated);
        let removed = cleanup_dir(&dir, &identity).unwrap();
        assert_eq!(removed.len(), 8);
        assert!(removed.contains(&"v.f398.mp4.part".to_string()));
        assert!(dir.join("sub").is_dir());
        assert!(std::fs::symlink_metadata(dir.join("link.mp4")).is_ok());
        assert!(std::path::Path::new("/etc/hosts").exists());
        for n in 0..25 {
            std::fs::write(dir.join(format!("item_{n:02}.jpg")), b"x").unwrap();
        }
        let (names, truncated) = list_job_files(&dir, &identity).unwrap();
        assert_eq!(names.len(), MAX_JOB_ARTIFACTS);
        assert!(truncated);
        // A replaced folder is never cleaned by pathname.
        let other = Identity {
            device: identity.device,
            inode: identity.inode.wrapping_add(1),
        };
        assert!(cleanup_dir(&dir, &other).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn partial_names_are_recognized() {
        for n in [
            "a.mp4.part",
            "a.mp4.part-Frag12",
            "a.ytdl",
            "a.f251.webm",
            "a.f398.mp4",
            ".DS_Store",
            "._a.mp4",
            "x.temp",
        ] {
            assert!(is_partial_name(n), "{n}");
        }
        // D-07: a produced file whose name the source forced to start with
        // dots is an artifact, not a leftover.
        for n in [
            "a.mp4",
            "a.fr.vtt",
            "a_1.jpg",
            "movie.final.mkv",
            "f.mp4",
            "......gv-enc-escape.mp4",
            ".hidden.mp4",
        ] {
            assert!(!is_partial_name(n), "{n}");
        }
    }
    #[test]
    fn unknown_success_and_foreign_principal_cannot_be_reissued() {
        let mut c = db();
        let i = reserve_in(&mut c, &p(), "one", &serde_json::json!({}), opts()).unwrap();
        for stage in ["executing", "unknown"] {
            c.execute("UPDATE download_intents SET stage=?1", [stage])
                .unwrap();
            assert_eq!(
                prepare_retry_in(
                    &mut c,
                    &p(),
                    i.job_id,
                    "r",
                    "download_retry",
                    &serde_json::json!({}),
                    999999
                )
                .unwrap_err(),
                "OUTCOME_UNKNOWN_OR_ATTEMPT_ACTIVE"
            );
        }
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        assert!(record_terminal_in(
            &mut c,
            i.job_id,
            "failed",
            Some("WORKER_TERMINATION_UNCONFIRMED".into()),
            None,
            None,
            true,
            1000
        )
        .is_err());
        record_terminal_in(
            &mut c,
            i.job_id,
            "success",
            None,
            Some("artifact".into()),
            Some(123),
            false,
            1000,
        )
        .unwrap();
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                i.job_id,
                "r",
                "download_retry",
                &serde_json::json!({}),
                999999
            )
            .unwrap_err(),
            "RETRY_REQUIRES_LOCAL_ACTION_OR_MORE_EVIDENCE"
        );
        let mut foreign = p();
        foreign.id = "B".into();
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &foreign,
                i.job_id,
                "r",
                "download_retry",
                &serde_json::json!({}),
                999999
            )
            .unwrap_err(),
            "DOWNLOAD_NOT_FOUND"
        );
    }
    #[test]
    fn terminal_write_failure_preserves_executing_and_absence_of_evidence() {
        let mut c = db();
        let i = reserve_in(&mut c, &p(), "one", &serde_json::json!({}), opts()).unwrap();
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        c.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF terminal ON download_attempts BEGIN SELECT RAISE(ABORT,'fixture fsync failure'); END;").unwrap();
        assert!(record_terminal_in(
            &mut c,
            i.job_id,
            "failed",
            Some("network".into()),
            None,
            None,
            true,
            1000
        )
        .is_err());
        assert_eq!(load_in(&c, i.job_id).unwrap().unwrap().stage, "executing");
        assert!(c
            .query_row("SELECT terminal IS NULL FROM download_attempts", [], |r| {
                r.get::<_, bool>(0)
            })
            .unwrap());
    }
    #[test]
    fn terminal_and_new_attempt_crash_boundaries_reopen_without_new_effect() {
        let dir =
            std::env::temp_dir().join(format!("omniget-attempt-crash-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("journal.sqlite");
        let mut c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE receipts(principal TEXT,operation TEXT,key TEXT,fingerprint TEXT,result TEXT,PRIMARY KEY(principal,operation,key)); CREATE TABLE owners(download INTEGER PRIMARY KEY,principal TEXT);").unwrap();
        schema(&c).unwrap();
        let i = reserve_in(&mut c, &p(), "original", &serde_json::json!({}), opts()).unwrap();
        let id = i.job_id;
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        c.execute("UPDATE download_attempts SET settled=0", [])
            .unwrap();
        record_terminal_in(
            &mut c,
            id,
            "failed",
            Some("network timeout".into()),
            None,
            None,
            true,
            1000,
        )
        .unwrap();
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        let args = serde_json::json!({"id":id});
        assert_eq!(
            prepare_retry_in(&mut c, &p(), id, "retry", "download_retry", &args, 10000)
                .unwrap_err(),
            "WORKER_TERMINATION_UNCONFIRMED"
        );
        c.execute("UPDATE download_attempts SET settled=1", [])
            .unwrap();
        let retry =
            prepare_retry_in(&mut c, &p(), id, "retry", "download_retry", &args, 10000).unwrap();
        drop(c);
        let mut c = Connection::open(&path).unwrap();
        schema(&c).unwrap();
        let replay =
            prepare_retry_in(&mut c, &p(), id, "retry", "download_retry", &args, 10000).unwrap();
        assert_eq!(replay.attempt, 1);
        assert_eq!(replay.stage, "prepared");
        assert_eq!(replay.options.quality, retry.options.quality);
        assert_eq!(replay.job_id, id);
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM download_attempts", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            2
        );
        drop(c);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn pause_is_not_a_failure_and_resume_skips_cooldown_and_retry_budget() {
        // D-03: pause then resume within seconds used to hit RETRY_COOLDOWN_UNTIL.
        let mut c = db();
        let i = reserve_in(&mut c, &p(), "k", &serde_json::json!({"k":1}), opts()).unwrap();
        let id = i.job_id;
        for n in 1..=3u32 {
            let now = 1000 + n as u64;
            c.execute(
                "UPDATE download_intents SET stage='executing' WHERE job_id=?1",
                [id],
            )
            .unwrap();
            record_terminal_in(
                &mut c,
                id,
                "paused",
                Some("paused".into()),
                None,
                None,
                true,
                now,
            )
            .unwrap();
            assert_eq!(receipt_in(&c, id).next_allowed_at, now);
            let r = prepare_retry_in(
                &mut c,
                &p(),
                id,
                &format!("resume-{n}"),
                "download_resume",
                &serde_json::json!({"n":n}),
                now + 1,
            )
            .unwrap();
            assert_eq!(r.attempt, n);
            assert!(resumes_pause(&c, id, n).unwrap());
        }
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM download_host_cooldowns", [], |r| r
                .get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(retries_used_in(&c, id).unwrap(), 0);
        // Another job on the host is rate limited: a resume of a pause is not
        // held by that failure cooldown...
        let other = reserve_in(&mut c, &p(), "o", &serde_json::json!({"o":1}), opts()).unwrap();
        terminal(&mut c, other.job_id, "HTTP 429 rate limited", 2000);
        c.execute(
            "UPDATE download_intents SET stage='executing' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        record_terminal_in(
            &mut c,
            id,
            "paused",
            Some("paused".into()),
            None,
            None,
            true,
            2001,
        )
        .unwrap();
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            id,
            "resume-4",
            "download_resume",
            &serde_json::json!({"n":4}),
            2002
        )
        .is_ok());
        // ...while a failure retry still waits for it and spends the budget.
        c.execute(
            "UPDATE download_intents SET stage='executing' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        record_terminal_in(
            &mut c,
            id,
            "failed",
            Some("network timeout".into()),
            None,
            None,
            true,
            2003,
        )
        .unwrap();
        assert!(prepare_retry_in(
            &mut c,
            &p(),
            id,
            "retry-1",
            "download_retry",
            &serde_json::json!({"r":1}),
            2004
        )
        .unwrap_err()
        .starts_with("RETRY_COOLDOWN_UNTIL:"));
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                id,
                "retry-1",
                "download_retry",
                &serde_json::json!({"r":1}),
                70000
            )
            .unwrap()
            .attempt,
            5
        );
        assert!(!resumes_pause(&c, id, 5).unwrap());
        assert_eq!(retries_used_in(&c, id).unwrap(), 1);
    }
    #[test]
    fn resume_gate_ignores_host_cooldown_only_for_a_paused_attempt() {
        let temp = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-resume-gate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        let mut c = db();
        c.execute_batch("CREATE TABLE clients(id TEXT,name TEXT,scopes TEXT,revoked INTEGER); INSERT INTO clients VALUES('A','A','[\"enqueue\"]',0);").unwrap();
        let root = Root::open(&temp, None).unwrap();
        let mut options = opts();
        options.parent = temp.to_str().unwrap().into();
        options.parent_identity = root.identity().clone();
        let mut i = reserve_in(&mut c, &p(), "g", &serde_json::json!({"g":1}), options).unwrap();
        let id = i.job_id;
        root.create_directory(Path::new(&format!("omniget-mcp-{id}")))
            .unwrap();
        i.destination_identity = Some(
            Root::open(Path::new(&i.destination), None)
                .unwrap()
                .identity()
                .clone(),
        );
        c.execute(
            "UPDATE download_intents SET body=?2,stage='executing' WHERE job_id=?1",
            params![id, serde_json::to_string(&i).unwrap()],
        )
        .unwrap();
        record_terminal_in(
            &mut c,
            id,
            "paused",
            Some("paused".into()),
            None,
            None,
            true,
            now_ms(),
        )
        .unwrap();
        prepare_retry_in(
            &mut c,
            &p(),
            id,
            "resume",
            "download_resume",
            &serde_json::json!({}),
            now_ms(),
        )
        .unwrap();
        c.execute(
            "INSERT INTO download_host_cooldowns VALUES(?1,?2)",
            params![
                host_hash("https://example.com/a").unwrap(),
                now_ms() + 600_000
            ],
        )
        .unwrap();
        c.execute(
            "UPDATE download_intents SET stage='enqueued' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        assert_eq!(before_execute_in(&mut c, id).unwrap().attempt, 1);
        // A failure retry on the same host is still held by the cooldown.
        record_terminal_in(
            &mut c,
            id,
            "failed",
            Some("network timeout".into()),
            None,
            None,
            true,
            1,
        )
        .unwrap();
        c.execute_batch(
            "UPDATE download_attempts SET settled=1; DELETE FROM download_host_cooldowns;",
        )
        .unwrap();
        prepare_retry_in(
            &mut c,
            &p(),
            id,
            "retry",
            "download_retry",
            &serde_json::json!({"r":1}),
            10_000,
        )
        .unwrap();
        c.execute(
            "INSERT INTO download_host_cooldowns VALUES(?1,?2)",
            params![
                host_hash("https://example.com/a").unwrap(),
                now_ms() + 600_000
            ],
        )
        .unwrap();
        c.execute(
            "UPDATE download_intents SET stage='enqueued' WHERE job_id=?1",
            [id],
        )
        .unwrap();
        assert!(before_execute_in(&mut c, id)
            .unwrap_err()
            .starts_with("RETRY_COOLDOWN_UNTIL:"));
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn cancel_after_pause_settles_cancelled_only_once_the_pause_is_settled() {
        // D-06: a paused attempt keeps partials; cancelling it must turn it
        // into a cancelled (cleanable, non-resumable) attempt.
        let mut c = db();
        let i = reserve_in(&mut c, &p(), "k", &serde_json::json!({"k":1}), opts()).unwrap();
        c.execute("UPDATE download_intents SET stage='executing'", [])
            .unwrap();
        c.execute("UPDATE download_attempts SET settled=0", [])
            .unwrap();
        record_terminal_in(
            &mut c,
            i.job_id,
            "paused",
            Some("paused".into()),
            None,
            None,
            true,
            1000,
        )
        .unwrap();
        assert!(
            !cancel_paused_in(&mut c, i.job_id).unwrap(),
            "worker teardown not acknowledged yet"
        );
        c.execute("UPDATE download_attempts SET settled=1", [])
            .unwrap();
        assert!(cancel_paused_in(&mut c, i.job_id).unwrap());
        let r = receipt_in(&c, i.job_id);
        assert_eq!((r.outcome.as_str(), r.retryable), ("cancelled", false));
        assert!(!cancel_paused_in(&mut c, i.job_id).unwrap());
        assert_eq!(
            prepare_retry_in(
                &mut c,
                &p(),
                i.job_id,
                "r",
                "download_resume",
                &serde_json::json!({}),
                99999
            )
            .unwrap_err(),
            "RETRY_REQUIRES_LOCAL_ACTION_OR_MORE_EVIDENCE"
        );
    }
    #[test]
    fn signed_urls_are_persisted_redacted() {
        // D-11: the journal body and attempt snapshots hold the display form.
        let mut c = db();
        let mut options = opts();
        options.url =
            "https://cdn.example.com/v.mp4?X-Amz-Signature=SYNTHETIC_SECRET&token=SYNTHETIC_SECRET"
                .into();
        let i = reserve_in(&mut c, &p(), "s", &serde_json::json!({"s":1}), options).unwrap();
        assert!(i.options.sealed);
        assert!(i.options.url.starts_with("https://cdn.example.com/v.mp4?"));
        for sql in [
            "SELECT body FROM download_intents",
            "SELECT snapshot FROM download_attempts",
        ] {
            let body: String = c.query_row(sql, [], |r| r.get(0)).unwrap();
            assert!(!body.contains("SYNTHETIC_SECRET"), "{sql}: {body}");
        }
        assert_eq!(
            host_hash(&i.options.url).unwrap(),
            host_hash("https://cdn.example.com/x").unwrap()
        );
        let (plain, raw) = seal(opts());
        assert!(!plain.sealed && raw.is_none());
    }
    #[test]
    fn a_running_job_is_not_an_interrupted_one() {
        // G04 (gates runner): a second enqueue while the first job is still
        // running was refused as "interrupted" because `executing` counted as
        // unresolved. A job live in the queue is running, not lost.
        let (dir, p) = store();
        let running = job(&dir, &p, "running");
        executing(running.job_id);
        assert_eq!(
            unresolved_for_url(&p, "https://example.com/a", &[running.job_id]).unwrap(),
            None
        );
        assert_eq!(
            unresolved_for_url(&p, "https://example.com/a", &[]).unwrap(),
            Some(running.job_id),
            "not in the queue after a crash: unresolved"
        );
        assert_eq!(
            unresolved_for_url(&p, "https://example.com/other", &[]).unwrap(),
            None
        );
    }
    /// Journal on disk through the real `connection()`, like the app.
    fn store() -> (std::path::PathBuf, Principal) {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-intents-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        policy::TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.join("policy")));
        let grant = policy::create("A".into(), vec!["enqueue".into(), "control".into()]).unwrap();
        std::fs::create_dir(dir.join("media")).unwrap();
        (dir, grant.principal)
    }
    fn job(dir: &Path, p: &Principal, key: &str) -> Intent {
        let media = dir.join("media");
        let mut options = opts();
        options.parent = media.to_str().unwrap().into();
        options.parent_identity = Root::open(&media, None).unwrap().identity().clone();
        let i = reserve_in(
            &mut connection().unwrap(),
            p,
            key,
            &serde_json::json!({"key":key}),
            options,
        )
        .unwrap();
        prepare_destination(p, i.job_id).unwrap()
    }
    fn executing(id: u64) {
        connection()
            .unwrap()
            .execute(
                "UPDATE download_intents SET stage='executing' WHERE job_id=?1",
                [id],
            )
            .unwrap();
        connection()
            .unwrap()
            .execute(
                "UPDATE download_attempts SET settled=0 WHERE job_id=?1",
                [id],
            )
            .unwrap();
    }
    fn old_file(path: &Path, body: &[u8]) {
        std::fs::write(path, body).unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(past)
            .unwrap();
    }
    #[test]
    fn crash_mid_download_is_visible_and_reconciles_without_a_new_download() {
        // D-08: after kill -9 the job was invisible and stuck unknown.
        let (dir, p) = store();
        let lost = job(&dir, &p, "lost");
        executing(lost.job_id);
        unknown(lost.job_id).unwrap();
        old_file(&Path::new(&lost.destination).join("hang.mp4.part"), b"");
        // Visible from the journal, with a typed way out, while unsettled.
        let view = crate::mcp::downloads::journal_item(&load(lost.job_id).unwrap().unwrap());
        assert_eq!(view["status"]["type"], "Unknown");
        assert_eq!(view["status"]["data"]["effectMayHaveRun"], true);
        assert_eq!(
            unresolved_for_url(&p, "https://example.com/a", &[]).unwrap(),
            Some(lost.job_id)
        );
        let (outcome, removed) = reconcile(Some(&p), lost.job_id).unwrap();
        assert_eq!(
            (outcome.as_str(), removed),
            ("interrupted", vec!["hang.mp4.part".to_string()])
        );
        let r = receipt(lost.job_id).unwrap().unwrap();
        assert_eq!(
            (r.outcome.as_str(), r.retryable, r.reconciled.as_deref()),
            ("failed", true, Some("no_complete_artifact"))
        );
        assert_eq!(
            unresolved_for_url(&p, "https://example.com/a", &[]).unwrap(),
            None
        );
        assert_eq!(
            reconcile(Some(&p), lost.job_id).unwrap().0,
            "already_terminal"
        );
        // Retry is admitted at once (a crash is no server answer) as attempt 1.
        let retry = prepare_retry(
            &p,
            lost.job_id,
            "after-crash",
            "download_retry",
            &serde_json::json!({"r":1}),
        )
        .unwrap();
        assert_eq!(retry.attempt, 1);
        let history = attempts(lost.job_id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["reconciled"], "no_complete_artifact");
        // Complete output and no partials: the effect is confirmed, not redone.
        let done = job(&dir, &p, "done");
        executing(done.job_id);
        old_file(&Path::new(&done.destination).join("a.mp4"), b"abc");
        assert_eq!(reconcile(Some(&p), done.job_id).unwrap().0, "completed");
        let r = receipt(done.job_id).unwrap().unwrap();
        assert_eq!(
            (r.outcome.as_str(), r.bytes, r.files.len()),
            ("success", Some(3), 1)
        );
        assert_eq!(
            crate::mcp::downloads::journal_item(&load(done.job_id).unwrap().unwrap())["status"]
                ["data"]["success"],
            true
        );
        // A folder written a moment ago may still have a live writer.
        let busy = job(&dir, &p, "busy");
        executing(busy.job_id);
        std::fs::write(Path::new(&busy.destination).join("b.mp4.part"), b"x").unwrap();
        assert!(reconcile(Some(&p), busy.job_id)
            .unwrap_err()
            .starts_with("RECONCILE_WRITER_ACTIVE:"));
        // Never started: the enqueue replay restores it, not reconciliation.
        let fresh = job(&dir, &p, "fresh");
        assert!(reconcile(Some(&p), fresh.job_id)
            .unwrap_err()
            .starts_with("RECONCILE_NOT_NEEDED"));
        let mut foreign = p.clone();
        foreign.id = "B".into();
        assert_eq!(
            reconcile(Some(&foreign), busy.job_id).unwrap_err(),
            "DOWNLOAD_NOT_FOUND"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cancel_after_pause_removes_the_partial_file() {
        let (dir, p) = store();
        let i = job(&dir, &p, "slow");
        executing(i.job_id);
        let part = Path::new(&i.destination).join("slow.mp4.part");
        std::fs::write(&part, vec![0u8; 1024]).unwrap();
        settled(i.job_id, 0, Some("paused")).unwrap();
        assert!(part.exists(), "a pause keeps its partial");
        assert_eq!(
            cancel_paused(i.job_id).unwrap(),
            vec!["slow.mp4.part".to_string()]
        );
        assert!(!part.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn resume_discards_what_the_paused_attempt_left() {
        let (dir, p) = store();
        let i = job(&dir, &p, "resume");
        executing(i.job_id);
        std::fs::write(Path::new(&i.destination).join("v.mp4.part"), vec![0u8; 700]).unwrap();
        settled(i.job_id, 0, Some("paused")).unwrap();
        let next =
            prepare_retry(&p, i.job_id, "r", "download_resume", &serde_json::json!({})).unwrap();
        assert_eq!(discard_for_restart(&next).unwrap(), (1, 700));
        assert_eq!(std::fs::read_dir(&i.destination).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn success_receipt_counts_every_produced_file() {
        // D-17: bytes named only the last carousel file.
        let (dir, p) = store();
        let i = job(&dir, &p, "carousel");
        executing(i.job_id);
        for (name, len) in [
            ("post_1.jpg", 3usize),
            ("post_2.jpg", 5),
            ("post_3.mp4", 7),
            ("......escaped.mp4", 11),
        ] {
            std::fs::write(Path::new(&i.destination).join(name), vec![1u8; len]).unwrap();
        }
        let last = Path::new(&i.destination)
            .join("post_3.mp4")
            .to_str()
            .unwrap()
            .to_owned();
        terminal_receipt(i.job_id, true, None, Some(last), Some(7), false).unwrap();
        let r = receipt(i.job_id).unwrap().unwrap();
        assert_eq!(r.bytes, Some(26));
        assert_eq!(r.files.len(), 4);
        assert!(r.files.iter().any(|f| f.name == "......escaped.mp4"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
