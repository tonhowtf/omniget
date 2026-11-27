use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::mcp::policy::Principal;

pub const SCOPE_JOBS_READ: &str = "jobs:read";
pub const SCOPE_JOBS_OPERATE: &str = "jobs:operate";
pub const SCOPE_APPROVALS_RESPOND: &str = "approvals:respond";
pub const SCOPE_AUDIT_READ: &str = "audit:read";
pub const SCOPE_BUDGET_WRITE: &str = "budget:write";
pub const SCOPE_WORKFLOW_TRUST: &str = "workflow:trust";
pub const SCOPE_CANVAS_NODE: &str = "canvas:node";
pub const SCOPES: &[&str] = &[
    SCOPE_JOBS_READ,
    SCOPE_JOBS_OPERATE,
    SCOPE_APPROVALS_RESPOND,
    SCOPE_AUDIT_READ,
    SCOPE_BUDGET_WRITE,
    SCOPE_WORKFLOW_TRUST,
];

pub const JOB_TOKEN_ENV: &str = "OMNIGET_JOB_TOKEN";
pub const EXTENSION_PRINCIPAL: &str = "extension";
const LEGACY_SCOPES: &[&str] = &[SCOPE_JOBS_READ, SCOPE_JOBS_OPERATE];

const USER_PREFIX: &str = "ogt_";
const JOB_PREFIX: &str = "ojt_";
const CANVAS_PREFIX: &str = "oct_";
const CANVAS_TOKEN_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const JOB_TOKEN_SLACK_MS: u64 = 5 * 60 * 1000;
const JOB_TOKEN_DEFAULT_TTL_MS: u64 = 24 * 60 * 60 * 1000;
const TOUCH_EVERY_MS: u64 = 60 * 1000;
const JOB_TOKEN_KEEP_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    User,
    Cli,
    Job,
    Canvas,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Cli => "cli",
            Kind::Job => "job",
            Kind::Canvas => "canvas",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "cli" => Kind::Cli,
            "job" => Kind::Job,
            "canvas" => Kind::Canvas,
            _ => Kind::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TokenView {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub scopes: Vec<String>,
    pub job_id: Option<String>,
    pub expires_ms: Option<u64>,
    pub created_ms: u64,
    pub revoked_ms: Option<u64>,
    pub last_used_ms: Option<u64>,
    pub hint: String,
}

pub struct Minted {
    pub token: String,
    pub view: TokenView,
}

pub fn ensure_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS bridge_tokens (
            id TEXT PRIMARY KEY, kind TEXT NOT NULL DEFAULT 'user',
            name TEXT NOT NULL DEFAULT '', hash TEXT NOT NULL, scopes TEXT NOT NULL,
            job_id TEXT, expires_ms INTEGER, created_ms INTEGER NOT NULL,
            revoked_ms INTEGER, last_used_ms INTEGER);
         CREATE INDEX IF NOT EXISTS bridge_tokens_job ON bridge_tokens(job_id);",
    )
    .map_err(|e| e.to_string())
}

fn digest(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn prefix_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Job => JOB_PREFIX,
        Kind::Canvas => CANVAS_PREFIX,
        _ => USER_PREFIX,
    }
}

fn hint_of(kind: Kind, id: &str) -> String {
    format!("{}{id}_…", prefix_of(kind))
}

fn parse(token: &str) -> Option<(&str, &str)> {
    let rest = token
        .strip_prefix(USER_PREFIX)
        .or_else(|| token.strip_prefix(JOB_PREFIX))
        .or_else(|| token.strip_prefix(CANVAS_PREFIX))?;
    let (id, secret) = rest.split_once('_')?;
    let hexish = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    (id.len() == 12 && secret.len() == 64 && hexish(id) && hexish(secret)).then_some((id, secret))
}

pub fn mint(
    db: &Connection,
    kind: Kind,
    name: &str,
    scopes: &[String],
    job_id: Option<&str>,
    expires_ms: Option<u64>,
    now: u64,
) -> Result<Minted, String> {
    let name = name.trim();
    if name.chars().count() > 80 {
        return Err("ERR_TOKEN_NAME: at most 80 characters".into());
    }
    let canvas_ok = kind == Kind::Canvas && scopes.len() == 1 && scopes[0] == SCOPE_CANVAS_NODE;
    if kind == Kind::Canvas && !canvas_ok {
        return Err(format!(
            "ERR_TOKEN_SCOPE: a canvas token has only {SCOPE_CANVAS_NODE}"
        ));
    }
    if !canvas_ok && (scopes.is_empty() || scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))) {
        return Err(format!(
            "ERR_TOKEN_SCOPE: choose from {}",
            SCOPES.join(", ")
        ));
    }
    let mut scopes: Vec<String> = scopes.to_vec();
    scopes.sort();
    scopes.dedup();
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token = format!("{}{id}_{secret}", prefix_of(kind));
    db.execute(
        "INSERT INTO bridge_tokens(id,kind,name,hash,scopes,job_id,expires_ms,created_ms)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            id,
            kind.as_str(),
            name,
            digest(&secret),
            serde_json::to_string(&scopes).map_err(|e| e.to_string())?,
            job_id,
            expires_ms.map(|v| v as i64),
            now as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(Minted {
        view: TokenView {
            hint: hint_of(kind, &id),
            id,
            kind,
            name: name.to_string(),
            scopes,
            job_id: job_id.map(String::from),
            expires_ms,
            created_ms: now,
            revoked_ms: None,
            last_used_ms: None,
        },
        token,
    })
}

const COLS: &str = "id,kind,name,scopes,job_id,expires_ms,created_ms,revoked_ms,last_used_ms";

