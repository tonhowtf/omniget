//! Scoped control tokens of the local bridge (2.1, S08).
//!
//! Until 0.10 every agent route (`/v1/agent/*`, `/v1/jobs*`, `/v1/hooks/*`,
//! `/v1/agents*`, `/v1/loops`, `/v1/state`, `/v1/playbooks`) took the same
//! bearer as the browser extension, so a leaked extension key could start
//! agents with a shell (PROB S1) and create CLI agents (S2). These routes now
//! want a token of their own, with one of three scopes:
//!
//! * `jobs:read`: list jobs, their state, explain and diff;
//! * `jobs:operate`: start, cancel, create loops and agents, fire webhooks;
//! * `approvals:respond`: answer a pending ask (the routes arrive in 2.2).
//!
//! Tokens live in `jobs.db` (`bridge_tokens`), only as a SHA-256 of their
//! secret. The id travels in the token itself (`ogt_<id>_<secret>`), so a
//! lookup reads one row by id and compares the digest in constant time.
//! Three kinds:
//!
//! * `user`: minted from the UI (LLM → Jobs → Access tokens), for scripts
//!   and webhooks; optional expiry.
//! * `cli`: one per machine, minted by the app into `<llm_dir>/cli-token`
//!   (0600), which `omniget agent` reads; it never goes near the browser.
//! * `job`: minted when a job's turn starts and handed to the child as
//!   `OMNIGET_JOB_TOKEN`; bound to that job (another job's routes and the
//!   global routes answer 403) and revoked when the job ends (then 401).
//!
//! The principal is the MCP one (`mcp::policy::Principal`): same identity
//! model for the bridge and the MCP endpoint, so the audit log (2.8) and the
//! approvals (2.2) name callers the same way.
//!
//! Compatibility (`81#P8`, `82#P3`): the extension bearer keeps working on the
//! agent routes for one version while `bridge.legacy_agent_routes` is on
//! (the default in this version), with a `Deprecation` header, a log line and
//! a notice in the UI. It never creates agents (`/v1/agents/cli`), and it
//! never gets `approvals:respond`.

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
/// Reading the audit log (2.8): who did what with every token, so neither a
/// job's token nor the extension key ever has it.
pub const SCOPE_AUDIT_READ: &str = "audit:read";
/// Setting and raising budget ceilings (3.3): raising one lets agents spend
/// more, so neither a job's token nor the extension key ever has it.
pub const SCOPE_BUDGET_WRITE: &str = "budget:write";
/// Trusting a repository's `.omniget/workflow.md` (3.10): its hooks and
/// checks then run on this machine, so neither a job's token (an agent
/// trusting its own repository's commands) nor the extension key ever has it.
pub const SCOPE_WORKFLOW_TRUST: &str = "workflow:trust";
pub const SCOPES: &[&str] = &[
    SCOPE_JOBS_READ,
    SCOPE_JOBS_OPERATE,
    SCOPE_APPROVALS_RESPOND,
    SCOPE_AUDIT_READ,
    SCOPE_BUDGET_WRITE,
    SCOPE_WORKFLOW_TRUST,
];

/// What the child of a job finds its token in.
pub const JOB_TOKEN_ENV: &str = "OMNIGET_JOB_TOKEN";
/// Principal id of the extension bearer while the grace period lasts.
pub const EXTENSION_PRINCIPAL: &str = "extension";
/// Scopes the extension bearer keeps during the grace period: what it could
/// do on these routes before, minus answering asks.
const LEGACY_SCOPES: &[&str] = &[SCOPE_JOBS_READ, SCOPE_JOBS_OPERATE];

const USER_PREFIX: &str = "ogt_";
const JOB_PREFIX: &str = "ojt_";
/// A job token outlives the job's own deadline by this much at most (the
/// revoke at the end is what normally kills it).
const JOB_TOKEN_SLACK_MS: u64 = 5 * 60 * 1000;
/// A job without a deadline still gets an expiry.
const JOB_TOKEN_DEFAULT_TTL_MS: u64 = 24 * 60 * 60 * 1000;
/// `last_used_ms` is written at most this often per token.
const TOUCH_EVERY_MS: u64 = 60 * 1000;
/// Revoked job tokens are kept this long (a 401 needs the row), then purged.
const JOB_TOKEN_KEEP_MS: u64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    User,
    Cli,
    Job,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::User => "user",
            Kind::Cli => "cli",
            Kind::Job => "job",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "cli" => Kind::Cli,
            "job" => Kind::Job,
            _ => Kind::User,
        }
    }
}

