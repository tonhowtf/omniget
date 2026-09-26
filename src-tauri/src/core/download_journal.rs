//! Persistent, bounded diagnostic journal. Only sanitized text crosses the
//! worker boundary; a full channel is reported as an evidence gap.
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, OnceLock,
};

const MAX_BYTES: i64 = 100 * 1024 * 1024;
// Pending counters are deliberately bounded atomics: producers never wait on disk.
static DROPPED: AtomicU64 = AtomicU64::new(0);
static TRUNCATED: AtomicU64 = AtomicU64::new(0);
static WORKER: OnceLock<mpsc::SyncSender<Command>> = OnceLock::new();

enum Command {
    Begin(u64),
    Diagnose(u64, mpsc::Sender<Result<Page, String>>),
    Event(u64, String),
    Read(u64, u64, usize, usize, mpsc::Sender<Result<Page, String>>),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub event_id: u64,
    pub timestamp: i64,
    pub download_id: u64,
    pub attempt_id: String,
    pub phase: String,
    pub level: String,
    pub message: String,
    pub external_content: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub events: Vec<Event>,
    pub next_cursor: u64,
    pub has_more: bool,
    pub dropped: u64,
    pub oldest_available: Option<u64>,
    pub truncated: u64,
    /// Counts are lower bounds after an uncheckpointed process termination.
    pub evidence_incomplete: bool,
    pub accounting_pending: bool,
    pub prior_session_gaps: u64,
    pub accounting_note: &'static str,
}

