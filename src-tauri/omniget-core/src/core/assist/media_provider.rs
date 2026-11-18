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

pub(crate) struct OpaqueLocator(String);
impl OpaqueLocator {
    pub(crate) fn as_url(&self) -> &str {
        &self.0
    }
}
pub struct Cli {
    path: PathBuf,
    interpreter: Option<PathBuf>,
    timeout: std::time::Duration,
}
impl Cli {
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
    pub(crate) async fn run(&self, args: Vec<String>) -> Result<Value, String> {
        use std::process::Stdio;
        let mut cmd = match &self.interpreter {
            Some(i) => {
                let mut c = tokio::process::Command::new(i);
                c.arg(&self.path);
                c
            }
            None => tokio::process::Command::new(&self.path),
        };
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
    #[allow(clippy::too_many_arguments)]
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
    #[allow(clippy::too_many_arguments)]
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
