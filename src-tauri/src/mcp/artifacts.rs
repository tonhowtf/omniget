use super::auth::now_secs;
use super::policy::{self, Principal};
use omniget_core::core::secure_files::{Identity, Root, Snapshot};
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const ARTIFACT_GRANT_LIMIT: i64 = 100;
const ARTIFACT_TTL_SECS: i64 = 900;
static TRANSFERS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
pub fn admit() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    TRANSFERS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "ARTIFACT_BUSY".into())
}
fn db() -> Result<rusqlite::Connection, String> {
    let c = policy::db()?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS file_roots(id TEXT PRIMARY KEY, principal TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS artifact_grants(id TEXT PRIMARY KEY, principal TEXT NOT NULL, root TEXT NOT NULL, relative TEXT NOT NULL, digest TEXT NOT NULL, bytes INTEGER NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);").map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
    Ok(c)
}

pub fn grant_root(principal: &str, path: &Path) -> Result<String, String> {
    let c = db()?;
    let valid: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM clients WHERE id=?1 AND revoked=0)",
            [principal],
            |r| r.get(0),
        )
        .map_err(|_| "INVALID_CLIENT")?;
    if !valid {
        return Err("INVALID_CLIENT".into());
    }
    let path = path.canonicalize().map_err(|_| "INVALID_ROOT")?;
    let root = Root::open(&path, None).map_err(|_| "INVALID_ROOT")?;
    let identity = serde_json::to_string(root.identity()).map_err(|_| "INVALID_ROOT")?;
    use rusqlite::OptionalExtension;
    let existing: Option<String> = c.query_row(
        "SELECT id FROM file_roots WHERE principal=?1 AND path=?2 AND identity=?3 AND revoked=0 LIMIT 1",
        params![principal,path.to_str().ok_or("INVALID_ROOT")?,identity],
        |row|row.get(0),
    ).optional().map_err(|_|"ROOT_GRANT_FAILED")?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO file_roots(id,principal,path,identity) VALUES(?1,?2,?3,?4)",
        params![
            id,
            principal,
            path.to_str().ok_or("INVALID_ROOT")?,
            serde_json::to_string(root.identity()).map_err(|_| "INVALID_ROOT")?
        ],
    )
    .map_err(|_| "ROOT_GRANT_FAILED")?;
    Ok(id)
}
pub fn revoke_root(id: &str) -> Result<(), String> {
    db()?
        .execute("UPDATE file_roots SET revoked=1 WHERE id=?1", [id])
        .map_err(|_| "ROOT_REVOKE_FAILED")?;
    Ok(())
}

