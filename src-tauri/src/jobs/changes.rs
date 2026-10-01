//! `job_changes` (3.4, S19, `85#P4`): the files a job wrote through the
//! CLI's own file tools, each with its content before the first touch, from
//! the stream's `originalFile` (SP3, [`omniget_core::core::llm::file_changes`]).
//!
//! What it buys: the diff of a job in a shared folder no longer depends on
//! the shadow snapshots being there. When the snapshot diff is unavailable
//! (no git, the snapshot failed, a job recorded before snapshots), the diff
//! is rebuilt from these baselines and the files as they are now. Changes
//! made through the shell are not in this table (the snapshot remains the
//! first source when it exists).

use rusqlite::{params, Connection};

use omniget_core::core::llm::file_changes::{self, FileTouch};
use omniget_core::core::llm::snapshot::{FileChange, JobDiff};

use super::{Jobs, ERR_JOBS};

pub(super) fn ensure_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS job_changes (
            job_id TEXT NOT NULL, path TEXT NOT NULL, op TEXT NOT NULL,
            baseline BLOB, too_big INTEGER NOT NULL DEFAULT 0,
            first_ms INTEGER NOT NULL,
            PRIMARY KEY (job_id, path));",
    )
    .map_err(|e| format!("{ERR_JOBS}: {e}"))
}

/// One row: the first touch of `path` by the job.
#[derive(Debug, Clone, PartialEq)]
pub struct JobChange {
    pub path: String,
    pub op: String,
    pub baseline: Option<String>,
    pub too_big: bool,
}

/// Files the touch under `job` (first touch per path wins: that one has the
/// content from before the job).
pub(super) fn insert(db: &Connection, job: &str, touch: &FileTouch, now: i64) {
    let _ = db.execute(
        "INSERT OR IGNORE INTO job_changes (job_id, path, op, baseline, too_big, first_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            job,
            touch.path,
            touch.op,
            touch.original,
            i64::from(touch.too_big),
            now
        ],
    );
}

pub(super) fn of_job(db: &Connection, job: &str) -> Vec<JobChange> {
    let Ok(mut stmt) = db.prepare(
        "SELECT path, op, baseline, too_big FROM job_changes WHERE job_id=?1 ORDER BY first_ms, path",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![job], |r| {
        Ok(JobChange {
            path: r.get(0)?,
            op: r.get(1)?,
            baseline: r.get(2)?,
            too_big: r.get::<_, i64>(3)? != 0,
        })
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

pub(super) fn forget(db: &Connection, job: &str) {
    let _ = db.execute("DELETE FROM job_changes WHERE job_id=?1", params![job]);
}

/// The job's diff from its baselines and the files as they are now. Paths
/// under `folder` are shown relative to it. A file whose baseline was too
/// big is listed without lines.
pub fn diff_from_baselines(folder: Option<&std::path::Path>, rows: &[JobChange]) -> JobDiff {
    let mut out = JobDiff {
        complete: true,
        ..JobDiff::default()
    };
    for row in rows {
        let abs = std::path::Path::new(&row.path);
        let rel = folder
            .and_then(|f| abs.strip_prefix(f).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| row.path.trim_start_matches('/').to_string());
        let now = file_changes::current(abs);
        if row.too_big {
            out.files.push(FileChange {
                path: rel,
                status: if now.is_some() { "M" } else { "D" }.into(),
                additions: None,
                deletions: None,
                binary: false,
            });
            continue;
        }
        let before = row.baseline.as_deref();
        let (patch, adds, dels) = file_changes::patch(&rel, before, now.as_deref());
        if patch.is_empty() {
            continue;
        }
        let status = match (before, now.is_some()) {
            (None, true) => "A",
            (_, false) => "D",
            _ => "M",
        };
        out.files.push(FileChange {
            path: rel,
            status: status.into(),
            additions: Some(adds),
            deletions: Some(dels),
            binary: false,
        });
        out.additions += adds;
        out.deletions += dels;
        out.patch.push_str(&patch);
    }
    out
}

impl Jobs {
    /// The sink of [`file_changes::record`]: the touch goes to the job whose
    /// turn it is (`request`); a turn that is no job's is dropped.
    pub(super) fn record_change(&self, request: &str, touch: FileTouch) {
        let Some(job) = self.job_of_request(request) else {
            return;
        };
        insert(
            &self.db(),
            &job,
            &touch,
            crate::llm_manager::now_ms() as i64,
        );
    }

    /// What the job wrote through the CLI's file tools, with the baselines.
    pub fn changes(&self, id: &str) -> Vec<JobChange> {
        of_job(&self.db(), id)
    }
}
