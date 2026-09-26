//! Restrained Higgsfield CLI adapter. Only host-side code may configure the
//! executable or grant a credit budget. No upload/path/shell surface is exposed.
//! A durable unknown intent precedes submission; it is never submitted again.
use super::db::{AssistDb, Migration};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "media_provider",
    version: 1,
    sql: r#"
CREATE TABLE media_provider_budgets(principal TEXT NOT NULL, mission TEXT NOT NULL REFERENCES missions_missions(id), ceiling INTEGER NOT NULL CHECK(ceiling>0), reserved INTEGER NOT NULL DEFAULT 0 CHECK(reserved>=0), PRIMARY KEY(principal,mission));
CREATE TABLE media_provider_quotes(id TEXT PRIMARY KEY, principal TEXT NOT NULL, mission TEXT NOT NULL REFERENCES missions_missions(id), fingerprint TEXT NOT NULL, credits INTEGER NOT NULL CHECK(credits>0), expires_ms INTEGER NOT NULL, used INTEGER NOT NULL DEFAULT 0);
CREATE TABLE media_provider_intents(id TEXT PRIMARY KEY, principal TEXT NOT NULL, mission TEXT NOT NULL REFERENCES missions_missions(id), intent_key TEXT NOT NULL, fingerprint TEXT NOT NULL, quote_id TEXT NOT NULL UNIQUE REFERENCES media_provider_quotes(id), reserved INTEGER NOT NULL, provider_id TEXT UNIQUE, state TEXT NOT NULL, created_ms INTEGER NOT NULL, UNIQUE(principal,mission,intent_key));
"#,
}];
const LIMIT: u64 = 1024 * 1024;
const QUOTE_MS: i64 = 5 * 60 * 1000;
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
fn db_error(_: rusqlite::Error) -> String {
    "PROVIDER_STORAGE_ERROR".into()
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn owner(
    conn: &rusqlite::Connection,
    principal: &str,
    mission: &str,
    active: bool,
) -> Result<(), String> {
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM missions_missions WHERE id=?1 AND principal=?2",
            params![mission, principal],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_error)?;
    match state {
        Some(s) if !active || matches!(s.as_str(), "queued" | "running" | "verifying") => Ok(()),
        _ => Err("PROVIDER_MISSION_DENIED".into()),
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model {
    #[serde(rename = "gpt_image_2_5")]
    GptImage2_5,
    #[serde(rename = "seedance_2_5")]
    Seedance2_5,
    #[serde(rename = "recraft_v4_1")]
    RecraftV4_1,
    #[serde(rename = "nano_banana_2")]
    NanoBanana2,
    SeedAudio,
}
impl Model {
    fn id(self) -> &'static str {
        match self {
            Self::GptImage2_5 => "gpt_image_2_5",
            Self::Seedance2_5 => "seedance_2_5",
            Self::RecraftV4_1 => "recraft_v4_1",
            Self::NanoBanana2 => "nano_banana_2",
            Self::SeedAudio => "seed_audio",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub model: Model,
    pub prompt: String,
    pub aspect_ratio: Option<String>,
    pub resolution: Option<String>,
    pub duration: Option<u8>,
    pub quality: Option<String>,
}
impl Generation {
    fn args(&self, verb: &str) -> Result<Vec<String>, String> {
        if self.prompt.trim().is_empty()
            || self.prompt.len() > 12000
            || self.prompt.trim_start().starts_with('@')
            || self.prompt.contains('\0')
        {
            return Err("PROVIDER_INVALID_PROMPT".into());
        }
        let mut a = vec![
            "generate".into(),
            verb.into(),
            self.model.id().into(),
            format!("--prompt={}", self.prompt),
            "--json".into(),
            "--no-color".into(),
        ];
        for (key, value, allowed) in [
            (
                "aspect_ratio",
                self.aspect_ratio.as_deref(),
                &[
                    "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "21:9", "4:5", "5:4",
                ][..],
            ),
            (
                "resolution",
                self.resolution.as_deref(),
                &["1k", "2k", "4k", "480p", "720p", "1080p"][..],
            ),
            (
                "quality",
                self.quality.as_deref(),
                &["low", "medium", "high", "xhigh", "max"][..],
            ),
        ] {
            if let Some(value) = value {
                if !allowed.contains(&value) {
                    return Err("PROVIDER_INVALID_PARAMETER".into());
                }
                a.push(format!("--{key}={value}"));
            }
        }
        if let Some(seconds) = self.duration {
            if !(2..=30).contains(&seconds) {
                return Err("PROVIDER_INVALID_DURATION".into());
            }
            a.push(format!("--duration={seconds}"));
        }
        Ok(a)
    }
    fn fingerprint(&self) -> Result<String, String> {
        self.args("cost")?;
        Ok(hex::encode(Sha256::digest(
            serde_json::to_vec(self).map_err(|_| "PROVIDER_INVALID_REQUEST")?,
        )))
    }
}
/// Credits, not dollars. Integers avoid cumulative floating point under-reservation.
#[derive(Debug, Serialize)]
pub struct Quote {
    pub id: String,
    pub microcredits: i64,
    pub unit: &'static str,
    pub expires_ms: i64,
}
#[derive(Debug, Serialize, Clone)]
pub struct Intent {
    pub id: String,
    pub provider_id: Option<String>,
    pub state: String,
    pub reserved_microcredits: i64,
}
#[derive(Debug, Serialize)]
pub struct Job {
    pub intent: Intent,
    pub status: String,
    pub artifact_count: usize,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub duration: Option<f64>,
    pub cost_actual_known: bool,
    pub license_known: bool,
}

/// Internal capability; deliberately neither Debug nor Serialize.
pub(crate) struct OpaqueLocator(String);
impl OpaqueLocator {
    pub(crate) fn as_url(&self) -> &str {
        &self.0
    }
}
pub struct Cli {
    path: PathBuf,
    /// Interpreter of a `#!/usr/bin/env` script, resolved at trusted
    /// configuration time (see `media_tools::resolve_interpreter`).
    interpreter: Option<PathBuf>,
    timeout: std::time::Duration,
}
impl Cli {
    /// The path comes from local trusted configuration, never tool arguments.
    pub fn installed(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || !path.is_file() {
            return Err("PROVIDER_CLI_UNAVAILABLE".into());
        }
        Ok(Self {
            path: path.to_path_buf(),
            interpreter: None,
            timeout: std::time::Duration::from_secs(45),
        })
    }
    pub fn installed_with(path: &Path, interpreter: Option<PathBuf>) -> Result<Self, String> {
        let mut cli = Self::installed(path)?;
        if let Some(i) = &interpreter {
            if !i.is_absolute() || !i.is_file() {
                return Err("PROVIDER_CLI_UNAVAILABLE".into());
            }
        }
        cli.interpreter = interpreter;
        Ok(cli)
    }
    async fn run(&self, args: Vec<String>) -> Result<Value, String> {
        use std::process::Stdio;
        let mut cmd = match &self.interpreter {
            Some(i) => {
                let mut c = tokio::process::Command::new(i);
                c.arg(&self.path);
                c
            }
            None => tokio::process::Command::new(&self.path),
        };
        // Only trusted configured directories, never the ambient PATH.
        let search_path = std::env::join_paths([
            self.interpreter
                .as_deref()
                .and_then(Path::parent)
                .unwrap_or(self.path.parent().ok_or("PROVIDER_CLI_UNAVAILABLE")?),
            self.path.parent().ok_or("PROVIDER_CLI_UNAVAILABLE")?,
            Path::new("/usr/bin"),
            Path::new("/bin"),
            Path::new("/usr/sbin"),
            Path::new("/sbin"),
        ])
        .map_err(|_| "PROVIDER_CLI_UNAVAILABLE")?;
        cmd.args(args)
            .env_clear()
            .env("PATH", search_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Credentials stay in the CLI's host-local credential store. Never
        // inherit bearer tokens, endpoint overrides or proxy environment.
        for name in ["HOME", "XDG_CONFIG_HOME", "USER"] {
            if let Some(v) = std::env::var_os(name) {
                cmd.env(name, v);
            }
        }
        #[cfg(unix)]
        {
            cmd.as_std_mut().process_group(0);
        }
        let mut child = cmd.spawn().map_err(|_| "PROVIDER_CLI_START_FAILED")?;
        struct Group(Option<u32>);
        impl Drop for Group {
            fn drop(&mut self) {
                #[cfg(unix)]
                if let Some(id) = self.0 {
                    unsafe {
                        libc::kill(-(id as i32), libc::SIGKILL);
                    }
                }
            }
        }
        let _group = Group(child.id());
        let stdout = child.stdout.take().ok_or("PROVIDER_CLI_START_FAILED")?;
        let stderr = child.stderr.take().ok_or("PROVIDER_CLI_START_FAILED")?;
        async fn bounded(reader: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>, String> {
            let mut bytes = Vec::new();
            reader
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "PROVIDER_IO_ERROR")?;
            if bytes.len() as u64 > LIMIT {
                Err("PROVIDER_OUTPUT_LIMIT".into())
            } else {
                Ok(bytes)
            }
        }
        let output = tokio::time::timeout(self.timeout, async {
            let (stdout, stderr, status) =
                tokio::try_join!(bounded(stdout), bounded(stderr), async {
                    child
                        .wait()
                        .await
                        .map_err(|_| "PROVIDER_IO_ERROR".to_string())
                })?;
            if !status.success() {
                // Operator log only (redacted, clipped); never returned to the
                // model or persisted with the intent.
                let head: String = String::from_utf8_lossy(&stderr).chars().take(400).collect();
                let out_head: String = String::from_utf8_lossy(&stdout).chars().take(200).collect();
                tracing::warn!(
                    "[media] provider CLI exited {:?}: {} | {}",
                    status.code(),
                    super::missions::diag::redact(&head),
                    super::missions::diag::redact(&out_head)
                );
                return Err("PROVIDER_CLI_FAILED".into());
            }
            serde_json::from_slice::<Value>(&stdout).map_err(|_| "PROVIDER_INVALID_RESPONSE".into())
        })
        .await
        .map_err(|_| "PROVIDER_TIMEOUT".to_string())?;
        // Neither raw stderr nor raw provider JSON is persisted or returned.
        output
    }
    pub async fn discover(&self) -> Result<Value, String> {
        let raw = self
            .run(vec![
                "model".into(),
                "list".into(),
                "--json".into(),
                "--no-color".into(),
            ])
            .await?;
        let rows = raw.as_array().ok_or("PROVIDER_INVALID_RESPONSE")?;
        Ok(Value::Array(
            rows.iter()
                .take(500)
                .filter_map(|v| {
                    let id = v.get("job_type")?.as_str()?;
                    if !identifier(id) {
                        return None;
                    }
                    let supported = [
                        Model::GptImage2_5,
                        Model::Seedance2_5,
                        Model::RecraftV4_1,
                        Model::NanoBanana2,
                        Model::SeedAudio,
                    ]
                    .iter()
                    .any(|m| m.id() == id);
                    Some(json!({"model":id,"supported":supported}))
                })
                .collect(),
        ))
    }
    pub async fn model_get(&self, model: Model) -> Result<Value, String> {
        let raw = self
            .run(vec![
                "model".into(),
                "get".into(),
                model.id().into(),
                "--json".into(),
                "--no-color".into(),
            ])
            .await?;
        if raw.get("job_type").and_then(Value::as_str) != Some(model.id()) {
            return Err("PROVIDER_MODEL_MISMATCH".into());
        }
        let names: Vec<&str> = raw
            .get("params")
            .and_then(Value::as_array)
            .ok_or("PROVIDER_INVALID_RESPONSE")?
            .iter()
            .filter_map(|v| v.get("name").and_then(Value::as_str))
            .filter(|v| {
                matches!(
                    *v,
                    "prompt" | "aspect_ratio" | "resolution" | "duration" | "quality"
                )
            })
            .collect();
        Ok(json!({"model":model.id(),"accepted_adapter_fields":names,"uploads_supported":false}))
    }
    pub async fn cost(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        generation: &Generation,
    ) -> Result<Quote, String> {
        db.tx(|tx| owner(tx, principal, mission, true))?;
        let fingerprint = generation.fingerprint()?;
        let raw = self.run(generation.args("cost")?).await?;
        let credits = raw
            .get("credits")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 1_000_000.0)
            .ok_or("PROVIDER_COST_UNKNOWN")?;
        let microcredits = (credits * 1_000_000.0).ceil() as i64;
        let quote = Quote {
            id: uuid::Uuid::new_v4().to_string(),
            microcredits,
            unit: "microcredits",
            expires_ms: now() + QUOTE_MS,
        };
        db.tx(|tx| {
            owner(tx, principal, mission, true)?;
            tx.execute(
                "INSERT INTO media_provider_quotes VALUES(?1,?2,?3,?4,?5,?6,0)",
                params![
                    quote.id,
                    principal,
                    mission,
                    fingerprint,
                    microcredits,
                    quote.expires_ms
                ],
            )
            .map_err(db_error)?;
            Ok(())
        })?;
        Ok(quote)
    }
    pub async fn submit(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        key: &str,
        quote: &str,
        generation: &Generation,
    ) -> Result<Intent, String> {
        self.submit_inner(db, principal, mission, key, quote, generation, None)
            .await
    }
    /// External execution context is supplied by the host, never generation arguments.
    pub async fn submit_authorized(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        key: &str,
        quote: &str,
        generation: &Generation,
        conversation: &str,
        bot: &str,
    ) -> Result<Intent, String> {
        self.submit_inner(
            db,
            principal,
            mission,
            key,
            quote,
            generation,
            Some((conversation, bot)),
        )
        .await
    }
    async fn submit_inner(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        key: &str,
        quote: &str,
        generation: &Generation,
        execution: Option<(&str, &str)>,
    ) -> Result<Intent, String> {
        let fingerprint = generation.fingerprint()?;
        let (intent, dispatch) =
            reserve_authorized(db, principal, mission, key, quote, &fingerprint, execution)?;
        if !dispatch {
            return Ok(intent);
        }
        // The committed unknown intent and reservation are already durable.
        // Any error, timeout, cancellation or crash stays unknown and reserved.
        let raw = match self.run(generation.args("create")?).await {
            Ok(raw) => raw,
            Err(_) => return Ok(intent),
        };
        let Some(provider_id) = submitted_id(&raw) else {
            return Ok(intent);
        };
        db.tx(|tx|{
            tx.execute("UPDATE media_provider_intents SET provider_id=?1,state='submitted' WHERE id=?2 AND principal=?3 AND mission=?4 AND state='unknown' AND provider_id IS NULL",params![provider_id,intent.id,principal,mission]).map_err(db_error)?;
            lookup(tx,principal,mission,&intent.id)
        })
    }
    async fn owned_job_raw(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        intent_id: &str,
    ) -> Result<(Intent, Value), String> {
        let intent = db.tx(|tx| {
            owner(tx, principal, mission, false)?;
            lookup(tx, principal, mission, intent_id)
        })?;
        let id = intent
            .provider_id
            .as_deref()
            .ok_or("PROVIDER_OUTCOME_UNKNOWN")?;
        if !identifier(id) {
            return Err("PROVIDER_INVALID_ID".into());
        }
        let raw = self
            .run(vec![
                "generate".into(),
                "get".into(),
                id.into(),
                "--json".into(),
                "--no-color".into(),
            ])
            .await?;
        if raw
            .get("id")
            .or_else(|| raw.get("job_id"))
            .and_then(Value::as_str)
            .is_some_and(|returned| returned != id)
        {
            return Err("PROVIDER_ID_MISMATCH".into());
        }
        Ok((intent, raw))
    }
    pub(crate) async fn owned_artifact_locator(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        intent_id: &str,
        index: usize,
    ) -> Result<OpaqueLocator, String> {
        if index >= 20 {
            return Err("PROVIDER_ARTIFACT_INDEX_INVALID".into());
        }
        let (_, raw) = self
            .owned_job_raw(db, principal, mission, intent_id)
            .await?;
        if !matches!(
            raw.get("status").and_then(Value::as_str),
            Some("completed" | "succeeded")
        ) {
            return Err("PROVIDER_ARTIFACT_NOT_READY".into());
        }
        // CLI 1.1.x returns `result_url` for single-output jobs; list-shaped
        // `results[].url` is kept for multi-output responses.
        let raw_url = raw
            .get("results")
            .and_then(Value::as_array)
            .and_then(|v| v.get(index))
            .and_then(|v| v.get("url"))
            .and_then(Value::as_str)
            .or_else(|| {
                (index == 0)
                    .then(|| raw.get("result_url").and_then(Value::as_str))
                    .flatten()
            })
            .ok_or("PROVIDER_ARTIFACT_UNAVAILABLE")?;
        if raw_url.len() > 8192 {
            return Err("PROVIDER_ARTIFACT_URL_INVALID".into());
        }
        let url = url::Url::parse(raw_url).map_err(|_| "PROVIDER_ARTIFACT_URL_INVALID")?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.host_str().is_none()
            || url.fragment().is_some()
        {
            return Err("PROVIDER_ARTIFACT_URL_INVALID".into());
        }
        Ok(OpaqueLocator(url.to_string()))
    }
    pub async fn get(
        &self,
        db: &AssistDb,
        principal: &str,
        mission: &str,
        intent_id: &str,
    ) -> Result<Job, String> {
        let (intent, raw) = self
            .owned_job_raw(db, principal, mission, intent_id)
            .await?;
        let status = raw
            .get("status")
            .and_then(Value::as_str)
            .filter(|s| {
                matches!(
                    *s,
                    "queued"
                        | "pending"
                        | "running"
                        | "processing"
                        | "completed"
                        | "succeeded"
                        | "failed"
                        | "cancelled"
                )
            })
            .unwrap_or("unknown")
            .to_string();
        // No URLs, prompts, metadata, errors, credentials or signed locators
        // cross this projection. Artifact retrieval requires a separate grant.
        Ok(Job {
            intent,
            status,
            artifact_count: raw
                .get("results")
                .and_then(Value::as_array)
                .map(|a| a.len().min(100))
                .unwrap_or(0),
            width: raw.get("width").and_then(Value::as_u64),
            height: raw.get("height").and_then(Value::as_u64),
            duration: raw
                .get("duration")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0),
            cost_actual_known: false,
            license_known: false,
        })
    }
}
fn submitted_id(raw: &Value) -> Option<String> {
    let value = if let Some(id) = raw.as_str() {
        Some(id)
    } else if let Some(a) = raw.as_array() {
        if a.len() == 1 {
            a[0].as_str()
                .or_else(|| a[0].get("id").and_then(Value::as_str))
        } else {
            None
        }
    } else {
        raw.get("id")
            .or_else(|| raw.get("job_id"))
            .and_then(Value::as_str)
            .or_else(|| {
                let a = raw.get("job_ids")?.as_array()?;
                if a.len() == 1 {
                    a[0].as_str()
                } else {
                    None
                }
            })
    }?;
    identifier(value).then(|| value.to_string())
}
fn lookup(
    conn: &rusqlite::Connection,
    principal: &str,
    mission: &str,
    id: &str,
) -> Result<Intent, String> {
    conn.query_row("SELECT id,provider_id,state,reserved FROM media_provider_intents WHERE id=?1 AND principal=?2 AND mission=?3",params![id,principal,mission],|r|Ok(Intent{id:r.get(0)?,provider_id:r.get(1)?,state:r.get(2)?,reserved_microcredits:r.get(3)?})).map_err(|_|"PROVIDER_INTENT_DENIED".into())
}
/// Trusted local authorization entry point. Never expose this as a model tool.
pub fn set_local_budget(
    db: &AssistDb,
    principal: &str,
    mission: &str,
    microcredits: i64,
) -> Result<(), String> {
    if !(1..=1_000_000_000_000).contains(&microcredits) {
        return Err("PROVIDER_INVALID_BUDGET".into());
    }
    db.tx(|tx|{owner(tx,principal,mission,false)?;
        let reserved:i64=tx.query_row("SELECT reserved FROM media_provider_budgets WHERE principal=?1 AND mission=?2",params![principal,mission],|r|r.get(0)).optional().map_err(db_error)?.unwrap_or(0);
        if microcredits<reserved{return Err("PROVIDER_BUDGET_BELOW_RESERVATIONS".into());}
        tx.execute("INSERT INTO media_provider_budgets(principal,mission,ceiling) VALUES(?1,?2,?3) ON CONFLICT(principal,mission) DO UPDATE SET ceiling=excluded.ceiling",params![principal,mission,microcredits]).map_err(db_error)?;Ok(())
    })
}
/// Trusted local revocation stops new reservations while retaining all unknown
/// or spent reservations. Already dispatched provider jobs are not canceled.
pub fn revoke_local_budget(db: &AssistDb, principal: &str, mission: &str) -> Result<(), String> {
    db.tx(|tx| {
        owner(tx, principal, mission, false)?;
        // ceiling is positive by schema; delete an unused grant, otherwise
        // reduce it to the amount already reserved. No credit is refunded.
        tx.execute(
            "DELETE FROM media_provider_budgets WHERE principal=?1 AND mission=?2 AND reserved=0",
            params![principal, mission],
        )
        .map_err(db_error)?;
        tx.execute(
            "UPDATE media_provider_budgets SET ceiling=reserved WHERE principal=?1 AND mission=?2",
            params![principal, mission],
        )
        .map_err(db_error)?;
        Ok(())
    })
}
#[cfg(test)]
fn reserve(
    db: &AssistDb,
    principal: &str,
    mission: &str,
    key: &str,
    quote: &str,
    fingerprint: &str,
) -> Result<(Intent, bool), String> {
    reserve_authorized(db, principal, mission, key, quote, fingerprint, None)
}
fn reserve_authorized(
    db: &AssistDb,
    principal: &str,
    mission: &str,
    key: &str,
    quote: &str,
    fingerprint: &str,
    execution: Option<(&str, &str)>,
) -> Result<(Intent, bool), String> {
    if !identifier(key) {
        return Err("PROVIDER_INVALID_IDEMPOTENCY_KEY".into());
    }
    db.tx(|tx|{
        owner(tx,principal,mission,true)?;
        if let Some((conversation,bot))=execution {
            let body:Option<String>=tx.query_row("SELECT g.body FROM external_executions e JOIN external_grants g ON g.id=e.grant_id JOIN missions_missions m ON m.id=e.mission WHERE e.conversation=?1 AND e.bot=?2 AND e.mission=?3 AND m.conversation_id=e.conversation AND m.principal=?4 AND g.principal=m.principal AND g.revoked=0 AND m.state IN ('running','verifying')",params![conversation,bot,mission,principal],|r|r.get(0)).optional().map_err(db_error)?;
            let grant:Value=serde_json::from_str(&body.ok_or("PROVIDER_AUTHORITY_REVOKED")?).map_err(|_|"PROVIDER_AUTHORITY_REVOKED")?;
            let allows=|field:&str,value:&str|grant.get(field).and_then(Value::as_array).is_some_and(|items|items.iter().any(|v|v.as_str()==Some(value)));
            if grant.get("principal").and_then(Value::as_str)!=Some(principal)||!allows("bots",bot)||!allows("tools","media_submit") {return Err("PROVIDER_AUTHORITY_REVOKED".into());}
        }
        let synchronous:i64=tx.query_row("PRAGMA synchronous",[],|r|r.get(0)).map_err(db_error)?;
        if synchronous<2{return Err("PROVIDER_DURABILITY_REQUIRED".into());}
        let previous:Option<(String,String)>=tx.query_row("SELECT id,fingerprint FROM media_provider_intents WHERE principal=?1 AND mission=?2 AND intent_key=?3",params![principal,mission,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_error)?;
        if let Some((id,old))=previous {if old!=fingerprint {return Err("PROVIDER_IDEMPOTENCY_CONFLICT".into());}return Ok((lookup(tx,principal,mission,&id)?,false));}
        let amount:Option<i64>=tx.query_row("SELECT credits FROM media_provider_quotes WHERE id=?1 AND principal=?2 AND mission=?3 AND fingerprint=?4 AND expires_ms>=?5 AND used=0",params![quote,principal,mission,fingerprint,now()],|r|r.get(0)).optional().map_err(db_error)?;
        let amount=amount.ok_or("PROVIDER_QUOTE_INVALID")?;
        let changed=tx.execute("UPDATE media_provider_budgets SET reserved=reserved+?1 WHERE principal=?2 AND mission=?3 AND ceiling-reserved>=?1",params![amount,principal,mission]).map_err(db_error)?;
        if changed!=1{return Err("PROVIDER_BUDGET_NOT_AUTHORIZED".into());}
        let id=uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO media_provider_intents VALUES(?1,?2,?3,?4,?5,?6,?7,NULL,'unknown',?8)",params![id,principal,mission,key,fingerprint,quote,amount,now()]).map_err(db_error)?;
        tx.execute("UPDATE media_provider_quotes SET used=1 WHERE id=?1",[quote]).map_err(db_error)?;
        Ok((lookup(tx,principal,mission,&id)?,true))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn fixture() -> (tempfile::TempDir, Arc<AssistDb>) {
        let dir = tempfile::tempdir().unwrap();
        let mut migrations=vec![Migration{module:"provider_test_mission",version:1,sql:"CREATE TABLE missions_missions(id TEXT PRIMARY KEY,principal TEXT,state TEXT); INSERT INTO missions_missions VALUES('mission','alice','running');"}];
        migrations.extend_from_slice(MIGRATIONS);
        let db = Arc::new(AssistDb::open_with(&dir.path().join("assist.db"), &migrations).unwrap());
        (dir, db)
    }
    fn generation() -> Generation {
        Generation {
            model: Model::GptImage2_5,
            prompt: "safe fixture".into(),
            aspect_ratio: Some("1:1".into()),
            resolution: None,
            duration: None,
            quality: None,
        }
    }
    fn cli(dir: &Path, create: &str) -> Cli {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fixture-cli");
        let count = dir.join("submissions");
        let script=format!("#!/bin/sh\ncase \"$1 $2\" in\n'model list') echo '[{{\"job_type\":\"gpt_image_2_5\",\"secret\":\"token\"}}]' ;;\n'model get') echo '{{\"job_type\":\"gpt_image_2_5\",\"params\":[{{\"name\":\"prompt\"}},{{\"name\":\"api_key\"}}]}}' ;;\n'generate cost') echo '{{\"credits\":0.25,\"token\":\"secret\"}}' ;;\n'generate create') echo x >> '{}'; {} ;;\n'generate get') echo '{{\"status\":\"completed\",\"results\":[{{\"url\":\"https://example.com/?token=secret\"}}],\"prompt\":\"secret\"}}' ;;\nesac\n",count.display(),create);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Cli::installed(&path).unwrap()
    }
    #[tokio::test]
    async fn fixture_submit_is_owned_idempotent_reserved_and_survives_reopen() {
        let (dir, db) = fixture();
        let cli = cli(dir.path(), "echo '{\"job_ids\":[\"provider-job-1\"]}'");
        let g = generation();
        assert!(cli
            .discover()
            .await
            .unwrap()
            .to_string()
            .find("secret")
            .is_none());
        let model = cli.model_get(g.model).await.unwrap().to_string();
        assert!(!model.contains("api_key"));
        let q = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        assert!(cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap_err()
            .contains("BUDGET"));
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let first = cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(first.state, "submitted");
        let again = cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(first.id, again.id);
        assert!(cli.get(&db, "bob", "mission", &first.id).await.is_err());
        let job =
            serde_json::to_string(&cli.get(&db, "alice", "mission", &first.id).await.unwrap())
                .unwrap();
        assert!(!job.contains("secret"));
        assert!(!job.contains("example.com"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        let path = db.path().unwrap().to_path_buf();
        drop(db);
        let reopened = AssistDb::open_with(&path, &[]).unwrap();
        assert_eq!(
            cli.get(&reopened, "alice", "mission", &first.id)
                .await
                .unwrap()
                .intent
                .provider_id,
            Some("provider-job-1".into())
        );
    }
    #[tokio::test]
    async fn ambiguous_submit_never_retries_or_releases_its_reservation() {
        let (dir, db) = fixture();
        let cli = cli(dir.path(), "echo 'not-json'");
        let g = generation();
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let q = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        let first = cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(first.state, "unknown");
        let replay = cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(first.id, replay.id);
        let q2 = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        assert!(cli
            .submit(&db, "alice", "mission", "key2", &q2.id, &g)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn concurrent_reservations_cannot_exceed_granted_credits() {
        let (dir, db) = fixture();
        let cli = cli(dir.path(), "exit 1");
        let g = generation();
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let a = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        let b = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        let fingerprint = g.fingerprint().unwrap();
        let threads: Vec<_> = [a.id, b.id]
            .into_iter()
            .enumerate()
            .map(|(n, q)| {
                let db = db.clone();
                let f = fingerprint.clone();
                std::thread::spawn(move || {
                    reserve(&db, "alice", "mission", &format!("key{n}"), &q, &f)
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .filter(|t| t.thread().id() != std::thread::current().id())
                .map(|t| t.join().unwrap())
                .filter(Result::is_ok)
                .count(),
            1
        );
    }
    #[tokio::test]
    async fn expired_mismatched_quotes_and_canceled_missions_are_denied() {
        let (dir, db) = fixture();
        let cli = cli(dir.path(), "exit 1");
        let mut g = generation();
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let q = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        g.prompt = "different".into();
        assert!(cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .is_err());
        g = generation();
        db.with(|c| c.execute("UPDATE media_provider_quotes SET expires_ms=0", []))
            .unwrap();
        assert!(cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .is_err());
        db.with(|c| c.execute("UPDATE missions_missions SET state='cancelled'", []))
            .unwrap();
        assert!(cli.cost(&db, "alice", "mission", &g).await.is_err());
        assert!(!dir.path().join("submissions").exists());
    }
    #[tokio::test]
    #[ignore = "host-installed authenticated CLI; readonly discovery/model/cost only"]
    async fn installed_cli_readonly_discovery_and_cost() {
        let (_dir, db) = fixture();
        let executable = std::env::var_os("OMNIGET_HIGGSFIELD_TEST_CLI")
            .expect("set a trusted installed CLI path");
        let cli = Cli::installed(Path::new(&executable)).unwrap();
        assert!(cli
            .discover()
            .await
            .unwrap()
            .as_array()
            .is_some_and(|a| !a.is_empty()));
        let g = generation();
        assert_eq!(
            cli.model_get(g.model).await.unwrap()["model"],
            "gpt_image_2_5"
        );
        let quote = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        assert!(quote.microcredits > 0);
        println!("readonly quote: {} microcredits", quote.microcredits);
    }
    #[tokio::test]
    async fn crash_after_reservation_stays_unknown_after_reopen() {
        let (dir, db) = fixture();
        let cli = cli(dir.path(), "echo '{\"id\":\"should-not-run\"}'");
        let g = generation();
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let q = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        let (reserved, dispatch) = reserve(
            &db,
            "alice",
            "mission",
            "key",
            &q.id,
            &g.fingerprint().unwrap(),
        )
        .unwrap();
        assert!(dispatch);
        let path = db.path().unwrap().to_path_buf();
        drop(db);
        let reopened = AssistDb::open_with(&path, &[]).unwrap();
        let replay = cli
            .submit(&reopened, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(replay.id, reserved.id);
        assert_eq!(replay.state, "unknown");
        assert!(!dir.path().join("submissions").exists());
    }
    #[tokio::test]
    async fn timeout_is_bounded_and_does_not_surface_stderr_secrets() {
        let (dir, db) = fixture();
        let mut cli = cli(
            dir.path(),
            "echo 'api_key=fixture-secret' >&2; /bin/sleep 5",
        );
        let g = generation();
        set_local_budget(&db, "alice", "mission", 250000).unwrap();
        let q = cli.cost(&db, "alice", "mission", &g).await.unwrap();
        cli.timeout = std::time::Duration::from_millis(50);
        let start = std::time::Instant::now();
        let result = cli
            .submit(&db, "alice", "mission", "key", &q.id, &g)
            .await
            .unwrap();
        assert_eq!(result.state, "unknown");
        assert!(start.elapsed() < std::time::Duration::from_secs(3));
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("fixture-secret"));
        revoke_local_budget(&db, "alice", "mission").unwrap();
    }
    #[tokio::test]
    async fn oversized_output_is_rejected_without_unbounded_capture() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("oversized-cli");
        std::fs::write(
            &executable,
            "#!/bin/sh\n/usr/bin/head -c 1050000 /dev/zero\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cli = Cli::installed(&executable).unwrap();
        assert_eq!(cli.discover().await.unwrap_err(), "PROVIDER_OUTPUT_LIMIT");
    }
    #[test]
    fn injection_and_upload_parameters_are_not_in_the_typed_surface() {
        let mut g = generation();
        g.prompt = "@/etc/passwd".into();
        assert!(g.args("create").is_err());
        g = generation();
        g.resolution = Some("--url=http://localhost".into());
        assert!(g.args("create").is_err());
        assert!(serde_json::from_value::<Generation>(
            json!({"model":"gpt_image_2_5","prompt":"x","image":"/etc/passwd"})
        )
        .is_err());
        assert_eq!(submitted_id(&json!({"job_ids":["a","b"]})), None);
        assert_eq!(submitted_id(&json!({"id":"--token"})), None);
    }
}