fn row_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<TokenView> {
    let id: String = r.get(0)?;
    let kind = Kind::parse(&r.get::<_, String>(1)?);
    Ok(TokenView {
        hint: hint_of(kind, &id),
        id,
        kind,
        name: r.get(2)?,
        scopes: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
        job_id: r.get(4)?,
        expires_ms: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        created_ms: r.get::<_, i64>(6)? as u64,
        revoked_ms: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        last_used_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Valid(TokenView),
    Ended,
    Unknown,
}

pub fn lookup(db: &Connection, token: &str, now: u64) -> Lookup {
    let Some((id, secret)) = parse(token) else {
        return Lookup::Unknown;
    };
    let row: Option<(String, TokenView)> = db
        .query_row(
            &format!("SELECT {COLS},hash FROM bridge_tokens WHERE id=?1"),
            [id],
            |r| Ok((r.get(9)?, row_view(r)?)),
        )
        .optional()
        .ok()
        .flatten();
    let Some((hash, mut view)) = row else {
        return Lookup::Unknown;
    };
    if !constant_time_eq::constant_time_eq(hash.as_bytes(), digest(secret).as_bytes()) {
        return Lookup::Unknown;
    }
    if !token.starts_with(prefix_of(view.kind)) {
        return Lookup::Unknown;
    }
    if view.revoked_ms.is_some() || view.expires_ms.is_some_and(|e| now >= e) {
        return Lookup::Ended;
    }
    if view
        .last_used_ms
        .is_none_or(|t| now.saturating_sub(t) >= TOUCH_EVERY_MS)
    {
        if view.kind == Kind::Canvas {
            let until = now + CANVAS_TOKEN_TTL_MS;
            let _ = db.execute(
                "UPDATE bridge_tokens SET last_used_ms=?2, expires_ms=?3 WHERE id=?1",
                params![view.id, now as i64, until as i64],
            );
            view.expires_ms = Some(until);
        } else {
            let _ = db.execute(
                "UPDATE bridge_tokens SET last_used_ms=?2 WHERE id=?1",
                params![view.id, now as i64],
            );
        }
        view.last_used_ms = Some(now);
    }
    Lookup::Valid(view)
}

pub fn revoke(db: &Connection, id: &str, now: u64) -> Result<bool, String> {
    db.execute(
        "UPDATE bridge_tokens SET revoked_ms=?2 WHERE id=?1 AND revoked_ms IS NULL",
        params![id, now as i64],
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

pub fn revoke_job(db: &Connection, job_id: &str, now: u64) -> Result<usize, String> {
    let n = db
        .execute(
            "UPDATE bridge_tokens SET revoked_ms=?2 WHERE job_id=?1 AND revoked_ms IS NULL",
            params![job_id, now as i64],
        )
        .map_err(|e| e.to_string())?;
    let _ = db.execute(
        "DELETE FROM bridge_tokens WHERE kind='job' AND revoked_ms IS NOT NULL AND revoked_ms < ?1",
        params![now.saturating_sub(JOB_TOKEN_KEEP_MS) as i64],
    );
    Ok(n)
}

pub fn revoke_orphan_job_tokens(db: &Connection, now: u64) -> Result<usize, String> {
    db.execute(
        "UPDATE bridge_tokens SET revoked_ms=?1 WHERE kind='job' AND revoked_ms IS NULL",
        params![now as i64],
    )
    .map_err(|e| e.to_string())
}

pub fn mint_canvas(db: &Connection, node_id: &str, name: &str, now: u64) -> Result<Minted, String> {
    revoke_canvas_node(db, node_id, now)?;
    let label: String = format!("canvas {name}").chars().take(80).collect();
    mint(
        db,
        Kind::Canvas,
        &label,
        &[SCOPE_CANVAS_NODE.to_string()],
        Some(node_id),
        Some(now + CANVAS_TOKEN_TTL_MS),
        now,
    )
}

pub fn live_canvas_token(db: &Connection, node_id: &str, token: &str, now: u64) -> Option<String> {
    match lookup(db, token.trim(), now) {
        Lookup::Valid(v) if v.kind == Kind::Canvas && v.job_id.as_deref() == Some(node_id) => {
            Some(v.id)
        }
        _ => None,
    }
}

pub fn revoke_canvas_node(db: &Connection, node_id: &str, now: u64) -> Result<usize, String> {
    let n = db
        .execute(
            "UPDATE bridge_tokens SET revoked_ms=?2 WHERE kind='canvas' AND job_id=?1 AND revoked_ms IS NULL",
            params![node_id, now as i64],
        )
        .map_err(|e| e.to_string())?;
    let _ = db.execute(
        "DELETE FROM bridge_tokens WHERE kind='canvas' AND revoked_ms IS NOT NULL AND revoked_ms < ?1",
        params![now.saturating_sub(JOB_TOKEN_KEEP_MS) as i64],
    );
    Ok(n)
}

pub fn list(db: &Connection, now: u64) -> Vec<TokenView> {
    let Ok(mut stmt) = db.prepare(&format!(
        "SELECT {COLS} FROM bridge_tokens WHERE kind NOT IN ('job','canvas') AND revoked_ms IS NULL
         AND (expires_ms IS NULL OR expires_ms > ?1) ORDER BY created_ms DESC"
    )) else {
        return Vec::new();
    };
    stmt.query_map([now as i64], row_view)
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

pub fn job_token_expiry(deadline_ms: Option<u64>, now: u64) -> u64 {
    deadline_ms
        .map(|d| d.max(now) + JOB_TOKEN_SLACK_MS)
        .unwrap_or(now + JOB_TOKEN_DEFAULT_TTL_MS)
}

pub fn job_scopes() -> Vec<String> {
    vec![SCOPE_JOBS_READ.to_string()]
}

pub const CLI_TOKEN_FILE: &str = "cli-token";
const CLI_TOKEN_NAME: &str = "omniget CLI (this computer)";

pub fn ensure_cli_token(db: &Connection, dir: &std::path::Path, now: u64) -> Result<(), String> {
    let path = dir.join(CLI_TOKEN_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Lookup::Valid(v) = lookup(db, text.trim(), now) {
            if v.kind == Kind::Cli && SCOPES.iter().all(|s| v.scopes.iter().any(|x| x == s)) {
                return Ok(());
            }
        }
    }
    let _ = db.execute(
        "UPDATE bridge_tokens SET revoked_ms=?1 WHERE kind='cli' AND revoked_ms IS NULL",
        params![now as i64],
    );
    let scopes: Vec<String> = SCOPES.iter().map(|s| s.to_string()).collect();
    let minted = mint(db, Kind::Cli, CLI_TOKEN_NAME, &scopes, None, None, now)?;
    write_private(&path, minted.token.as_bytes()).map_err(|e| e.to_string())
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Need {
    pub scope: &'static str,
    pub job: Option<String>,
    pub legacy_ok: bool,
}

pub fn route_need(method: &Method, path: &str) -> Option<Need> {
    let need = |scope, job: Option<&str>| {
        Some(Need {
            scope,
            job: job.map(String::from),
            legacy_ok: true,
        })
    };
    let segs: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    let get = method == Method::GET;
    let post = method == Method::POST;
    let delete = method == Method::DELETE;
    match segs.as_slice() {
        ["v1", "jobs"]
        | ["v1", "state"]
        | ["v1", "loops"]
        | ["v1", "agents"]
        | ["v1", "playbooks"]
            if get =>
        {
            need(SCOPE_JOBS_READ, None)
        }
        ["v1", "jobs", id] if get => need(SCOPE_JOBS_READ, Some(id)),
        ["v1", "jobs", id, "explain" | "diff" | "proof" | "report"] if get => {
            need(SCOPE_JOBS_READ, Some(id))
        }
        ["v1", "jobs", id, "hunks" | "notes" | "integrate"] if get => {
            need(SCOPE_JOBS_READ, Some(id))
        }
        ["v1", "jobs", id, "hunks", "discard"]
        | ["v1", "jobs", id, "notes"]
        | ["v1", "jobs", id, "notes", "send"]
        | ["v1", "jobs", id, "integrate" | "discard" | "walkthrough"]
            if post =>
        {
            need(SCOPE_JOBS_OPERATE, Some(id))
        }
        ["v1", "jobs", id, "notes", _] if delete => need(SCOPE_JOBS_OPERATE, Some(id)),
        ["v1", "jobs", id, "cancel"] if post => need(SCOPE_JOBS_OPERATE, Some(id)),
        ["v1", "jobs", id, "wait"] if get => need(SCOPE_JOBS_READ, Some(id)),
        ["v1", "commands"] if post => Some(Need {
            scope: SCOPE_JOBS_READ,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "agentkit", _]
        | ["v1", "agent", "run" | "run-tool" | "loop" | "loop-catalog"]
        | ["v1", "hooks", _]
            if post =>
        {
            Some(Need {
                scope: SCOPE_JOBS_OPERATE,
                job: None,
                legacy_ok: false,
            })
        }
        ["v1", "approvals"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "approvals", _] if post => Some(Need {
            scope: SCOPE_APPROVALS_RESPOND,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "audit"] if get => Some(Need {
            scope: SCOPE_AUDIT_READ,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "budget"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "budget", "policies"] | ["v1", "budget", "incidents", _] if post => Some(Need {
            scope: SCOPE_BUDGET_WRITE,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "workflow"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "workflow", "trust"] if post => Some(Need {
            scope: SCOPE_WORKFLOW_TRUST,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "company", "rpc"] if post => Some(Need {
            scope: SCOPE_JOBS_READ,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "agents", "cli"] if post => Some(Need {
            scope: SCOPE_JOBS_OPERATE,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "canvas", "me" | "peers" | "screen"] if get => Some(Need {
            scope: SCOPE_CANVAS_NODE,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "canvas", "asks", _, "wait"] if get => Some(Need {
            scope: SCOPE_CANVAS_NODE,
            job: None,
            legacy_ok: false,
        }),
        ["v1", "canvas", "asks" | "reply" | "hook" | "rpc"] if post => Some(Need {
            scope: SCOPE_CANVAS_NODE,
            job: None,
            legacy_ok: false,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Denied {
    Unauthorized(&'static str),
    Forbidden(String),
}

impl Denied {
    pub fn into_response(self) -> Response {
        let (status, msg) = match self {
            Denied::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m.to_string()),
            Denied::Forbidden(m) => (StatusCode::FORBIDDEN, m),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

pub struct AuthCtx<'a> {
    pub extension: &'a str,
    pub legacy_allowed: bool,
    pub lookup: &'a dyn Fn(&str) -> Lookup,
}

fn bearer_of(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
}

pub fn check_scoped(
    headers: &HeaderMap,
    need: &Need,
    ctx: &AuthCtx<'_>,
) -> Result<Principal, Denied> {
    let Some(token) = bearer_of(headers).filter(|t| !t.is_empty()) else {
        return Err(Denied::Unauthorized("bad or missing bearer token"));
    };
    match (ctx.lookup)(token) {
        Lookup::Valid(v) => {
            if !v.scopes.iter().any(|s| s == need.scope) {
                return Err(Denied::Forbidden(format!(
                    "this token lacks the {} scope",
                    need.scope
                )));
            }
            if v.kind == Kind::Canvas {
                let node = v.job_id.clone().unwrap_or_default();
                return Ok(Principal {
                    id: format!("canvas:{node}"),
                    name: v.name,
                    scopes: v.scopes,
                });
            }
            if let Some(own) = &v.job_id {
                if need.job.as_deref() != Some(own.as_str()) {
                    return Err(Denied::Forbidden(
                        "a job token only reaches its own job".into(),
                    ));
                }
            }
            return Ok(Principal {
                id: match &v.job_id {
                    Some(j) => format!("job:{j}"),
                    None => format!("token:{}", v.id),
                },
                name: v.name,
                scopes: v.scopes,
            });
        }
        Lookup::Ended => return Err(Denied::Unauthorized("this token was revoked or expired")),
        Lookup::Unknown => {}
    }
    if !ctx.extension.is_empty()
        && constant_time_eq::constant_time_eq(token.as_bytes(), ctx.extension.as_bytes())
    {
        if !need.legacy_ok || !ctx.legacy_allowed || !LEGACY_SCOPES.contains(&need.scope) {
            return Err(Denied::Forbidden(
                "the browser extension key cannot use agent routes: create a scoped token in OmniGet → LLM → Jobs → Access tokens".into(),
            ));
        }
        return Ok(Principal {
            id: EXTENSION_PRINCIPAL.into(),
            name: "browser extension (deprecated on agent routes)".into(),
            scopes: LEGACY_SCOPES.iter().map(|s| s.to_string()).collect(),
        });
    }
    Err(Denied::Unauthorized("bad or missing bearer token"))
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct LegacyUse {
    pub at_ms: u64,
    pub route: String,
}

#[derive(Default)]
pub struct LegacyLog {
    last: Mutex<Option<LegacyUse>>,
    count: AtomicU64,
    logged_ms: AtomicU64,
}

impl LegacyLog {
    pub fn record(&self, route: &str, now: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(LegacyUse {
            at_ms: now,
            route: route.to_string(),
        });
        let before = self.logged_ms.load(Ordering::Relaxed);
        if now.saturating_sub(before) >= 60_000
            && self
                .logged_ms
                .compare_exchange(before, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            tracing::warn!(
                "[bridge] the browser extension key was used on {route}: agent routes stop \
                 accepting it in the next version; use a scoped token (LLM → Jobs → Access tokens)"
            );
        }
    }
    pub fn last(&self) -> Option<LegacyUse> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub fn legacy_flag() -> Arc<AtomicBool> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    FLAG.get_or_init(|| Arc::new(AtomicBool::new(true))).clone()
}

pub fn legacy_log() -> Arc<LegacyLog> {
    static LOG: OnceLock<Arc<LegacyLog>> = OnceLock::new();
    LOG.get_or_init(|| Arc::new(LegacyLog::default())).clone()
}

pub type LookupFn = Arc<dyn Fn(&str) -> Lookup + Send + Sync>;

#[derive(Clone)]
pub struct Guard {
    pub extension: Arc<String>,
    pub legacy_allowed: Arc<AtomicBool>,
    pub legacy_log: Arc<LegacyLog>,
    pub lookup: LookupFn,
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

const DEPRECATION_WARNING: &str = "299 omniget \"the browser extension key stops working on agent routes in the next version; use a scoped token (OmniGet: LLM, Jobs, Access tokens)\"";

pub async fn guard(State(g): State<Guard>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let Some(need) = route_need(req.method(), &path) else {
        return Denied::Forbidden(format!("no scope is defined for {path}")).into_response();
    };
    let checked = {
        let live = crate::local_bridge::live_token(&g.extension);
        let ctx = AuthCtx {
            extension: live.as_str(),
            legacy_allowed: g.legacy_allowed.load(Ordering::Relaxed),
            lookup: g.lookup.as_ref(),
        };
        check_scoped(req.headers(), &need, &ctx)
    };
    let principal = match checked {
        Ok(p) => p,
        Err(d) => return d.into_response(),
    };
    let legacy = principal.id == EXTENSION_PRINCIPAL;
    if legacy {
        g.legacy_log.record(&path, (g.now)());
    }
    crate::llm_audit::on_route(&principal, req.method(), &path, (g.now)());
    req.extensions_mut().insert(principal);
    let mut resp = next.run(req).await;
    if legacy {
        resp.headers_mut()
            .insert("deprecation", HeaderValue::from_static("true"));
        resp.headers_mut()
            .insert("warning", HeaderValue::from_static(DEPRECATION_WARNING));
    }
    resp
}

pub struct JobTokenLease {
    conversation: String,
    revoke: Option<Box<dyn FnOnce() + Send>>,
}

impl JobTokenLease {
    pub fn start(
        conversation: &str,
        mint: impl FnOnce() -> Result<Minted, String>,
        revoke: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        let minted = mint()?;
        omniget_core::core::llm::cli_runtime::set_conversation_env(
            conversation,
            BTreeMap::from([(JOB_TOKEN_ENV.to_string(), minted.token)]),
        );
        Ok(Self {
            conversation: conversation.to_string(),
            revoke: Some(Box::new(revoke)),
        })
    }

    pub fn start_with(
        conversation: &str,
        identity: BTreeMap<String, String>,
        mint: impl FnOnce() -> Result<Minted, String>,
        revoke: impl FnOnce() + Send + 'static,
    ) -> (Self, Option<String>) {
        let mut env = identity;
        let (revoke, error): (Option<Box<dyn FnOnce() + Send>>, _) = match mint() {
            Ok(minted) => {
                env.insert(JOB_TOKEN_ENV.to_string(), minted.token);
                (Some(Box::new(revoke)), None)
            }
            Err(e) => (None, Some(e)),
        };
        omniget_core::core::llm::cli_runtime::set_conversation_env(conversation, env);
        (
            Self {
                conversation: conversation.to_string(),
                revoke,
            },
            error,
        )
    }
}

pub fn job_identity_env(
    job_id: &str,
    agent_id: &str,
    bridge_url: Option<&str>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([
        ("OMNIGET_ENV".to_string(), "1".to_string()),
        ("OMNIGET_JOB_ID".to_string(), job_id.to_string()),
        ("OMNIGET_AGENT_ID".to_string(), agent_id.to_string()),
    ]);
    if let Some(url) = bridge_url.filter(|u| !u.is_empty()) {
        env.insert("OMNIGET_BRIDGE_URL".to_string(), url.to_string());
    }
    env
}

impl Drop for JobTokenLease {
    fn drop(&mut self) {
        omniget_core::core::llm::cli_runtime::clear_conversation_env(&self.conversation);
        if let Some(revoke) = self.revoke.take() {
            revoke();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db).unwrap();
        db
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn headers(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        h
    }

    #[test]
    fn a_token_is_stored_as_a_digest_and_found_by_its_id() {
        let db = db();
        let m = mint(&db, Kind::User, "ci", &s(&[SCOPE_JOBS_READ]), None, None, 1).unwrap();
        assert!(m.token.starts_with("ogt_"));
        let stored: String = db
            .query_row("SELECT hash FROM bridge_tokens", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains(&m.token) && !m.token.contains(&stored));
        assert!(matches!(lookup(&db, &m.token, 2), Lookup::Valid(v) if v.id == m.view.id));
        let mut bad = m.token.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == '0' { '1' } else { '0' });
        assert_eq!(lookup(&db, &bad, 2), Lookup::Unknown);
        assert_eq!(
            lookup(&db, &m.token.replacen("ogt_", "ojt_", 1), 2),
            Lookup::Unknown
        );
    }

    #[test]
    fn unknown_scopes_and_empty_lists_are_refused() {
        let db = db();
        assert!(mint(&db, Kind::User, "x", &s(&["jobs:admin"]), None, None, 1).is_err());
        assert!(mint(&db, Kind::User, "x", &[], None, None, 1).is_err());
    }

    #[test]
    fn revoked_and_expired_tokens_end_with_401() {
        let db = db();
        let a = mint(
            &db,
            Kind::User,
            "a",
            &s(&[SCOPE_JOBS_READ]),
            None,
            Some(100),
            1,
        )
        .unwrap();
        assert!(matches!(lookup(&db, &a.token, 99), Lookup::Valid(_)));
        assert_eq!(lookup(&db, &a.token, 100), Lookup::Ended);
        let b = mint(&db, Kind::User, "b", &s(&[SCOPE_JOBS_READ]), None, None, 1).unwrap();
        assert!(revoke(&db, &b.view.id, 5).unwrap());
        assert!(!revoke(&db, &b.view.id, 6).unwrap());
        assert_eq!(lookup(&db, &b.token, 7), Lookup::Ended);
        let need = route_need(&Method::GET, "/v1/jobs").unwrap();
        let look = |t: &str| lookup(&db, t, 7);
        let ctx = AuthCtx {
            extension: "ext",
            legacy_allowed: true,
            lookup: &look,
        };
        assert!(matches!(
            check_scoped(&headers(&b.token), &need, &ctx),
            Err(Denied::Unauthorized("this token was revoked or expired"))
        ));
        assert!(list(&db, 200).is_empty());
    }

    #[test]
    fn budgets_are_written_only_with_their_own_scope() {
        let read = route_need(&Method::GET, "/v1/budget").unwrap();
        assert_eq!(read.scope, SCOPE_JOBS_READ);
        for path in ["/v1/budget/policies", "/v1/budget/incidents/bi_1"] {
            let need = route_need(&Method::POST, path).unwrap();
            assert_eq!(need.scope, SCOPE_BUDGET_WRITE, "{path}");
            assert!(!need.legacy_ok, "{path}: never the extension key");
        }
        assert!(!job_scopes().contains(&SCOPE_BUDGET_WRITE.to_string()));
        assert!(!LEGACY_SCOPES.contains(&SCOPE_BUDGET_WRITE));
        assert!(
            SCOPES.contains(&SCOPE_BUDGET_WRITE),
            "the CLI token gets it"
        );
    }

    #[test]
    fn a_workflow_is_trusted_only_with_its_own_scope() {
        assert_eq!(
            route_need(&Method::GET, "/v1/workflow").unwrap().scope,
            SCOPE_JOBS_READ
        );
        let report = route_need(&Method::GET, "/v1/jobs/j1/report").unwrap();
        assert_eq!(
            (report.scope, report.job.as_deref()),
            (SCOPE_JOBS_READ, Some("j1"))
        );
        let trust = route_need(&Method::POST, "/v1/workflow/trust").unwrap();
        assert_eq!(trust.scope, SCOPE_WORKFLOW_TRUST);
        assert!(!trust.legacy_ok);
        assert!(!job_scopes().contains(&SCOPE_WORKFLOW_TRUST.to_string()));
        assert!(!LEGACY_SCOPES.contains(&SCOPE_WORKFLOW_TRUST));
        assert!(
            SCOPES.contains(&SCOPE_WORKFLOW_TRUST),
            "the CLI token gets it"
        );
    }

    #[test]
    fn every_agent_route_has_a_scope() {
        let src = include_str!("local_bridge/jobs.rs");
        let served = src.split("#[cfg(test)]").next().unwrap();
        let mut paths = Vec::new();
        let mut rest = served;
        while let Some(i) = rest.find(".route(") {
            rest = &rest[i + 7..];
            let q = rest.find('"').unwrap();
            let end = rest[q + 1..].find('"').unwrap();
            paths.push(rest[q + 1..q + 1 + end].to_string());
        }
        assert_eq!(paths.len(), 35, "{paths:?}");
        for p in &paths {
            let concrete = p.replace("{id}", "j1").replace("{note}", "n1");
            let ok = route_need(&Method::GET, &concrete).is_some()
                || route_need(&Method::POST, &concrete).is_some()
                || route_need(&Method::DELETE, &concrete).is_some();
            assert!(ok, "{p} has no scope in bridge_tokens::route_need");
        }
        assert!(!src.contains("check_bearer("));
        assert!(src.contains("route_layer("));
        let company = include_str!("local_bridge/company.rs");
        let company = company.split("#[cfg(test)]").next().unwrap();
        assert!(!company.contains("check_bearer("));
        assert_eq!(company.matches(".route(").count(), 1);
        let need = route_need(&Method::POST, "/v1/company/rpc").expect("company route");
        assert_eq!(need.scope, SCOPE_JOBS_READ);
        assert!(!need.legacy_ok, "the extension key never reached companies");
    }

    #[cfg_attr(
        windows,
        ignore = "reads source files as text and trips on Windows checkouts"
    )]
    #[test]
    fn the_guard_layer_comes_after_every_route() {
        for (name, src) in [
            ("local_bridge/jobs.rs", include_str!("local_bridge/jobs.rs")),
            (
                "local_bridge/agentkit.rs",
                include_str!("local_bridge/agentkit.rs"),
            ),
            (
                "local_bridge/company.rs",
                include_str!("local_bridge/company.rs"),
            ),
            (
                "local_bridge/canvas.rs",
                include_str!("local_bridge/canvas.rs"),
            ),
        ] {
            let served = src.split("#[cfg(test)]").next().unwrap();
            let start = served
                .find("pub fn router<")
                .unwrap_or_else(|| panic!("{name}: no router()"));
            let body = &served[start..];
            let end = body.find("\n}\n").unwrap_or(body.len());
            let body = &body[..end];
            let layer = body
                .rfind(".route_layer(")
                .unwrap_or_else(|| panic!("{name}: router() has no route_layer"));
            assert!(
                body[layer..].contains("bridge_tokens::guard"),
                "{name}: the layer is not the bridge guard"
            );
            let last_route = body.rfind(".route(").unwrap_or(0);
            let last_merge = body.rfind(".merge(").unwrap_or(0);
            assert!(
                layer > last_route && layer > last_merge,
                "{name}: a route of router() comes after its route_layer"
            );
            assert_eq!(
                served.matches(".route_layer(").count(),
                1,
                "{name}: the guard layer must be router()'s only"
            );
        }
    }

    #[test]
    fn the_routes_ask_for_the_right_scope() {
        let g = |p: &str| route_need(&Method::GET, p).unwrap();
        let p = |p: &str| route_need(&Method::POST, p).unwrap();
        assert_eq!(g("/v1/jobs").scope, SCOPE_JOBS_READ);
        assert_eq!(g("/v1/jobs/j1/diff").job.as_deref(), Some("j1"));
        assert_eq!(p("/v1/jobs/j1/cancel").scope, SCOPE_JOBS_OPERATE);
        assert_eq!(p("/v1/agent/run").scope, SCOPE_JOBS_OPERATE);
        assert_eq!(p("/v1/hooks/t1").scope, SCOPE_JOBS_OPERATE);
        let cli = p("/v1/agents/cli");
        assert_eq!(cli.scope, SCOPE_JOBS_OPERATE);
        assert!(!cli.legacy_ok);
        assert_eq!(g("/v1/approvals").scope, SCOPE_JOBS_READ);
        let answer = p("/v1/approvals/apr_1");
        assert_eq!(answer.scope, SCOPE_APPROVALS_RESPOND);
        assert!(!answer.legacy_ok);
        let audit = g("/v1/audit");
        assert_eq!(audit.scope, SCOPE_AUDIT_READ);
        assert!(!audit.legacy_ok);
        assert!(!job_scopes().contains(&SCOPE_AUDIT_READ.to_string()));
        assert!(route_need(&Method::POST, "/v1/approvals").is_none());
        let cmd = p("/v1/commands");
        assert!(!cmd.legacy_ok && cmd.job.is_none());
        let w = g("/v1/jobs/j1/wait");
        assert_eq!((w.scope, w.job.as_deref()), (SCOPE_JOBS_READ, Some("j1")));
        assert!(route_need(&Method::GET, "/v1/commands").is_none());
        assert_eq!(p("/v1/agentkit/apply").scope, SCOPE_JOBS_OPERATE);
        for route in [
            "/v1/agentkit/apply",
            "/v1/agent/run",
            "/v1/agent/run-tool",
            "/v1/agent/loop",
            "/v1/hooks/t1",
        ] {
            assert!(!p(route).legacy_ok, "{route}");
        }
        assert!(route_need(&Method::POST, "/v1/jobs").is_none());
        assert!(route_need(&Method::GET, "/v1/nope").is_none());
    }

    #[test]
    fn a_job_child_gets_its_identity_even_without_a_token() {
        use omniget_core::core::llm::cli_runtime::conversation_env;
        let id = job_identity_env("j7", "claude-code", Some("http://127.0.0.1:4242"));
        assert_eq!(id["OMNIGET_ENV"], "1");
        assert_eq!(id["OMNIGET_JOB_ID"], "j7");
        assert_eq!(id["OMNIGET_AGENT_ID"], "claude-code");
        assert_eq!(id["OMNIGET_BRIDGE_URL"], "http://127.0.0.1:4242");
        assert!(!job_identity_env("j7", "a", None).contains_key("OMNIGET_BRIDGE_URL"));
        let db = db();
        let conv = format!("identity-{}", uuid::Uuid::new_v4().simple());
        {
            let (_lease, err) = JobTokenLease::start_with(
                &conv,
                id.clone(),
                || mint(&db, Kind::Job, "job j7", &job_scopes(), Some("j7"), None, 1),
                || {},
            );
            assert!(err.is_none());
            let env = conversation_env(&conv);
            assert_eq!(env.get("OMNIGET_JOB_ID").map(String::as_str), Some("j7"));
            assert!(env
                .get(JOB_TOKEN_ENV)
                .is_some_and(|t| t.starts_with("ojt_")));
        }
        assert!(conversation_env(&conv).is_empty(), "gone with the lease");
        let (_lease, err) = JobTokenLease::start_with(&conv, id, || Err("no db".into()), || {});
        assert_eq!(err.as_deref(), Some("no db"));
        let env = conversation_env(&conv);
        assert_eq!(env.get("OMNIGET_JOB_ID").map(String::as_str), Some("j7"));
        assert!(!env.contains_key(JOB_TOKEN_ENV));
    }

    #[test]
    fn the_cli_token_file_is_reused_while_valid_and_private() {
        let db = db();
        let dir = std::env::temp_dir().join(format!("omniget-cli-token-{}", uuid::Uuid::new_v4()));
        ensure_cli_token(&db, &dir, 1).unwrap();
        let first = std::fs::read_to_string(dir.join(CLI_TOKEN_FILE)).unwrap();
        ensure_cli_token(&db, &dir, 2).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(CLI_TOKEN_FILE)).unwrap(),
            first
        );
        assert!(
            matches!(lookup(&db, &first, 3), Lookup::Valid(v) if v.kind == Kind::Cli && v.scopes.len() == SCOPES.len())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(CLI_TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let id = list(&db, 3)[0].id.clone();
        revoke(&db, &id, 4).unwrap();
        ensure_cli_token(&db, &dir, 5).unwrap();
        let second = std::fs::read_to_string(dir.join(CLI_TOKEN_FILE)).unwrap();
        assert_ne!(first, second);
        assert!(matches!(lookup(&db, &second, 6), Lookup::Valid(_)));
        let old = mint(
            &db,
            Kind::Cli,
            "old cli",
            &s(&[SCOPE_JOBS_READ, SCOPE_JOBS_OPERATE, SCOPE_APPROVALS_RESPOND]),
            None,
            None,
            7,
        )
        .unwrap();
        write_private(&dir.join(CLI_TOKEN_FILE), old.token.as_bytes()).unwrap();
        ensure_cli_token(&db, &dir, 8).unwrap();
        let third = std::fs::read_to_string(dir.join(CLI_TOKEN_FILE)).unwrap();
        assert_ne!(third, old.token);
        assert!(
            matches!(lookup(&db, &third, 9), Lookup::Valid(v) if v.scopes.contains(&SCOPE_AUDIT_READ.to_string()))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_crash_leaves_no_live_job_token() {
        let db = db();
        let j = mint(
            &db,
            Kind::Job,
            "job j1",
            &job_scopes(),
            Some("j1"),
            Some(10_000),
            1,
        )
        .unwrap();
        assert!(j.token.starts_with("ojt_"));
        assert_eq!(revoke_orphan_job_tokens(&db, 2).unwrap(), 1);
        assert_eq!(lookup(&db, &j.token, 3), Lookup::Ended);
    }

    #[test]
    fn a_canvas_token_reaches_only_the_canvas_routes_of_its_node() {
        let db = db();
        assert!(mint(
            &db,
            Kind::User,
            "x",
            &s(&[SCOPE_CANVAS_NODE]),
            None,
            None,
            1
        )
        .is_err());
        let first = mint_canvas(&db, "n1", "Planner", 1).unwrap();
        let t = mint_canvas(&db, "n1", "Planner", 2).unwrap();
        assert!(t.token.starts_with("oct_"));
        assert_eq!(lookup(&db, &first.token, 3), Lookup::Ended);
        let ctx = AuthCtx {
            extension: "ext",
            legacy_allowed: true,
            lookup: &|tok: &str| lookup(&db, tok, 3),
        };
        let need = route_need(&Method::GET, "/v1/canvas/peers").unwrap();
        assert_eq!(need.scope, SCOPE_CANVAS_NODE);
        assert!(!need.legacy_ok);
        let p = check_scoped(&headers(&t.token), &need, &ctx).unwrap();
        assert_eq!(p.id, "canvas:n1");
        let jobs = route_need(&Method::GET, "/v1/jobs").unwrap();
        assert!(matches!(
            check_scoped(&headers(&t.token), &jobs, &ctx),
            Err(Denied::Forbidden(_))
        ));
        assert!(matches!(
            check_scoped(&headers("ext"), &need, &ctx),
            Err(Denied::Forbidden(_))
        ));
        assert!(route_need(&Method::POST, "/v1/canvas/asks").is_some());
        assert!(route_need(&Method::GET, "/v1/canvas/asks/ask_1/wait").is_some());
        assert_eq!(revoke_canvas_node(&db, "n1", 4).unwrap(), 1);
        assert_eq!(lookup(&db, &t.token, 5), Lookup::Ended);
        let again = mint_canvas(&db, "n2", "Coder", 6).unwrap();
        assert_eq!(revoke_orphan_job_tokens(&db, 7).unwrap(), 0);
        assert!(matches!(lookup(&db, &again.token, 8), Lookup::Valid(_)));
        assert_eq!(revoke_canvas_node(&db, "n2", 9).unwrap(), 1);
        assert_eq!(lookup(&db, &again.token, 10), Lookup::Ended);
        assert!(list(&db, 11).is_empty());
    }

    #[test]
    fn a_canvas_token_survives_a_restart_and_is_reused_for_its_node() {
        let db = db();
        let t = mint_canvas(&db, "n1", "Planner", 1).unwrap();
        let job = mint(&db, Kind::Job, "job j1", &job_scopes(), Some("j1"), None, 1).unwrap();
        assert_eq!(revoke_orphan_job_tokens(&db, 2).unwrap(), 1);
        assert_eq!(lookup(&db, &job.token, 3), Lookup::Ended);
        assert_eq!(
            live_canvas_token(&db, "n1", &t.token, 3),
            Some(t.view.id.clone())
        );
        assert_eq!(live_canvas_token(&db, "n2", &t.token, 3), None);
        assert_eq!(live_canvas_token(&db, "j1", &job.token, 3), None);
        assert_eq!(live_canvas_token(&db, "n1", "oct_garbage", 3), None);
    }

    #[test]
    fn a_used_canvas_token_slides_its_expiry() {
        let db = db();
        let t = mint_canvas(&db, "n1", "Planner", 0).unwrap();
        let late = CANVAS_TOKEN_TTL_MS - 1;
        match lookup(&db, &t.token, late) {
            Lookup::Valid(v) => assert_eq!(v.expires_ms, Some(late + CANVAS_TOKEN_TTL_MS)),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            lookup(&db, &t.token, CANVAS_TOKEN_TTL_MS + 10),
            Lookup::Valid(_)
        ));
        let idle = mint_canvas(&db, "n2", "Coder", 0).unwrap();
        assert_eq!(lookup(&db, &idle.token, CANVAS_TOKEN_TTL_MS), Lookup::Ended);
    }
}

#[cfg(test)]
pub(crate) mod bn5 {
    use super::*;
    use axum::routing::{get, post};
    use axum::Router;
    use std::time::Instant;

    pub(crate) fn emit(line: serde_json::Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    async fn echo(axum::Extension(p): axum::Extension<Principal>) -> Json<serde_json::Value> {
        Json(json!({ "principal": p.id }))
    }

    pub(crate) fn router(g: Guard) -> Router {
        Router::new()
            .route("/v1/jobs", get(echo))
            .route("/v1/jobs/{id}", get(echo))
            .route("/v1/jobs/{id}/explain", get(echo))
            .route("/v1/jobs/{id}/diff", get(echo))
            .route("/v1/jobs/{id}/cancel", post(echo))
            .route("/v1/agent/run", post(echo))
            .route("/v1/agents/cli", post(echo))
            .route("/v1/hooks/{id}", post(echo))
            .route_layer(axum::middleware::from_fn_with_state(g, guard))
    }

    pub(crate) async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn bn5_auth() {
        let db = Arc::new(Mutex::new(db_for_test()));
        let now = Arc::new(AtomicU64::new(1_000));
        let flag = Arc::new(AtomicBool::new(true));
        let log = Arc::new(LegacyLog::default());
        let extension = "ext-key-0123456789abcdefghijklmnopqrstuvw".to_string();
        let g = Guard {
            extension: Arc::new(extension.clone()),
            legacy_allowed: flag.clone(),
            legacy_log: log.clone(),
            lookup: {
                let (db, now) = (db.clone(), now.clone());
                Arc::new(move |t: &str| lookup(&db.lock().unwrap(), t, now.load(Ordering::Relaxed)))
            },
            now: {
                let now = now.clone();
                Arc::new(move || now.load(Ordering::Relaxed))
            },
        };
        let base = serve(router(g)).await;
        let http = reqwest::Client::new();
        let mut slowest_ms: u128 = 0;
        let mut cases = 0;
        let mut failed = Vec::new();
        let mut call = async |case: &str,
                              method: &str,
                              path: &str,
                              token: Option<&str>,
                              want: u16|
               -> reqwest::Response {
            let url = format!("{base}{path}");
            let mut req = if method == "GET" {
                http.get(url)
            } else {
                http.post(url).json(&json!({}))
            };
            if let Some(t) = token {
                req = req.bearer_auth(t);
            }
            let t0 = Instant::now();
            let resp = req.send().await.unwrap();
            let ms = t0.elapsed().as_millis();
            slowest_ms = slowest_ms.max(ms);
            cases += 1;
            let status = resp.status().as_u16();
            if status != want {
                failed.push(format!("{case}: {status} != {want}"));
            }
            emit(json!({
                "scenario": "auth", "case": case, "route": format!("{method} {path}"),
                "status": status, "want": want, "pass": status == want, "ms": ms,
                "deprecation": resp.headers().contains_key("deprecation"),
            }));
            resp
        };

        call(
            "extension_refused_on_agent_run",
            "POST",
            "/v1/agent/run",
            Some(&extension),
            403,
        )
        .await;
        let r = call(
            "extension_grace_jobs_deprecated",
            "GET",
            "/v1/jobs",
            Some(&extension),
            200,
        )
        .await;
        assert!(r.headers().contains_key("deprecation"));
        call(
            "extension_grace_jobs",
            "GET",
            "/v1/jobs",
            Some(&extension),
            200,
        )
        .await;
        call(
            "extension_grace_agents_cli",
            "POST",
            "/v1/agents/cli",
            Some(&extension),
            403,
        )
        .await;
        let seen = log.last().map(|l| l.route);
        flag.store(false, Ordering::Relaxed);
        call(
            "extension_after_grace_run",
            "POST",
            "/v1/agent/run",
            Some(&extension),
            403,
        )
        .await;
        call(
            "extension_after_grace_jobs",
            "GET",
            "/v1/jobs",
            Some(&extension),
            403,
        )
        .await;
        call(
            "extension_after_grace_cancel",
            "POST",
            "/v1/jobs/j1/cancel",
            Some(&extension),
            403,
        )
        .await;
        call("no_token", "GET", "/v1/jobs", None, 401).await;
        call(
            "unknown_token",
            "GET",
            "/v1/jobs",
            Some("ogt_000000000000_x"),
            401,
        )
        .await;

        let (reader, operator) = {
            let db = db.lock().unwrap();
            (
                mint(
                    &db,
                    Kind::User,
                    "reader",
                    &s(&[SCOPE_JOBS_READ]),
                    None,
                    None,
                    1_000,
                )
                .unwrap(),
                mint(
                    &db,
                    Kind::User,
                    "operator",
                    &s(&[SCOPE_JOBS_READ, SCOPE_JOBS_OPERATE]),
                    None,
                    None,
                    1_000,
                )
                .unwrap(),
            )
        };
        call("reader_lists", "GET", "/v1/jobs", Some(&reader.token), 200).await;
        call(
            "reader_runs",
            "POST",
            "/v1/agent/run",
            Some(&reader.token),
            403,
        )
        .await;
        call(
            "reader_creates_agent",
            "POST",
            "/v1/agents/cli",
            Some(&reader.token),
            403,
        )
        .await;
        call(
            "operator_creates_agent",
            "POST",
            "/v1/agents/cli",
            Some(&operator.token),
            200,
        )
        .await;
        call(
            "operator_runs",
            "POST",
            "/v1/agent/run",
            Some(&operator.token),
            200,
        )
        .await;

        let lease = {
            let (db_m, db_r) = (db.clone(), db.clone());
            JobTokenLease::start(
                "bn5-conv-j1",
                move || {
                    mint(
                        &db_m.lock().unwrap(),
                        Kind::Job,
                        "job j1",
                        &job_scopes(),
                        Some("j1"),
                        Some(job_token_expiry(None, 1_000)),
                        1_000,
                    )
                },
                move || {
                    let _ = revoke_job(&db_r.lock().unwrap(), "j1", 2_000);
                },
            )
            .unwrap()
        };
        let env = omniget_core::core::llm::cli_runtime::conversation_env("bn5-conv-j1");
        let job_token = env
            .get(JOB_TOKEN_ENV)
            .cloned()
            .expect("the child env carries the job token");
        call(
            "job_token_own_job",
            "GET",
            "/v1/jobs/j1",
            Some(&job_token),
            200,
        )
        .await;
        call(
            "job_token_own_diff",
            "GET",
            "/v1/jobs/j1/diff",
            Some(&job_token),
            200,
        )
        .await;
        call(
            "job_token_other_job",
            "GET",
            "/v1/jobs/j2",
            Some(&job_token),
            403,
        )
        .await;
        call(
            "job_token_other_explain",
            "GET",
            "/v1/jobs/j2/explain",
            Some(&job_token),
            403,
        )
        .await;
        call(
            "job_token_global_list",
            "GET",
            "/v1/jobs",
            Some(&job_token),
            403,
        )
        .await;
        call(
            "job_token_starts_job",
            "POST",
            "/v1/agent/run",
            Some(&job_token),
            403,
        )
        .await;
        call(
            "job_token_cancels_itself",
            "POST",
            "/v1/jobs/j1/cancel",
            Some(&job_token),
            403,
        )
        .await;
        drop(lease);
        let env_after = omniget_core::core::llm::cli_runtime::conversation_env("bn5-conv-j1");
        call(
            "job_token_after_end",
            "GET",
            "/v1/jobs/j1",
            Some(&job_token),
            401,
        )
        .await;

        let short = mint(
            &db.lock().unwrap(),
            Kind::User,
            "short",
            &s(&[SCOPE_JOBS_READ]),
            None,
            Some(5_000),
            1_000,
        )
        .unwrap();
        call(
            "user_token_before_expiry",
            "GET",
            "/v1/jobs",
            Some(&short.token),
            200,
        )
        .await;
        now.store(5_000, Ordering::Relaxed);
        call(
            "user_token_after_expiry",
            "GET",
            "/v1/jobs",
            Some(&short.token),
            401,
        )
        .await;

        emit(json!({"scenario": "auth", "metric": "slowest_call_ms", "value": slowest_ms}));
        emit(json!({"scenario": "auth", "metric": "cases", "value": cases}));
        emit(json!({"scenario": "auth", "metric": "legacy_use_seen_by_ui", "value": seen}));
        emit(
            json!({"scenario": "auth", "metric": "job_env_cleared_at_end", "value": env_after.is_empty()}),
        );
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(seen.as_deref(), Some("/v1/jobs"));
        assert!(env_after.is_empty());
        assert!(slowest_ms < 2_000);
    }

    fn db_for_test() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db).unwrap();
        db
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
}
