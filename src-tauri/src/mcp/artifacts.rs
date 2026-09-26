//! Artifact grants live beside MCP identity; media remains owned by the queue.
//! A grant selects a locally opened root object, never a caller-supplied path.
use super::policy::{self, Principal};
use omniget_core::core::secure_files::{Identity, Root, Snapshot};
use rusqlite::params;
use serde::Serialize;
use std::path::Path;

const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
static TRANSFERS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
/// Includes hashing/copying and streaming lifetime: at most two 2GiB snapshots.
pub fn admit() -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    TRANSFERS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "ARTIFACT_BUSY".into())
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn db() -> Result<rusqlite::Connection, String> {
    let c = policy::db()?;
    c.execute_batch("CREATE TABLE IF NOT EXISTS file_roots(id TEXT PRIMARY KEY, principal TEXT NOT NULL, path TEXT NOT NULL, identity TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS artifact_grants(id TEXT PRIMARY KEY, principal TEXT NOT NULL, root TEXT NOT NULL, relative TEXT NOT NULL, digest TEXT NOT NULL, bytes INTEGER NOT NULL, expires INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);").map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
    Ok(c)
}

/// Trusted local UI only. Canonicalization is a local user's selection;
/// subsequent opens pin and check the selected directory's device/inode.
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

/// Root of an external mission workspace, derived from a live local execution
/// grant (the user chose that folder in the trusted UI). Idempotent per grant.
/// Transfers from it are re-checked against the execution grant on every
/// access, so revoking the grant revokes its artifact links too.
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
    c.execute("DELETE FROM artifact_grants WHERE expires<=?1", [now()])
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
    for (root_id, root_path, identity) in roots {
        let Ok(relative) = path.strip_prefix(&root_path) else {
            continue;
        };
        let identity: Identity = serde_json::from_str(&identity).map_err(|_| "INVALID_ROOT")?;
        let root =
            Root::open(Path::new(&root_path), Some(&identity)).map_err(|_| "ROOT_CHANGED")?;
        let snap = root
            .snapshot(relative, MAX_BYTES)
            .map_err(|_| "ARTIFACT_UNSAFE_OR_CHANGED")?;
        c.execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| "ARTIFACT_STORE_BUSY")?;
        // One live grant per recipient/root/path/revision, even when clients poll.
        use rusqlite::OptionalExtension;
        let old:Option<(String,i64)>=c.query_row("SELECT id,expires FROM artifact_grants WHERE principal=?1 AND root=?2 AND relative=?3 AND digest=?4 AND revoked=0 AND expires>?5",params![p.id,root_id,relative.to_str().ok_or("INVALID_ARTIFACT")?,snap.digest,now()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
        if let Some((id, expires)) = old {
            return Ok(Artifact {
                download_path: format!("/mcp/artifacts/{id}"),
                artifact_id: id,
                digest: snap.digest,
                bytes: snap.bytes,
                name: relative
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                expires_at: expires,
                transfer_allowed: true,
            });
        }
        let count: u64 = c
            .query_row(
                "SELECT count(*) FROM artifact_grants WHERE principal=?1",
                [&p.id],
                |r| r.get(0),
            )
            .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
        if count >= 100 {
            return Err("ARTIFACT_GRANT_LIMIT".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let expires = now() + 900;
        c.execute("INSERT INTO artifact_grants(id,principal,root,relative,digest,bytes,expires) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,p.id,root_id,relative.to_str().ok_or("INVALID_ARTIFACT")?,snap.digest,snap.bytes,expires]).map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
        c.execute_batch("COMMIT")
            .map_err(|_| "ARTIFACT_STORE_UNAVAILABLE")?;
        return Ok(Artifact {
            download_path: format!("/mcp/artifacts/{id}"),
            artifact_id: id,
            digest: snap.digest,
            bytes: snap.bytes,
            name: relative
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            expires_at: expires,
            transfer_allowed: true,
        });
    }
    Err("ROOT_NOT_GRANTED".into())
}

pub fn authorized(p: &Principal, id: &str) -> Result<(), String> {
    policy::active(p)?;
    if !p.scopes.iter().any(|s| s == "transfer") {
        return Err("TRANSFER_NOT_GRANTED".into());
    }
    let yes:bool=db()?.query_row("SELECT EXISTS(SELECT 1 FROM artifact_grants a JOIN file_roots r ON r.id=a.root WHERE a.id=?1 AND a.principal=?2 AND r.principal=?2 AND a.revoked=0 AND r.revoked=0 AND a.expires>?3)",params![id,p.id,now()],|r|r.get(0)).map_err(|_|"ARTIFACT_STORE_UNAVAILABLE")?;
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
/// Metadata of one live grant of this recipient (for `artifact_access`); the
/// same checks as a transfer, without opening the file.
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

/// Single explicit range only. Multipart/suffix ranges are intentionally not
/// supported, so a small request cannot multiply server work.
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
mod tests {
    use super::*;
    #[test]
    fn ranges_are_bounded_and_unambiguous() {
        assert_eq!(range(Some("bytes=2-4"), 10).unwrap(), (2, 4));
        assert_eq!(range(Some("bytes=5-"), 10).unwrap(), (5, 9));
        for r in [
            "bytes=-4",
            "bytes=0-999",
            "bytes=8-2",
            "bytes=0-1,4-5",
            "bytes=0-18446744073709551616",
        ] {
            assert!(range(Some(r), 10).is_err());
        }
    }
    #[test]
    fn recipient_revision_expiry_and_root_revocation_are_enforced() {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("omniget-artifacts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        policy::TEST_DIR.with(|p| *p.borrow_mut() = Some(dir.join("policy")));
        let root = dir.join("files");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("result.txt");
        std::fs::write(&file, b"original").unwrap();
        let a = policy::create("A".into(), vec!["transfer".into()]).unwrap();
        let b = policy::create("B".into(), vec!["transfer".into()]).unwrap();
        assert!(register(&a.principal, &file).is_err());
        let rid = grant_root(&a.principal.id, &root).unwrap();
        let grant = register(&a.principal, &file.canonicalize().unwrap()).unwrap();
        assert!(open(&b.principal, &grant.artifact_id, &grant.digest).is_err());
        assert!(open(&a.principal, &grant.artifact_id, "wrong").is_err());
        let mut snap = open(&a.principal, &grant.artifact_id, &grant.digest).unwrap();
        std::fs::write(&file, b"modified").unwrap();
        assert!(open(&a.principal, &grant.artifact_id, &grant.digest).is_err());
        use std::io::Read;
        let mut received = String::new();
        snap.file.read_to_string(&mut received).unwrap();
        assert_eq!(received, "original");
        std::fs::write(&file, b"original").unwrap();
        db().unwrap()
            .execute(
                "UPDATE artifact_grants SET expires=0 WHERE id=?1",
                [&grant.artifact_id],
            )
            .unwrap();
        assert!(authorized(&a.principal, &grant.artifact_id).is_err());
        let grant = register(&a.principal, &file.canonicalize().unwrap()).unwrap();
        revoke_root(&rid).unwrap();
        assert!(authorized(&a.principal, &grant.artifact_id).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