pub fn exec_root(
    principal: &str,
    grant_id: &str,
    path: &Path,
    identity: &Identity,
) -> Result<String, String> {
    let c = db()?;
    let id = format!("exec:{grant_id}");
    let identity_text = serde_json::to_string(identity).map_err(|_| "INVALID_ROOT")?;
    c.execute(
        "INSERT OR IGNORE INTO file_roots(id,principal,path,identity) VALUES(?1,?2,?3,?4)",
        params![
            id,
            principal,
            path.to_str().ok_or("INVALID_ROOT")?,
            identity_text
        ],
    )
    .map_err(|_| "ROOT_GRANT_FAILED")?;
    let (owner, stored, revoked): (String, String, i64) = c
        .query_row(
            "SELECT principal,identity,revoked FROM file_roots WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "ROOT_GRANT_FAILED")?;
    if owner != principal || stored != identity_text || revoked != 0 {
        return Err("ROOT_NOT_GRANTED".into());
    }
    Ok(id)
}
fn exec_grant_alive(principal: &str, root: &str) -> bool {
    let Some(grant) = root.strip_prefix("exec:") else {
        return true;
    };
    omniget_core::core::assist::db::global()
        .ok()
        .map(|db| omniget_core::core::assist::authority::get(&db, grant, principal).is_ok())
        .unwrap_or(false)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub artifact_id: String,
    pub digest: String,
    pub bytes: u64,
    pub name: String,
    pub expires_at: i64,
    pub transfer_allowed: bool,
    pub download_path: String,
}

pub fn register(p: &Principal, path: &Path) -> Result<Artifact, String> {
    policy::active(p)?;
    if !p.scopes.iter().any(|s| s == "transfer") {
        return Err("TRANSFER_NOT_GRANTED".into());
    }
    let c = db()?;
    c.execute(
        "DELETE FROM artifact_grants WHERE expires<=?1",
        [now_secs()],
    )
    .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    let roots = {
        let mut stmt = c
            .prepare("SELECT id,path,identity FROM file_roots WHERE principal=?1 AND revoked=0")
            .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
        let rows = stmt
            .query_map([&p.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?
    };
    let (root_id, relative, root) = first_open(roots, |(_, root_path, identity)| {
        let relative = path.strip_prefix(root_path).ok()?.to_path_buf();
        let Ok(identity) = serde_json::from_str::<Identity>(identity) else {
            return Some(Err("INVALID_ROOT"));
        };
        Some(
            Root::open(Path::new(root_path), Some(&identity))
                .map(|root| (relative, root))
                .map_err(|_| "ROOT_CHANGED"),
        )
    })
    .map(|((root_id, _, _), (relative, root))| (root_id, relative, root))
    .map_err(|code| match code {
        "ROOT_CHANGED" => "ROOT_CHANGED: the granted folder was replaced since the grant; ask the person to grant that folder again, then retry".to_string(),
        other => other.to_string(),
    })?;
    let relative = relative.as_path();
    let snap = root
        .snapshot(relative, MAX_BYTES)
        .map_err(|_| "ARTIFACT_UNSAFE_OR_CHANGED")?;
    c.execute_batch("BEGIN IMMEDIATE")
        .map_err(|_| "ARTIFACT_STORE_BUSY")?;
    let issued = issue(&c, p, &root_id, relative, &snap);
    let end = if issued.is_ok() { "COMMIT" } else { "ROLLBACK" };
    if c.execute_batch(end).is_err() {
        let _ = c.execute_batch("ROLLBACK");
        issued?;
        return Err("ARTIFACT_STORE_UNAVAILABLE".into());
    }
    issued
}

fn first_open<R, T>(
    roots: impl IntoIterator<Item = R>,
    mut open: impl FnMut(&R) -> Option<Result<T, &'static str>>,
) -> Result<(R, T), &'static str> {
    let mut failed = None;
    for root in roots {
        match open(&root) {
            None => {}
            Some(Ok(opened)) => return Ok((root, opened)),
            Some(Err(code)) => failed = Some(code),
        }
    }
    Err(failed.unwrap_or("ROOT_NOT_GRANTED"))
}

fn issue(
    c: &rusqlite::Connection,
    p: &Principal,
    root_id: &str,
    relative: &Path,
    snap: &Snapshot,
) -> Result<Artifact, String> {
    use rusqlite::OptionalExtension;
    let rel = relative.to_str().ok_or("INVALID_ARTIFACT")?;
    let name = relative
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let now = now_secs();
    let old: Option<(String, i64)> = c
        .query_row(
            "SELECT id,expires FROM artifact_grants WHERE principal=?1 AND root=?2 AND relative=?3 AND digest=?4 AND revoked=0 AND expires>?5",
            params![p.id, root_id, rel, snap.digest, now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    if let Some((id, expires)) = old {
        return Ok(Artifact {
            download_path: format!("/mcp/artifacts/{id}"),
            artifact_id: id,
            digest: snap.digest.clone(),
            bytes: snap.bytes,
            name,
            expires_at: expires,
            transfer_allowed: true,
        });
    }
    c.execute(
        "DELETE FROM artifact_grants WHERE principal=?1 AND (revoked<>0 OR expires<=?2)",
        params![p.id, now],
    )
    .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    let live: i64 = c
        .query_row(
            "SELECT count(*) FROM artifact_grants WHERE principal=?1",
            [&p.id],
            |r| r.get(0),
        )
        .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    let over = live + 1 - ARTIFACT_GRANT_LIMIT;
    if over > 0 {
        c.execute(
            "DELETE FROM artifact_grants WHERE id IN (SELECT id FROM artifact_grants WHERE principal=?1 ORDER BY expires ASC, rowid ASC LIMIT ?2)",
            params![p.id, over],
        )
        .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let expires = now + ARTIFACT_TTL_SECS;
    c.execute(
        "INSERT INTO artifact_grants(id,principal,root,relative,digest,bytes,expires) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![id, p.id, root_id, rel, snap.digest, snap.bytes, expires],
    )
    .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    Ok(Artifact {
        download_path: format!("/mcp/artifacts/{id}"),
        artifact_id: id,
        digest: snap.digest.clone(),
        bytes: snap.bytes,
        name,
        expires_at: expires,
        transfer_allowed: true,
    })
}

pub fn authorized(p: &Principal, id: &str) -> Result<(), String> {
    policy::active(p)?;
    if !p.scopes.iter().any(|s| s == "transfer") {
        return Err("TRANSFER_NOT_GRANTED".into());
    }
    let yes:bool=db()?.query_row("SELECT EXISTS(SELECT 1 FROM artifact_grants a JOIN file_roots r ON r.id=a.root WHERE a.id=?1 AND a.principal=?2 AND r.principal=?2 AND a.revoked=0 AND r.revoked=0 AND a.expires>?3)",params![id,p.id,now_secs()],|r|r.get(0)).map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
    if !yes {
        return Err("ARTIFACT_NOT_FOUND_OR_EXPIRED".into());
    }
    let root: String = db()?
        .query_row("SELECT root FROM artifact_grants WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
    if exec_grant_alive(&p.id, &root) {
        Ok(())
    } else {
        Err("ARTIFACT_NOT_FOUND_OR_EXPIRED".into())
    }
}
pub fn describe(p: &Principal, id: &str) -> Result<Artifact, String> {
    authorized(p, id)?;
    let (relative,digest,bytes,expires):(String,String,u64,i64)=db()?.query_row("SELECT relative,digest,bytes,expires FROM artifact_grants WHERE id=?1 AND principal=?2",params![id,p.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"ARTIFACT_NOT_FOUND_OR_EXPIRED")?;
    let name = Path::new(&relative)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok(Artifact {
        download_path: format!("/mcp/artifacts/{id}"),
        artifact_id: id.to_owned(),
        digest,
        bytes,
        name,
        expires_at: expires,
        transfer_allowed: true,
    })
}
pub fn open(p: &Principal, id: &str, digest: &str) -> Result<Snapshot, String> {
    authorized(p, id)?;
    let (path,identity,relative,expected,bytes):(String,String,String,String,u64)=db()?.query_row("SELECT r.path,r.identity,a.relative,a.digest,a.bytes FROM artifact_grants a JOIN file_roots r ON r.id=a.root WHERE a.id=?1 AND a.principal=?2",params![id,p.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"ARTIFACT_NOT_FOUND")?;
    if digest != expected {
        return Err("ARTIFACT_REVISION_MISMATCH".into());
    }
    let identity = serde_json::from_str(&identity).map_err(|_| "INVALID_ROOT")?;
    let root = Root::open(Path::new(&path), Some(&identity)).map_err(|_| "ROOT_CHANGED")?;
    let snap = root
        .snapshot(Path::new(&relative), MAX_BYTES)
        .map_err(|_| "ARTIFACT_UNSAFE_OR_CHANGED")?;
    if snap.digest != expected || snap.bytes != bytes {
        return Err("ARTIFACT_REVISION_MISMATCH".into());
    }
    authorized(p, id)?;
    Ok(snap)
}
pub fn preflight(
    p: &Principal,
    id: &str,
    digest: &str,
    range_header: Option<&str>,
) -> Result<(u64, u64), String> {
    authorized(p, id)?;
    let (expected, bytes): (String, u64) = db()?
        .query_row(
            "SELECT digest,bytes FROM artifact_grants WHERE id=?1 AND principal=?2",
            params![id, p.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "ARTIFACT_NOT_FOUND")?;
    if expected != digest {
        return Err("ARTIFACT_REVISION_MISMATCH".into());
    }
    range(range_header, bytes)
}

pub fn range(header: Option<&str>, bytes: u64) -> Result<(u64, u64), String> {
    if bytes == 0 {
        return Err("EMPTY_ARTIFACT".into());
    }
    let Some(h) = header else {
        return Ok((0, bytes - 1));
    };
    let (a, b) = h
        .strip_prefix("bytes=")
        .and_then(|s| s.split_once('-'))
        .ok_or("INVALID_RANGE")?;
    let start: u64 = a.parse().map_err(|_| "INVALID_RANGE")?;
    let end = if b.is_empty() {
        bytes - 1
    } else {
        b.parse::<u64>().map_err(|_| "INVALID_RANGE")?
    };
    if start > end || end >= bytes {
        return Err("INVALID_RANGE".into());
    }
    Ok((start, end))
}

#[cfg(test)]
mod root_pick_tests {
    use super::first_open;

    #[test]
    fn a_stale_grant_does_not_hide_the_live_one_for_the_same_folder() {
        let roots = ["stale", "live"];
        let got = first_open(roots, |r| {
            Some(if *r == "live" {
                Ok(1)
            } else {
                Err("ROOT_CHANGED")
            })
        });
        assert_eq!(got, Ok(("live", 1)));
    }

    #[test]
    fn only_stale_grants_report_the_change() {
        let got = first_open(["stale"], |_| Some(Err::<(), _>("ROOT_CHANGED")));
        assert_eq!(got, Err("ROOT_CHANGED"));
    }

    #[test]
    fn a_folder_outside_every_grant_is_not_granted() {
        let got = first_open(["other"], |_| None::<Result<(), &'static str>>);
        assert_eq!(got, Err("ROOT_NOT_GRANTED"));
    }
}
