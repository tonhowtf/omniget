//! External MCP identity and grants. Never accepts the extension credential.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};

static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
}
#[derive(Serialize)]
pub struct ConnectionGrant {
    pub principal: Principal,
    pub token: String,
}
#[cfg(test)]
thread_local! { pub(super) static TEST_DIR: std::cell::RefCell<Option<std::path::PathBuf>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn db() -> Result<Connection, String> {
    #[cfg(not(test))]
    let dir = omniget_core::core::tools::tools_dir()
        .ok_or("no data directory")?
        .join("mcp");
    #[cfg(test)]
    let dir = TEST_DIR
        .with(|d| d.borrow().clone())
        .expect("policy tests require an isolated directory");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let db = Connection::open(dir.join("policy.sqlite3")).map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
      CREATE TABLE IF NOT EXISTS clients(id TEXT PRIMARY KEY, name TEXT NOT NULL, digest TEXT UNIQUE NOT NULL, scopes TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS owners(download INTEGER PRIMARY KEY, principal TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS receipts(principal TEXT NOT NULL, operation TEXT NOT NULL, key TEXT NOT NULL, fingerprint TEXT NOT NULL, result TEXT, PRIMARY KEY(principal,operation,key));")
      .map_err(|e|e.to_string())?;
    Ok(db)
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub const SCOPES: &[&str] = &[
    "discover",
    "enqueue",
    "control",
    "diagnostics",
    "artifacts",
    "history",
    "local_network",
    "transfer",
    "orchestration_read",
    "orchestration_control",
    "orchestration_create",
    "auth",
];
pub fn create(name: String, scopes: Vec<String>) -> Result<ConnectionGrant, String> {
    if name.trim().is_empty()
        || name.len() > 80
        || scopes.iter().any(|s| !SCOPES.contains(&s.as_str()))
    {
        return Err("INVALID_GRANT".into());
    }
    let principal = Principal {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        scopes,
    };
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    db()?
        .execute(
            "INSERT INTO clients(id,name,digest,scopes) VALUES(?1,?2,?3,?4)",
            params![
                principal.id,
                principal.name,
                digest(&token),
                serde_json::to_string(&principal.scopes).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(ConnectionGrant { principal, token })
}
pub fn list() -> Result<Vec<Principal>, String> {
    let db = db()?;
    let mut stmt = db
        .prepare("SELECT id,name,scopes FROM clients WHERE revoked=0 ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Principal {
                id: r.get(0)?,
                name: r.get(1)?,
                scopes: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
pub fn revoke(id: &str) -> Result<(), String> {
    #[cfg(not(test))]
    {
        let assist = omniget_core::core::assist::db::global();
        revoke_in(id, assist.as_deref().map(Some).map_err(Clone::clone))
    }
    #[cfg(test)]
    revoke_in(id, Ok(None))
}
/// Fail closed (L1): the execution grants (assist.db) are revoked first; if
/// that database is unavailable nothing is marked and the error surfaces, so
/// a "revoked" client can never keep live grants behind it.
pub(super) fn revoke_in(
    id: &str,
    assist: Result<Option<&omniget_core::core::assist::db::AssistDb>, String>,
) -> Result<(), String> {
    let assist =
        assist.map_err(|_| "ASSIST_DB_UNAVAILABLE: client not revoked, retry".to_string())?;
    if let Some(assist) = assist {
        omniget_core::core::assist::authority::revoke_principal(assist, id)?;
    }
    db()?
        .execute("UPDATE clients SET revoked=1 WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn authenticate(token: &str) -> Result<Principal, String> {
    if token.len() != 64 {
        return Err("UNAUTHORIZED".into());
    }
    db()?
        .query_row(
            "SELECT id,name,scopes FROM clients WHERE digest=?1 AND revoked=0",
            [digest(token)],
            |r| {
                Ok(Principal {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    scopes: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                })
            },
        )
        .map_err(|_| "UNAUTHORIZED".into())
}
pub fn active(p: &Principal) -> Result<(), String> {
    let yes: bool = db()?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM clients WHERE id=?1 AND revoked=0)",
            [&p.id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if yes {
        Ok(())
    } else {
        Err("REVOKED".into())
    }
}
/// Domain recovery loads current scopes, never a persisted bearer credential.
pub fn principal(id: &str) -> Result<Principal, String> {
    db()?
        .query_row(
            "SELECT id,name,scopes FROM clients WHERE id=?1 AND revoked=0",
            [id],
            |r| {
                Ok(Principal {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    scopes: serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                })
            },
        )
        .map_err(|_| "PRINCIPAL_INACTIVE".into())
}
pub fn scope(name: &str) -> Option<&'static str> {
    Some(match name {
        "omniget_capabilities"
        | "omniget_health"
        | "destinations_list"
        | "media_inspect"
        | "media_collection_list"
        | "downloads_preflight" => "discover",
        "download_enqueue" | "downloads_batch_enqueue" => "enqueue",
        "downloads_queue" | "download_status" | "download_wait" => "discover",
        "download_cancel" | "download_pause" | "download_resume" | "download_retry" => "control",
        "download_logs" | "download_diagnose" | "download_recovery_options" => "diagnostics",
        "download_artifacts" | "artifact_access" => "artifacts",
        "diagnostic_bundle_create" => "diagnostics",
        "auth_connection_status" | "auth_connection_request" => "auth",
        "download_history" => "history",
        "agents_list"
        | "workspaces_list"
        | "missions_list"
        | "missions_get"
        | "missions_events"
        | "missions_artifacts"
        | "missions_diagnostics"
        | "approvals_list" => "orchestration_read",
        "missions_cancel" | "missions_pause" | "missions_resume" => "orchestration_control",
        "missions_create" => "orchestration_create",
        "agents_derived_list" | "groups_get" => "orchestration_read",
        "agents_prepare" | "groups_prepare" | "group_tasks_create" => "orchestration_create",
        _ => return None,
    })
}
pub fn allowed(p: &Principal, name: &str) -> bool {
    scope(name).is_some_and(|s| p.scopes.iter().any(|v| v == s))
}
pub fn own(p: &Principal, id: u64) -> Result<(), String> {
    let yes: bool = db()?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM owners WHERE download=?1 AND principal=?2)",
            params![id, p.id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if yes {
        Ok(())
    } else {
        Err("DOWNLOAD_NOT_FOUND".into())
    }
}
pub fn claim(p: &Principal, id: u64) -> Result<(), String> {
    db()?
        .execute(
            "INSERT OR IGNORE INTO owners VALUES(?1,?2)",
            params![id, p.id],
        )
        .map_err(|e| e.to_string())?;
    own(p, id)
}
pub fn owned(p: &Principal) -> Result<Vec<u64>, String> {
    let db = db()?;
    let mut stmt = db
        .prepare("SELECT download FROM owners WHERE principal=?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([&p.id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
/// Durable reservation before effects. An interrupted reservation is unknown,
/// never an instruction to repeat an effect.
pub fn receipt_exists(p: &Principal, op: &str, key: &str) -> Result<bool, String> {
    db()?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3)",
            params![p.id, op, key],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
}

/// Read an existing intent without reserving a new one. Pending stays unknown.
pub fn receipt(
    p: &Principal,
    op: &str,
    key: &str,
    args: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    active(p)?;
    if key.is_empty() || key.len() > 100 {
        return Err("INVALID_IDEMPOTENCY_KEY".into());
    }
    let row:Option<(String,Option<String>)>=db()?.query_row("SELECT fingerprint,result FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3",params![p.id,op,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"RECEIPT_UNAVAILABLE")?;
    match row {
        None => Ok(None),
        Some((hash, _)) if hash != digest(&args.to_string()) => Err("IDEMPOTENCY_CONFLICT".into()),
        Some((_, None)) => Err("OUTCOME_UNKNOWN".into()),
        Some((_, Some(body))) => serde_json::from_str(&body)
            .map(Some)
            .map_err(|_| "RECEIPT_INVALID".into()),
    }
}
pub fn reserve(
    p: &Principal,
    op: &str,
    key: &str,
    args: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    if key.is_empty() || key.len() > 100 {
        return Err("INVALID_IDEMPOTENCY_KEY".into());
    }
    let _guard = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "POLICY_LOCK")?;
    let db = db()?;
    let fingerprint = digest(&args.to_string());
    let old:Option<(String,Option<String>)>=db.query_row("SELECT fingerprint,result FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3",params![p.id,op,key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    if let Some((hash, result)) = old {
        if hash != fingerprint {
            return Err("IDEMPOTENCY_CONFLICT".into());
        }
        return result
            .map(|s| {
                serde_json::from_str(&s)
                    .map(Some)
                    .map_err(|e| e.to_string())
            })
            .unwrap_or_else(|| {
                Err(
                    "OUTCOME_UNKNOWN: inspect the owned queue before creating another intent"
                        .into(),
                )
            });
    }
    db.execute(
        "INSERT INTO receipts VALUES(?1,?2,?3,?4,NULL)",
        params![p.id, op, key, fingerprint],
    )
    .map_err(|e| e.to_string())?;
    Ok(None)
}
/// Drops a still-pending reservation after a KNOWN refusal with no effect
/// (the domain transaction rolled back), so the key is not poisoned.
/// A finished receipt is never removed.
pub fn release(p: &Principal, op: &str, key: &str) -> Result<(), String> {
    db()?
        .execute(
            "DELETE FROM receipts WHERE principal=?1 AND operation=?2 AND key=?3 AND result IS NULL",
            params![p.id, op, key],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn finish(
    p: &Principal,
    op: &str,
    key: &str,
    result: &serde_json::Value,
) -> Result<(), String> {
    db()?
        .execute(
            "UPDATE receipts SET result=?4 WHERE principal=?1 AND operation=?2 AND key=?3",
            params![p.id, op, key, result.to_string()],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deny_by_default_including_direct_calls() {
        let p = Principal {
            id: "a".into(),
            name: "A".into(),
            scopes: vec!["discover".into()],
        };
        assert!(allowed(&p, "download_status"));
        for n in [
            "download_enqueue",
            "agent_delegate",
            "help_apply",
            "unknown",
        ] {
            assert!(!allowed(&p, n));
        }
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn owner_revocation_and_intent_isolation_survive_reopening() {
        let dir =
            std::env::temp_dir().join(format!("omniget-policy-test-{}", uuid::Uuid::new_v4()));
        TEST_DIR.with(|p| *p.borrow_mut() = Some(dir.clone()));
        let a = create("A".into(), vec!["discover".into(), "enqueue".into()]).unwrap();
        let b = create("B".into(), vec!["discover".into()]).unwrap();
        assert_eq!(authenticate(&a.token).unwrap().id, a.principal.id);
        claim(&a.principal, 42).unwrap();
        assert!(own(&b.principal, 42).is_err());
        assert!(claim(&b.principal, 42).is_err());
        assert!(owned(&b.principal).unwrap().is_empty());
        let args = serde_json::json!({"url":"https://example.com/video"});
        assert!(reserve(&a.principal, "enqueue", "same", &args)
            .unwrap()
            .is_none());
        assert!(reserve(&a.principal, "enqueue", "same", &args)
            .unwrap_err()
            .starts_with("OUTCOME_UNKNOWN"));
        finish(
            &a.principal,
            "enqueue",
            "same",
            &serde_json::json!({"id":42}),
        )
        .unwrap();
        assert_eq!(
            reserve(&a.principal, "enqueue", "same", &args).unwrap(),
            Some(serde_json::json!({"id":42}))
        );
        assert!(reserve(&b.principal, "enqueue", "same", &args)
            .unwrap()
            .is_none());
        assert_eq!(
            reserve(
                &a.principal,
                "enqueue",
                "same",
                &serde_json::json!({"url":"https://example.com/other"})
            )
            .unwrap_err(),
            "IDEMPOTENCY_CONFLICT"
        );
        // A released pending intent frees the key; a finished one stays.
        let ctl = serde_json::json!({"missionId":"m","idempotencyKey":"k"});
        assert!(reserve(&a.principal, "missions_pause", "k", &ctl)
            .unwrap()
            .is_none());
        release(&a.principal, "missions_pause", "k").unwrap();
        assert!(receipt(&a.principal, "missions_pause", "k", &ctl)
            .unwrap()
            .is_none());
        release(&a.principal, "enqueue", "same").unwrap();
        assert_eq!(
            receipt(&a.principal, "enqueue", "same", &args).unwrap(),
            Some(serde_json::json!({"id":42}))
        );
        revoke(&a.principal.id).unwrap();
        assert!(authenticate(&a.token).is_err());
        assert!(active(&a.principal).is_err());
        assert!(authenticate(&b.token).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn client_revocation_fails_closed_without_the_assistant_db() {
        use omniget_core::core::assist::{authority, db::AssistDb};
        let dir = std::env::temp_dir().join(format!("omniget-policy-l1-{}", uuid::Uuid::new_v4()));
        TEST_DIR.with(|p| *p.borrow_mut() = Some(dir.clone()));
        let a = create("A".into(), vec!["orchestration_read".into()]).unwrap();
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let assist = AssistDb::open_in_memory().unwrap();
        let grant = authority::grant(
            &assist,
            authority::Ceiling {
                id: String::new(),
                principal: a.principal.id.clone(),
                workspace_id: "w".into(),
                workspace: ws.to_string_lossy().into_owned(),
                workspace_identity: None,
                bots: vec!["b".into()],
                bot_revisions: std::collections::BTreeMap::from([("b".into(), "a".repeat(64))]),
                tools: vec!["fs_read".into()],
                max_tokens: 10,
                max_usd: None,
            },
        )
        .unwrap();
        // assist.db unavailable: nothing is marked, the error surfaces.
        assert!(revoke_in(&a.principal.id, Err("ERR_ASSIST_DB".into()))
            .unwrap_err()
            .starts_with("ASSIST_DB_UNAVAILABLE"));
        assert!(active(&a.principal).is_ok());
        assert!(authority::get(&assist, &grant, &a.principal.id).is_ok());
        // Available: grants first, then the client.
        revoke_in(&a.principal.id, Ok(Some(&assist))).unwrap();
        assert!(authority::get(&assist, &grant, &a.principal.id).is_err());
        assert!(active(&a.principal).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

pub fn operation_count(p: &Principal, op: &str, id: u64) -> Result<u64, String> {
    let db = db()?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS retry_counts(principal TEXT, operation TEXT, download INTEGER, count INTEGER, PRIMARY KEY(principal,operation,download));").map_err(|e|e.to_string())?;
    db.execute("INSERT INTO retry_counts VALUES(?1,?2,?3,1) ON CONFLICT(principal,operation,download) DO UPDATE SET count=count+1",params![p.id,op,id]).map_err(|e|e.to_string())?;
    db.query_row(
        "SELECT count FROM retry_counts WHERE principal=?1 AND operation=?2 AND download=?3",
        params![p.id, op, id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