/// A token as the UI and the guard see it: never the secret, never the hash.
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
    /// What the UI prints in place of the token (`ogt_<id>_…`).
    pub hint: String,
}

/// A fresh token: the only moment the secret exists outside its owner.
pub struct Minted {
    pub token: String,
    pub view: TokenView,
}

/// The tables of this module, created with the rest of `jobs.db`.
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
        _ => USER_PREFIX,
    }
}

fn hint_of(kind: Kind, id: &str) -> String {
    format!("{}{id}_…", prefix_of(kind))
}

/// `ogt_<id>_<secret>` → `(id, secret)`. Anything else is not ours.
fn parse(token: &str) -> Option<(&str, &str)> {
    let rest = token
        .strip_prefix(USER_PREFIX)
        .or_else(|| token.strip_prefix(JOB_PREFIX))?;
    let (id, secret) = rest.split_once('_')?;
    let hexish = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    (id.len() == 12 && secret.len() == 64 && hexish(id) && hexish(secret)).then_some((id, secret))
}

/// Mints a token. Scopes outside [`SCOPES`] are refused; an empty list too.
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
    if scopes.is_empty() || scopes.iter().any(|s| !SCOPES.contains(&s.as_str())) {
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

/// What a presented token is.
#[derive(Debug, Clone, PartialEq)]
pub enum Lookup {
    Valid(TokenView),
    /// Ours, but revoked or expired: 401, like a job token after its job.
    Ended,
    /// Not a token of this table.
    Unknown,
}

/// Reads the row by the id the token carries and compares the digest of its
/// secret in constant time. Refreshes `last_used_ms` now and then.
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
    // The prefix is part of the token: a job token cannot pass for a user one.
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
        let _ = db.execute(
            "UPDATE bridge_tokens SET last_used_ms=?2 WHERE id=?1",
            params![view.id, now as i64],
        );
        view.last_used_ms = Some(now);
    }
    Lookup::Valid(view)
}

