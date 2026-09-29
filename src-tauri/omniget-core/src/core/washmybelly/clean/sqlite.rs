//! SQLite vacuum and log truncation, done the careful way.
//!
//! A database is compacted through SQLite itself (rusqlite, bundled): a
//! `wal_checkpoint(TRUNCATE)` first, then `VACUUM`. If another program holds
//! the database, SQLite says so and we stop with a reason. The `-wal` and
//! `-shm` files are never touched by hand: deleting a WAL with the database
//! open can lose committed data (a bug the studies found upstream).

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use super::super::fsops;

/// How long a vacuum waits for another program's lock before it gives up.
/// Unit tests hold the lock on purpose, so they wait less.
const BUSY_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_millis(500)
};

const MAGIC: &[u8; 16] = b"SQLite format 3\0";

/// Bytes a `VACUUM` would give back, from the header: free pages × page
/// size. `None` when the file is not a SQLite database.
pub fn estimate(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut head = [0u8; 100];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..16] != MAGIC {
        return None;
    }
    let page = match u16::from_be_bytes([head[16], head[17]]) {
        1 => 65_536u64,
        n => u64::from(n),
    };
    let free = u64::from(u32::from_be_bytes([head[36], head[37], head[38], head[39]]));
    Some(free * page)
}

fn is_side_file(path: &Path) -> bool {
    let n = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    n.ends_with("-wal") || n.ends_with("-shm") || n.ends_with("-journal")
}

fn size_with_wal(path: &Path) -> u64 {
    let mut total = 0;
    for suffix in ["", "-wal"] {
        let p = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if let Ok(m) = std::fs::symlink_metadata(&p) {
            total += fsops::allocated(&m);
        }
    }
    total
}

fn locked(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Checkpoint and vacuum. Returns the bytes given back (database + WAL).
pub fn vacuum(path: &Path) -> Result<u64, String> {
    if is_side_file(path) {
        return Err(
            "PROTECTED: a -wal, -shm or -journal file is never vacuumed or removed by hand".into(),
        );
    }
    if estimate(path).is_none() {
        return Err(format!("{}: not a SQLite database", path.display()));
    }
    let before = size_with_wal(path);
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    db.busy_timeout(BUSY_WAIT).map_err(|e| e.to_string())?;
    let busy = || {
        format!(
            "LOCKED: {} is open in another program; close it and try again",
            path.display()
        )
    };
    let (blocked, _, _): (i64, i64, i64) = db
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(|e| if locked(&e) { busy() } else { e.to_string() })?;
    if blocked != 0 {
        return Err(busy());
    }
    db.execute_batch("VACUUM")
        .map_err(|e| if locked(&e) { busy() } else { e.to_string() })?;
    drop(db);
    Ok(before.saturating_sub(size_with_wal(path)))
}

/// Truncate a log file to zero bytes; the app keeps its handle.
pub fn truncate(path: &Path) -> Result<u64, String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() || fsops::is_link(&m) {
        return Err("only a regular file is truncated".into());
    }
    if is_side_file(path) {
        return Err("PROTECTED: a database side file is never truncated".into());
    }
    let before = fsops::allocated(&m);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.set_len(0))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bloated(path: &Path) {
        let db = Connection::open(path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t(x BLOB);")
            .unwrap();
        for _ in 0..200 {
            db.execute("INSERT INTO t VALUES (zeroblob(4096))", [])
                .unwrap();
        }
        db.execute_batch("DELETE FROM t; PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }

    #[test]
    fn vacuum_gives_space_back_and_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("History");
        bloated(&p);
        {
            let db = Connection::open(&p).unwrap();
            db.execute("INSERT INTO t VALUES (x'01')", []).unwrap();
        }
        assert!(estimate(&p).unwrap() > 100_000, "{:?}", estimate(&p));
        let freed = vacuum(&p).unwrap();
        assert!(freed > 100_000, "{freed}");
        let n: i64 = Connection::open(&p)
            .unwrap()
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn an_open_writer_blocks_the_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cookies");
        bloated(&p);
        let holder = Connection::open(&p).unwrap();
        holder
            .execute_batch("BEGIN EXCLUSIVE; INSERT INTO t VALUES (x'02');")
            .unwrap();
        let e = vacuum(&p).unwrap_err();
        assert!(e.starts_with("LOCKED"), "{e}");
        holder.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn side_files_and_non_databases_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let wal = dir.path().join("db-wal");
        std::fs::write(&wal, b"x").unwrap();
        assert!(vacuum(&wal).is_err());
        assert!(truncate(&wal).is_err());
        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, b"hello").unwrap();
        assert!(vacuum(&txt).is_err());
        assert!(estimate(&txt).is_none());
    }

    #[test]
    fn truncate_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("app.log");
        std::fs::write(&p, vec![1u8; 20_000]).unwrap();
        assert!(truncate(&p).unwrap() > 0);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
    }
}