fn open(path: &Path) -> Result<Connection, String> {
    let db = Connection::open(path).map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
      CREATE TABLE IF NOT EXISTS attempts(download INTEGER PRIMARY KEY, attempt TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp INTEGER NOT NULL,
        download INTEGER NOT NULL, attempt TEXT NOT NULL, phase TEXT NOT NULL, level TEXT NOT NULL, message TEXT NOT NULL);
      CREATE INDEX IF NOT EXISTS events_download ON events(download,id);
      CREATE TABLE IF NOT EXISTS journal_health(singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        dropped INTEGER NOT NULL DEFAULT 0, truncated INTEGER NOT NULL DEFAULT 0,
        prior_session_gaps INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 0);
      INSERT OR IGNORE INTO journal_health(singleton) VALUES(1);")
      .map_err(|e| e.to_string())?;
    Ok(db)
}
pub fn init() -> Result<(), String> {
    if WORKER.get().is_some() {
        return Ok(());
    }
    let dir = omniget_core::core::tools::tools_dir()
        .ok_or("no data directory")?
        .join("download-journal");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let db = open(&dir.join("events.sqlite3"))?;
    // A durable marker precedes accepting events. Without a shutdown hook we
    // conservatively retain it: the next run cannot prove its tail was flushed.
    start_session(&db)?;
    let (tx, rx) = mpsc::sync_channel(2048);
    if WORKER.set(tx).is_err() {
        return Ok(());
    }
    std::thread::spawn(move || {
        loop {
            // Flush even if the last event was rejected and no further traffic
            // arrives. Failure leaves the counters in RAM, never silently reset.
            let _ = checkpoint(&db, &DROPPED, &TRUNCATED);
            let command = match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let result = match command {
                Command::Begin(id) => begin(&db, id),
                Command::Diagnose(id, reply) => {
                    let result = latest(&db, id);
                    let _ = reply.send(result);
                    Ok(())
                }
                Command::Event(id, line) => append(&db, id, &line),
                Command::Read(id, cursor, limit, budget, reply) => {
                    let _ = reply.send(read(&db, id, cursor, limit, budget));
                    Ok(())
                }
            };
            if result.is_err() {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
        }
    });
    Ok(())
}
fn send(command: Command) {
    if WORKER.get().is_none_or(|tx| tx.try_send(command).is_err()) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn begin_attempt(id: u64) {
    send(Command::Begin(id));
}
pub fn record(id: u64, line: &str) {
    let safe = super::flight_recorder::redact(line);
    let safe = bounded_line(&safe, &TRUNCATED);
    send(Command::Event(id, safe));
}
fn bounded_line(line: &str, truncated: &AtomicU64) -> String {
    let mut chars = line.chars();
    let mut safe: String = chars.by_ref().take(8192).collect();
    if chars.next().is_some() {
        truncated.fetch_add(1, Ordering::Relaxed);
        safe.push_str(" [truncated at journal ingress]");
    }
    safe
}
fn start_session(db: &Connection) -> Result<(), String> {
    db.execute("UPDATE journal_health SET prior_session_gaps=prior_session_gaps+active, active=1 WHERE singleton=1", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn checkpoint(db: &Connection, dropped: &AtomicU64, truncated: &AtomicU64) -> Result<(), String> {
    let losses = dropped.load(Ordering::Relaxed);
    let cuts = truncated.load(Ordering::Relaxed);
    if losses == 0 && cuts == 0 {
        return Ok(());
    }
    // Single SQLite statement is atomic; counters are subtracted only after its
    // durable commit. Concurrent producer increments remain for the next pass.
    db.execute(
        "UPDATE journal_health SET dropped=dropped+?1, truncated=truncated+?2 WHERE singleton=1",
        params![losses, cuts],
    )
    .map_err(|e| e.to_string())?;
    dropped.fetch_sub(losses, Ordering::Relaxed);
    truncated.fetch_sub(cuts, Ordering::Relaxed);
    Ok(())
}
fn health(db: &Connection) -> Result<(u64, u64, u64, bool), String> {
    let (dropped, truncated, gaps): (u64, u64, u64) = db
        .query_row(
            "SELECT dropped,truncated,prior_session_gaps FROM journal_health WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let pending_dropped = DROPPED.load(Ordering::Relaxed);
    let pending_truncated = TRUNCATED.load(Ordering::Relaxed);
    Ok((
        dropped.saturating_add(pending_dropped),
        truncated.saturating_add(pending_truncated),
        gaps,
        pending_dropped != 0 || pending_truncated != 0,
    ))
}
const ACCOUNTING_NOTE: &str = "Global journal counters are lower bounds. Producers never wait for disk; pending accounting can be lost on process termination or disk failure. Prior-session gaps conservatively include shutdowns without a proven final checkpoint. Page-budget truncation is marked inline and does not delete stored text.";
fn begin(db: &Connection, id: u64) -> Result<(), String> {
    db.execute(
        "INSERT OR REPLACE INTO attempts VALUES(?1, ?2)",
        params![id, uuid::Uuid::new_v4().to_string()],
    )
    .map_err(|e| e.to_string())?;
    append(db, id, "Attempt started")
}
fn append(db: &Connection, id: u64, line: &str) -> Result<(), String> {
    db.execute(
        "INSERT OR IGNORE INTO attempts VALUES(?1,?2)",
        params![id, uuid::Uuid::new_v4().to_string()],
    )
    .map_err(|e| e.to_string())?;
    let lower = line.to_ascii_lowercase();
    let level = if lower.contains("error:") || lower.contains("error ") || lower.contains(" failed")
    {
        "error"
    } else if lower.starts_with("warning:") || lower.contains("] warning:") {
        "warning"
    } else {
        "info"
    };
    let phase = if lower.contains("finalization") || lower.contains("download finished") {
        "finalization"
    } else if lower.contains("fetching video info") || lower.contains("extract") {
        "extraction"
    } else if lower.contains("ffmpeg") || lower.contains("merg") || lower.contains("mux") {
        "postprocessing"
    } else if lower.contains("network") || lower.contains("proxy") {
        "network"
    } else {
        "engine"
    };
    db.execute(
        "INSERT INTO events(timestamp,download,attempt,phase,level,message)
      SELECT unixepoch(), ?1, attempt, ?4, ?2, ?3 FROM attempts WHERE download=?1",
        params![id, level, line, phase],
    )
    .map_err(|e| e.to_string())?;
    // Sample progress is bounded at the source; amortize retention scans.
    if db.last_insert_rowid() % 128 != 0 {
        return Ok(());
    }
    // Logical retention is enforced continuously; SQLite reuses deleted pages.
    db.execute(
        "DELETE FROM events WHERE timestamp < unixepoch()-1209600",
        [],
    )
    .map_err(|e| e.to_string())?;
    let size: i64 = db
        .query_row(
            "SELECT coalesce(sum(length(message)+128),0) FROM events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if size > MAX_BYTES {
        db.execute(
            "DELETE FROM events WHERE id IN (SELECT id FROM events ORDER BY id LIMIT 1024)",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn read(
    db: &Connection,
    id: u64,
    cursor: u64,
    limit: usize,
    budget: usize,
) -> Result<Page, String> {
    let oldest: Option<u64> = db
        .query_row("SELECT min(id) FROM events WHERE download=?1", [id], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let mut stmt = db.prepare("SELECT id,timestamp,download,attempt,phase,level,message FROM events WHERE download=?1 AND id>?2 ORDER BY id LIMIT ?3").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(params![id, cursor, limit.clamp(1, 200) + 1], |r| {
            Ok(Event {
                event_id: r.get(0)?,
                timestamp: r.get(1)?,
                download_id: r.get(2)?,
                attempt_id: r.get(3)?,
                phase: r.get(4)?,
                level: r.get(5)?,
                message: r.get(6)?,
                external_content: true,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    let mut used = 0;
    let mut has_more = false;
    for row in rows {
        let mut event = row.map_err(|e| e.to_string())?;
        let mut bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?.len();
        if bytes > budget.clamp(16384, 65536) {
            event.message = format!(
                "{} [truncated to page budget]",
                event.message.chars().take(2000).collect::<String>()
            );
            bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?.len();
        }
        if events.len() >= limit.clamp(1, 200) || used + bytes > budget.clamp(16384, 65536) {
            has_more = true;
            break;
        }
        used += bytes;
        events.push(event);
    }
    let next_cursor = events.last().map(|e| e.event_id).unwrap_or(cursor);
    let (dropped, truncated, prior_session_gaps, accounting_pending) = health(db)?;
    Ok(Page {
        events,
        next_cursor,
        has_more,
        dropped,
        truncated,
        prior_session_gaps,
        accounting_pending,
        evidence_incomplete: dropped > 0
            || truncated > 0
            || prior_session_gaps > 0
            || accounting_pending,
        accounting_note: ACCOUNTING_NOTE,
        oldest_available: oldest,
    })
}
fn latest(db: &Connection, id: u64) -> Result<Page, String> {
    // Preserve the first error even if thousands of progress lines follow it.
    // The ordinary log endpoint remains chronological and fully pageable.
    let mut stmt = db.prepare("SELECT id,timestamp,download,attempt,phase,level,message FROM events WHERE download=?1 AND attempt=(SELECT attempt FROM attempts WHERE download=?1) ORDER BY CASE level WHEN 'error' THEN 0 WHEN 'warning' THEN 1 ELSE 2 END, id LIMIT 201").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([id], |r| {
            Ok(Event {
                event_id: r.get(0)?,
                timestamp: r.get(1)?,
                download_id: r.get(2)?,
                attempt_id: r.get(3)?,
                phase: r.get(4)?,
                level: r.get(5)?,
                message: r.get(6)?,
                external_content: true,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut events = Vec::new();
    let mut bytes = 0;
    let mut has_more = false;
    for row in rows {
        let mut event = row.map_err(|e| e.to_string())?;
        let mut event_bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?.len();
        if event_bytes > 32768 {
            event.message = format!(
                "{} [truncated to diagnosis budget]",
                event.message.chars().take(2000).collect::<String>()
            );
            event_bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?.len();
        }
        bytes += event_bytes;
        if bytes > 32768 || events.len() >= 200 {
            has_more = true;
            break;
        }
        events.push(event);
    }
    let oldest = events.iter().map(|e| e.event_id).min();
    let (dropped, truncated, prior_session_gaps, accounting_pending) = health(db)?;
    Ok(Page {
        events,
        next_cursor: 0,
        has_more,
        dropped,
        truncated,
        prior_session_gaps,
        accounting_pending,
        evidence_incomplete: dropped > 0
            || truncated > 0
            || prior_session_gaps > 0
            || accounting_pending,
        accounting_note: ACCOUNTING_NOTE,
        oldest_available: oldest,
    })
}
pub fn diagnosis_page(id: u64) -> Result<Page, String> {
    let (tx, rx) = mpsc::channel();
    WORKER
        .get()
        .ok_or("JOURNAL_UNAVAILABLE")?
        .try_send(Command::Diagnose(id, tx))
        .map_err(|_| "JOURNAL_BUSY")?;
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "JOURNAL_TIMEOUT")?
}

pub fn page(id: u64, cursor: u64, limit: usize, budget: usize) -> Result<Page, String> {
    let (tx, rx) = mpsc::channel();
    WORKER
        .get()
        .ok_or("JOURNAL_UNAVAILABLE")?
        .try_send(Command::Read(id, cursor, limit, budget, tx))
        .map_err(|_| "JOURNAL_BUSY")?;
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "JOURNAL_TIMEOUT")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounting_survives_disk_full_recovery_and_reopen() {
        let path =
            std::env::temp_dir().join(format!("omniget-loss-{}.sqlite", uuid::Uuid::new_v4()));
        let dropped = AtomicU64::new(3);
        let truncated = AtomicU64::new(2);
        {
            let db = open(&path).unwrap();
            start_session(&db).unwrap();
            db.execute_batch(
                "CREATE TABLE fault_fill(data BLOB);
                CREATE TRIGGER fault_full BEFORE UPDATE ON journal_health BEGIN
                INSERT INTO fault_fill VALUES(zeroblob(1048576)); END;",
            )
            .unwrap();
            let pages: u64 = db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
            db.execute_batch(&format!("PRAGMA max_page_count={pages};"))
                .unwrap();
            let error = checkpoint(&db, &dropped, &truncated).unwrap_err();
            assert!(error.contains("full"), "{error}");
            assert_eq!(dropped.load(Ordering::Relaxed), 3);
            assert_eq!(truncated.load(Ordering::Relaxed), 2);
            let persisted: u64 = db
                .query_row("SELECT dropped FROM journal_health", [], |r| r.get(0))
                .unwrap();
            assert_eq!(persisted, 0);
            db.execute_batch("DROP TRIGGER fault_full;").unwrap();
            checkpoint(&db, &dropped, &truncated).unwrap();
            checkpoint(&db, &dropped, &truncated).unwrap(); // no double counting
            assert_eq!(dropped.load(Ordering::Relaxed), 0);
        }
        let db = open(&path).unwrap();
        start_session(&db).unwrap();
        let (losses, cuts, gaps, _) = health(&db).unwrap();
        assert_eq!((losses, cuts, gaps), (3, 2, 1));
        assert!(read(&db, 7, 0, 10, 32768).unwrap().evidence_incomplete);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn uncheckpointed_loss_is_reported_as_unknown_tail_after_restart() {
        let db = open(Path::new(":memory:")).unwrap();
        start_session(&db).unwrap();
        // Simulate a crash with losses still in RAM: their exact count is gone.
        start_session(&db).unwrap();
        let (losses, cuts, gaps, _) = health(&db).unwrap();
        assert_eq!((losses, cuts, gaps), (0, 0, 1));
        assert!(read(&db, 7, 0, 10, 32768).unwrap().evidence_incomplete);
    }
    #[test]
    fn oversized_first_error_remains_available_to_diagnosis() {
        let db = open(Path::new(":memory:")).unwrap();
        begin(&db, 7).unwrap();
        append(&db, 7, &format!("error: {}", "😀".repeat(8192))).unwrap();
        let page = latest(&db, 7).unwrap();
        assert_eq!(page.events[0].level, "error");
        assert!(page.events[0]
            .message
            .contains("truncated to diagnosis budget"));
    }
    #[test]
    fn ingress_truncation_is_unicode_safe_and_counted_once() {
        let counter = AtomicU64::new(0);
        assert_eq!(bounded_line("short", &counter), "short");
        let line = bounded_line(&"😀".repeat(8193), &counter);
        assert!(line.ends_with("[truncated at journal ingress]"));
        assert_eq!(line.matches('😀').count(), 8192);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn attempts_survive_reopening_and_pages_advance() {
        let path =
            std::env::temp_dir().join(format!("omniget-journal-{}.sqlite", uuid::Uuid::new_v4()));
        {
            let db = open(&path).unwrap();
            begin(&db, 7).unwrap();
            append(&db, 7, "first failure").unwrap();
            begin(&db, 7).unwrap();
            append(&db, 7, "second failure").unwrap();
        }
        let db = open(&path).unwrap();
        let a = read(&db, 7, 0, 2, 32768).unwrap();
        assert!(a.has_more);
        let b = read(&db, 7, a.next_cursor, 2, 32768).unwrap();
        assert_ne!(a.events[0].attempt_id, b.events[0].attempt_id);
        assert_eq!(read(&db, 8, 0, 100, 32768).unwrap().events.len(), 0);
        drop(db);
        let _ = std::fs::remove_file(path);
    }
    #[test]
    fn diagnosis_keeps_first_error_through_progress_flood_and_only_current_attempt() {
        let db = open(Path::new(":memory:")).unwrap();
        begin(&db, 7).unwrap();
        append(&db, 7, "error: old credentials").unwrap();
        begin(&db, 7).unwrap();
        for _ in 0..240 {
            append(&db, 7, "progress").unwrap();
        }
        append(&db, 7, "error: No space left on device").unwrap();
        for _ in 0..240 {
            append(&db, 7, "progress").unwrap();
        }
        let page = latest(&db, 7).unwrap();
        assert!(page.has_more);
        assert!(page.events[0].message.contains("No space"));
        assert!(!page
            .events
            .iter()
            .any(|e| e.message.contains("old credentials")));
    }
    #[test]
    fn oversized_unicode_line_cannot_stall_a_cursor() {
        let db = open(Path::new(":memory:")).unwrap();
        begin(&db, 7).unwrap();
        append(&db, 7, &"😀".repeat(8192)).unwrap();
        let page = read(&db, 7, 1, 1, 16384).unwrap();
        assert_eq!(page.events.len(), 1);
        assert!(page.next_cursor > 1);
        assert!(page.events[0].message.contains("truncated"));
    }
}