/// Revokes one token. `false` when there was nothing live to revoke.
pub fn revoke(db: &Connection, id: &str, now: u64) -> Result<bool, String> {
    db.execute(
        "UPDATE bridge_tokens SET revoked_ms=?2 WHERE id=?1 AND revoked_ms IS NULL",
        params![id, now as i64],
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

/// Revokes every live token of a job (its end) and purges job tokens revoked
/// long ago, so the table does not grow with every job.
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

/// At boot no job runs yet: every job token left live by a crash is revoked
/// (a resumed job mints a new one).
pub fn revoke_orphan_job_tokens(db: &Connection, now: u64) -> Result<usize, String> {
    db.execute(
        "UPDATE bridge_tokens SET revoked_ms=?1 WHERE kind='job' AND revoked_ms IS NULL",
        params![now as i64],
    )
    .map_err(|e| e.to_string())
}

/// Live user and CLI tokens, newest first (job tokens come and go with their
/// jobs and are not listed).
pub fn list(db: &Connection, now: u64) -> Vec<TokenView> {
    let Ok(mut stmt) = db.prepare(&format!(
        "SELECT {COLS} FROM bridge_tokens WHERE kind!='job' AND revoked_ms IS NULL
         AND (expires_ms IS NULL OR expires_ms > ?1) ORDER BY created_ms DESC"
    )) else {
        return Vec::new();
    };
    stmt.query_map([now as i64], row_view)
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// The expiry of a job's token: its deadline plus a little, or a day.
pub fn job_token_expiry(deadline_ms: Option<u64>, now: u64) -> u64 {
    deadline_ms
        .map(|d| d.max(now) + JOB_TOKEN_SLACK_MS)
        .unwrap_or(now + JOB_TOKEN_DEFAULT_TTL_MS)
}

/// Scopes of a job's own token: it reads its own job, nothing more. It never
/// answers an ask (an agent approving itself, `85#P2`) and never starts or
/// cancels jobs.
pub fn job_scopes() -> Vec<String> {
    vec![SCOPE_JOBS_READ.to_string()]
}

// ── the CLI token file ────────────────────────────────────────────────

pub const CLI_TOKEN_FILE: &str = "cli-token";
const CLI_TOKEN_NAME: &str = "omniget CLI (this computer)";

/// Keeps `<dir>/cli-token` holding a live `cli` token with every scope: the
/// file is reused while its token is valid, else a new one is minted (and
/// the other `cli` tokens revoked) and written with owner-only permissions.
pub fn ensure_cli_token(db: &Connection, dir: &std::path::Path, now: u64) -> Result<(), String> {
    let path = dir.join(CLI_TOKEN_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Lookup::Valid(v) = lookup(db, text.trim(), now) {
            // A token minted before a scope existed (`audit:read`, 2.8) is
            // replaced, so the CLI keeps every scope.
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

/// Written to a temp file created owner-only (0600 on Unix), then renamed.
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

// ── routes → scope ────────────────────────────────────────────────────

/// What a route asks of the caller: a scope, and the job it is about (for a
/// job-bound token), and whether the extension bearer may still use it.
#[derive(Debug, Clone, PartialEq)]
pub struct Need {
    pub scope: &'static str,
    pub job: Option<String>,
    pub legacy_ok: bool,
}

/// The scope of every agent route of the bridge (`local_bridge_jobs`).
/// `None` = not mapped, which the guard refuses: a route added without a
/// line here fails closed.
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
        // The review of a job (3.5): reading it is a read of that job; any
        // change (a hunk put back, notes, integrate, discard, walkthrough,
        // which spends a model turn) is operating it.
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
        // Item 2.5: waiting is a read of that job.
        ["v1", "jobs", id, "wait"] if get => need(SCOPE_JOBS_READ, Some(id)),
        // The command surface: any valid user token passes the route, and
        // each command checks its own scope (`job.start` and
        // `job.interrupt` need `jobs:operate`, `approval.respond`
        // `approvals:respond`). A job's own token is refused here (it is
        // bound to its job): a job never starts a job. The extension key is
        // not accepted on a route that did not exist before it.
        ["v1", "commands"] if post => Some(Need {
            scope: SCOPE_JOBS_READ,
            job: None,
            legacy_ok: false,
        }),
        // The Catalog installs things (S1 residue of 2.1): an operator's
        // scope; the extension key keeps working through the grace period,
        // like the other agent routes, because `omniget agentkit` sent it.
        ["v1", "agentkit", _] if post => need(SCOPE_JOBS_OPERATE, None),
        ["v1", "agent", "run" | "run-tool" | "loop" | "loop-catalog"] if post => {
            need(SCOPE_JOBS_OPERATE, None)
        }
        ["v1", "hooks", _] if post => need(SCOPE_JOBS_OPERATE, None),
        // Item 2.2: reading the asks is a read of the jobs; answering one is
        // shell power, so it has its own scope, which neither a job's own
        // token nor the extension key ever has.
        ["v1", "approvals"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "approvals", _] if post => Some(Need {
            scope: SCOPE_APPROVALS_RESPOND,
            job: None,
            legacy_ok: false,
        }),
        // The audit log (2.8): its own scope, never the extension's.
        ["v1", "audit"] if get => Some(Need {
            scope: SCOPE_AUDIT_READ,
            job: None,
            legacy_ok: false,
        }),
        // Budgets (3.3): reading them is a read of the jobs; setting or
        // raising one has its own scope, never the extension's.
        ["v1", "budget"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "budget", "policies"] | ["v1", "budget", "incidents", _] if post => Some(Need {
            scope: SCOPE_BUDGET_WRITE,
            job: None,
            legacy_ok: false,
        }),
        // A repository's workflow (3.10): reading it is a read; trusting it
        // lets its commands run, so it has its own scope.
        ["v1", "workflow"] if get => need(SCOPE_JOBS_READ, None),
        ["v1", "workflow", "trust"] if post => Some(Need {
            scope: SCOPE_WORKFLOW_TRUST,
            job: None,
            legacy_ok: false,
        }),
        // Creating an agent (with any account) was never something the
        // extension did: its bearer is refused here right away (S2).
        ["v1", "agents", "cli"] if post => Some(Need {
            scope: SCOPE_JOBS_OPERATE,
            job: None,
            legacy_ok: false,
        }),
        _ => None,
    }
}

// ── the check ─────────────────────────────────────────────────────────

/// Why a request was refused. 401 = no valid credential (none, unknown,
/// revoked, expired); 403 = a valid one that may not do this.
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

/// Everything the check needs besides the request.
pub struct AuthCtx<'a> {
    /// The extension bearer (`settings.bridge.token`).
    pub extension: &'a str,
    /// `bridge.legacy_agent_routes`: the grace period of the extension bearer.
    pub legacy_allowed: bool,
    pub lookup: &'a dyn Fn(&str) -> Lookup,
}

fn bearer_of(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
}

/// The principal behind the request if it may do what `need` says.
///
/// Order: a token of this table first (so a scoped token is never mistaken
/// for anything else), then the extension bearer under the grace rules.
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

// ── the grace period: flag and last use, for the log and the UI ─────────

/// The last time the extension bearer was used on an agent route.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct LegacyUse {
    pub at_ms: u64,
    pub route: String,
}

#[derive(Default)]
pub struct LegacyLog {
    last: Mutex<Option<LegacyUse>>,
    count: AtomicU64,
    /// When the log line was last written (one per minute is enough).
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
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }
}

/// The app's grace flag, set from the settings when the bridge starts and
/// when the user flips it.
pub fn legacy_flag() -> Arc<AtomicBool> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    FLAG.get_or_init(|| Arc::new(AtomicBool::new(true))).clone()
}

pub fn legacy_log() -> Arc<LegacyLog> {
    static LOG: OnceLock<Arc<LegacyLog>> = OnceLock::new();
    LOG.get_or_init(|| Arc::new(LegacyLog::default())).clone()
}

// ── the middleware ────────────────────────────────────────────────────

pub type LookupFn = Arc<dyn Fn(&str) -> Lookup + Send + Sync>;

/// State of [`guard`]: the extension bearer, the grace flag and log, and how
/// to read a token (the app reads `jobs.db`; tests hand their own database).
#[derive(Clone)]
pub struct Guard {
    pub extension: Arc<String>,
    pub legacy_allowed: Arc<AtomicBool>,
    pub legacy_log: Arc<LegacyLog>,
    pub lookup: LookupFn,
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

const DEPRECATION_WARNING: &str = "299 omniget \"the browser extension key stops working on agent routes in the next version; use a scoped token (OmniGet: LLM, Jobs, Access tokens)\"";

/// `route_layer` of the agent routes: maps the path to its scope, checks the
/// bearer, and hands the handler the [`Principal`] as a request extension.
pub async fn guard(State(g): State<Guard>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let Some(need) = route_need(req.method(), &path) else {
        return Denied::Forbidden(format!("no scope is defined for {path}")).into_response();
    };
    // The check borrows the lookup (not `Send`): done before any await.
    let checked = {
        let ctx = AuthCtx {
            extension: g.extension.as_str(),
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
    // Token use on an agent route (2.8), with the token's id as the actor.
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

// ── a job's token for its lifetime ─────────────────────────────────────

/// Holds a job's token while its turn runs: the child gets it through the
/// conversation's env, and dropping the lease (however `run_job` returns)
/// revokes it and takes it out of the env.
pub struct JobTokenLease {
    conversation: String,
    revoke: Option<Box<dyn FnOnce() + Send>>,
}

impl JobTokenLease {
    /// `mint` makes the token, `revoke` ends it; both talk to `jobs.db`.
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

    /// [`Self::start`] plus the job's identity in the child's env (2.6:
    /// `OMNIGET_ENV`, `OMNIGET_JOB_ID`, `OMNIGET_AGENT_ID`,
    /// `OMNIGET_BRIDGE_URL`), which the child gets even when the token
    /// could not be minted (the error comes back beside the lease).
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

/// The job's identity a child sees (2.6). `OMNIGET_JOB_ID` is what the
/// `omniget-agents` skill and `omniget agent run` read to refuse delegating
/// from inside a job (depth 1); the bridge address lets the job token call
/// back (2.1 left the child with a token and no port).
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
        // One changed character of the secret: not ours.
        let mut bad = m.token.clone();
        let last = bad.pop().unwrap();
        bad.push(if last == '0' { '1' } else { '0' });
        assert_eq!(lookup(&db, &bad, 2), Lookup::Unknown);
        // A job prefix on a user token: not ours either.
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
        // The list shows neither.
        assert!(list(&db, 200).is_empty());
    }

    /// 3.3: reading the budgets is a read of the jobs; setting or raising one
    /// needs `budget:write`, which neither a job's token nor the extension
    /// key ever has.
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

    /// 3.10: reading a workflow and a job's report are reads; trusting a
    /// workflow needs `workflow:trust`, which neither a job's token nor the
    /// extension key ever has.
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
        let src = include_str!("local_bridge_jobs.rs");
        // Only the served routes, not the ones the file's own tests build.
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
        // And no handler checks the extension bearer on its own any more.
        assert!(!src.contains("check_bearer("));
        assert!(src.contains("route_layer("));
    }

    /// A merge once moved `router()`'s `.route_layer(..)` to the end of
    /// another function, leaving the agent routes unguarded. In each file
    /// that serves agent routes, the guard is the last call of the builder:
    /// after every `.route(`/`.merge(` of that same function.
    #[test]
    fn the_guard_layer_comes_after_every_route() {
        for (name, src) in [
            ("local_bridge_jobs.rs", include_str!("local_bridge_jobs.rs")),
            (
                "local_bridge_agentkit.rs",
                include_str!("local_bridge_agentkit.rs"),
            ),
        ] {
            let served = src.split("#[cfg(test)]").next().unwrap();
            let start = served
                .find("pub fn router<")
                .unwrap_or_else(|| panic!("{name}: no router()"));
            let body = &served[start..];
            // The function ends at the first line that is just `}`.
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
            // And no other function of the file carries the guard.
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
        // 2.5: the command surface is closed to the extension key and to a
        // job's own token (bound to its job); waiting reads one job.
        let cmd = p("/v1/commands");
        assert!(!cmd.legacy_ok && cmd.job.is_none());
        let w = g("/v1/jobs/j1/wait");
        assert_eq!((w.scope, w.job.as_deref()), (SCOPE_JOBS_READ, Some("j1")));
        assert!(route_need(&Method::GET, "/v1/commands").is_none());
        assert_eq!(p("/v1/agentkit/apply").scope, SCOPE_JOBS_OPERATE);
        assert!(route_need(&Method::POST, "/v1/jobs").is_none());
        assert!(route_need(&Method::GET, "/v1/nope").is_none());
    }

    /// 2.6: the child of a job knows it is one (and where the bridge is),
    /// token or not; the lease takes it all away at the end.
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
        // Revoked from the UI: the next start writes a new one.
        let id = list(&db, 3)[0].id.clone();
        revoke(&db, &id, 4).unwrap();
        ensure_cli_token(&db, &dir, 5).unwrap();
        let second = std::fs::read_to_string(dir.join(CLI_TOKEN_FILE)).unwrap();
        assert_ne!(first, second);
        assert!(matches!(lookup(&db, &second, 6), Lookup::Valid(_)));
        // A CLI token from before `audit:read` (2.8) is replaced by one with it.
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
}

/// BN5 (`scripts/llm/delegation_e2e.py`), scenario `auth`: the real guard
/// over HTTP on 127.0.0.1, in front of stand-in handlers laid out like the
/// agent routes (the real handlers need the Tauri app). Every case is one
/// request with `reqwest`; the results go to `OMNIGET_BENCH_OUT` as JSON
/// lines. Item 2.5 adds its scenarios next to this one.
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

    /// The agent routes with the production layer on top.
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

        // 1. The extension key during the grace period: works, says so.
        let r = call(
            "extension_grace_run",
            "POST",
            "/v1/agent/run",
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
        // ... but never creates an agent (S2).
        call(
            "extension_grace_agents_cli",
            "POST",
            "/v1/agents/cli",
            Some(&extension),
            403,
        )
        .await;
        let seen = log.last().map(|l| l.route);
        // 2. After the grace flag: 403 everywhere.
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

        // 3. Scoped user tokens.
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

        // 4. A job's token, through the same lease `run_job` holds.
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

        // 5. Expiry.
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
