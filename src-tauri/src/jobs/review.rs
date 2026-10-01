//! Review of a job's changes (roadmap 3.5, S20): hunks with ids and the
//! discard of one, notes that go back to the agent as a follow-up turn in
//! the same conversation (and worktree), the integration of a worktree
//! job's branch into the user's checkout, discarding that work, and the
//! walkthrough on demand. The git and prompt work is in
//! `omniget_core::core::llm::review` and `omniget_core::core::vcs::integrate`;
//! this is the job side: which folder, which lock, what the row records.
//!
//! Every write here refuses while a job writes in an overlapping folder
//! (`ERR_REVIEW_BUSY`), and runs under the review lock of the folder it
//! touches.

use super::*;
use omniget_core::core::llm::review::{self, Note, Target, Walkthrough, ERR_REVIEW_BUSY};
use omniget_core::core::llm::snapshot;
use omniget_core::core::vcs::diff::{parse_hunks, split_patch};
use omniget_core::core::vcs::{integrate, job_worktree};

/// Longest note body (a review note, not a document).
pub const NOTE_MAX_BYTES: usize = 8 * 1024;

pub(super) fn ensure_notes_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS job_notes (
            id TEXT PRIMARY KEY, job_id TEXT NOT NULL, path TEXT NOT NULL,
            start_line INTEGER, end_line INTEGER, body TEXT NOT NULL,
            created_ms INTEGER NOT NULL, sent_ms INTEGER, sent_job TEXT, head_oid TEXT);
         CREATE INDEX IF NOT EXISTS job_notes_job ON job_notes(job_id, created_ms);",
    )
    .map_err(|e| format!("{ERR_JOBS}: {e}"))
}

fn row_to_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: r.get(0)?,
        job_id: r.get(1)?,
        path: r.get(2)?,
        start_line: r.get::<_, Option<i64>>(3)?.map(|v| v as u32),
        end_line: r.get::<_, Option<i64>>(4)?.map(|v| v as u32),
        body: r.get(5)?,
        created_ms: r.get::<_, i64>(6)? as u64,
        sent_ms: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        sent_job: r.get(8)?,
        head_oid: r.get(9)?,
    })
}

/// The patch file next to `jobs.db`: written when there is a patch, removed
/// when there is none (as `record_diff` does).
fn store_patch(job_id: &str, patch: Option<&str>) {
    let Some(path) = patch_path(job_id) else {
        return;
    };
    match patch.filter(|p| !p.is_empty()) {
        Some(text) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, text) {
                tracing::warn!("[jobs] patch of {job_id}: {e}");
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The job id that owns the worktree of `job` (its branch names it): a
/// follow-up job works in the worktree of the job it follows.
pub(super) fn worktree_owner(job: &Job) -> &str {
    job.branch
        .as_deref()
        .and_then(|b| b.strip_prefix(job_worktree::branch_of("").as_str()))
        .filter(|id| !id.is_empty())
        .unwrap_or(&job.id)
}

/// The first line of the job's prompt, the title of its commits.
fn title_of(job: &Job) -> String {
    let first: String = job
        .prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(72)
        .collect();
    if first.is_empty() {
        format!("omniget job {}", job.id)
    } else {
        first
    }
}

fn short(s: &str) -> String {
    s.chars().take(7).collect()
}

impl Jobs {
    fn job_or_err(&self, id: &str) -> Result<Job, String> {
        self.job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))
    }

    /// Refuses while this job runs or any job writes in a folder that
    /// overlaps `folder` (the undo guard's rule).
    fn ensure_quiet(&self, job: &Job, folder: &std::path::Path) -> Result<(), String> {
        if is_active(&job.state) {
            return Err(format!(
                "{ERR_REVIEW_BUSY}: job {} is {}; wait for it to end",
                job.id, job.state
            ));
        }
        let live = self.live_folders("");
        if let Some(other) = snapshot::shared_with(folder, &live) {
            return Err(format!(
                "{ERR_REVIEW_BUSY}: a job is writing in {} right now; try again when it ends",
                other.display()
            ));
        }
        Ok(())
    }

    /// Where the job's changes are and what they are measured from.
    async fn review_target(&self, job: &Job) -> Result<Target, String> {
        if job.worktree_path.is_some() {
            let base_oid = job
                .base_oid
                .clone()
                .ok_or_else(|| format!("{ERR_WORKTREE}: job {} has no base commit", job.id))?;
            let (_, root) = worktree_root(job)
                .await
                .ok_or_else(|| format!("{ERR_WORKTREE}: the job's repository is gone"))?;
            return Ok(Target::Worktree { root, base_oid });
        }
        if let Some(branch) = &job.branch {
            return Err(format!(
                "{ERR_WORKTREE}: the worktree of job {} was removed; its work is on branch {branch} (integrate or discard it)",
                job.id
            ));
        }
        let ws = self
            .diff_folder(job)
            .ok_or_else(|| format!("{NO_WORKSPACE}: the job ran without a folder"))?;
        let requests: Vec<String> = job.request_id.clone().into_iter().collect();
        let d = snapshot::job_diff(&ws, &sanitize_id(&job.conversation_id), &requests).await?;
        let base_tree = d
            .base
            .ok_or_else(|| format!("{}: no base tree", snapshot::ERR_DIFF_UNAVAILABLE))?;
        Ok(Target::Shared {
            workspace: ws,
            base_tree,
        })
    }

    /// The main checkout of a worktree job's repository.
    async fn repo_of(&self, job: &Job) -> Result<std::path::PathBuf, String> {
        if let Some((repo, _)) = worktree_root(job).await {
            return Ok(repo);
        }
        let ws = job
            .workspace
            .as_deref()
            .filter(|w| !w.trim().is_empty())
            .ok_or_else(|| format!("{ERR_WORKTREE}: job {} has no folder", job.id))?;
        job_worktree::repo_root(std::path::Path::new(ws))
            .await
            .ok_or_else(|| format!("{ERR_WORKTREE}: {ws} is not in a git repository any more"))
    }

    fn branch_of_job(&self, job: &Job) -> Result<String, String> {
        match job.branch.clone() {
            Some(b) if b.starts_with(&job_worktree::branch_of("")) => Ok(b),
            Some(b) => Err(format!(
                "{ERR_WORKTREE}: {b} is not an OmniGet job branch"
            )),
            None => Err(format!(
                "{ERR_WORKTREE}: job {} worked in your folder, not in a worktree: its changes are already there (discard hunks, or undo the turn)",
                job.id
            )),
        }
    }

    // ── hunks ────────────────────────────────────────────────────────

    /// The job's diff as hunks with ids, and the patch of each file (what
    /// the review panel draws). `actionable` = a hunk can be discarded now.
    pub async fn review_hunks(&self, id: &str) -> Result<Value, String> {
        let (job, patch, complete) = self.diff(id).await?;
        let patch = patch.unwrap_or_default();
        let files: Vec<Value> = split_patch(&patch)
            .into_iter()
            .filter_map(|chunk| {
                omniget_core::core::vcs::diff::parse_hunks(chunk)
                    .first()
                    .map(|h| h.path.clone())
                    .or_else(|| {
                        chunk
                            .lines()
                            .next()
                            .and_then(|l| l.strip_prefix("diff --git a/"))
                            .and_then(|r| r.split_once(" b/").map(|(a, _)| a.to_string()))
                    })
                    .map(|path| json!({ "path": path, "patch": chunk }))
            })
            .collect();
        let movable = job.worktree_path.is_some() || job.branch.is_none();
        Ok(json!({
            "job_id": job.id,
            "state": job.state,
            "complete": complete,
            "clipped": job.diff_clipped,
            "mode": if job.branch.is_some() { "worktree" } else { "shared" },
            "actionable": complete && movable && !is_active(&job.state),
            "hunks": parse_hunks(&patch),
            "files": files,
        }))
    }

    /// Discards one hunk (see `review::discard_hunk`) and records the diff
    /// as it is after it. A job left with nothing to review is `done`.
    pub async fn discard_hunk(
        &self,
        id: &str,
        hunk_id: &str,
        patch: &str,
    ) -> Result<Value, String> {
        let mut job = self.job_or_err(id)?;
        let target = self.review_target(&job).await?;
        self.ensure_quiet(&job, target.folder())?;
        let out = review::discard_hunk(&target, hunk_id, patch).await?;
        let mut diff = out.diff.clone();
        diff.patch = out.patch.clone();
        if matches!(target, Target::Shared { .. }) {
            // The folder may hold later changes of others: the job keeps
            // showing its own files.
            let own: Vec<String> = job
                .diff_files
                .iter()
                .flatten()
                .map(|f| f.path.clone())
                .collect();
            diff = review::restrict_to_paths(diff, &own);
        }
        let head = job.head_oid.clone();
        let text = apply_diff(&mut job, Ok(diff));
        if job.head_oid.is_none() {
            job.head_oid = head;
        }
        store_patch(&job.id, text.as_deref());
        job.log.push_str(&format!(
            "✂ hunk {hunk_id} discarded (+{} −{})\n",
            out.hunk.additions, out.hunk.deletions
        ));
        if job.state == "review" && job.diff_files.as_ref().is_some_and(|f| f.is_empty()) {
            job.state = "done".into();
            job.log.push_str("✓ nothing left to review\n");
        }
        job.log = clip(&job.log, LOG_MAX);
        self.save_job_checked(&job)?;
        Ok(json!({ "hunk": out.hunk, "job": job }))
    }

    // ── notes ────────────────────────────────────────────────────────

    pub fn notes(&self, id: &str) -> Vec<Note> {
        let db = self.db();
        let Ok(mut stmt) = db.prepare(
            "SELECT id, job_id, path, start_line, end_line, body, created_ms, sent_ms, sent_job, head_oid FROM job_notes WHERE job_id=?1 ORDER BY created_ms, id",
        ) else {
            return Vec::new();
        };
        stmt.query_map(params![id], row_to_note)
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn note_add(
        &self,
        id: &str,
        path: &str,
        start_line: Option<u32>,
        end_line: Option<u32>,
        body: &str,
    ) -> Result<Note, String> {
        let job = self.job_or_err(id)?;
        let (path, body) = (path.trim(), body.trim());
        if path.is_empty() || path.contains('\n') {
            return Err(format!("{ERR_JOBS}: a note needs the file it is about"));
        }
        if body.is_empty() {
            return Err(format!("{ERR_JOBS}: empty note"));
        }
        if body.len() > NOTE_MAX_BYTES {
            return Err(format!(
                "{ERR_JOBS}: a note is at most {} KB",
                NOTE_MAX_BYTES / 1024
            ));
        }
        let (start_line, end_line) = match (start_line, end_line) {
            (Some(0), _) | (_, Some(0)) => {
                return Err(format!("{ERR_JOBS}: lines start at 1"));
            }
            (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
            (a, b) => (a.or(b), a.or(b)),
        };
        let note = Note {
            id: short_id("n"),
            job_id: job.id.clone(),
            path: path.to_string(),
            start_line,
            end_line,
            body: body.to_string(),
            created_ms: now_ms(),
            sent_ms: None,
            sent_job: None,
            head_oid: job.head_oid.clone(),
        };
        self.db()
            .execute(
                "INSERT INTO job_notes (id, job_id, path, start_line, end_line, body, created_ms, head_oid) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    note.id,
                    note.job_id,
                    note.path,
                    note.start_line.map(i64::from),
                    note.end_line.map(i64::from),
                    note.body,
                    note.created_ms as i64,
                    note.head_oid
                ],
            )
            .map_err(|e| format!("{ERR_JOBS}: {e}"))?;
        Ok(note)
    }

    pub fn note_delete(&self, id: &str, note_id: &str) -> Result<(), String> {
        let n = self
            .db()
            .execute(
                "DELETE FROM job_notes WHERE id=?1 AND job_id=?2",
                params![note_id, id],
            )
            .map_err(|e| format!("{ERR_JOBS}: {e}"))?;
        if n == 0 {
            return Err(format!("{ERR_JOBS}: no note {note_id} on job {id}"));
        }
        Ok(())
    }

    /// Sends the notes not sent yet to the agent: a follow-up job in the
    /// same conversation (the CLI runtime resumes its session: the turn
    /// costs the notes, not the task) and the same folder or worktree. The
    /// job being reviewed hands its review to the follow-up (`done`).
    pub async fn notes_send(self: &Arc<Self>, id: &str) -> Result<Job, String> {
        let mut job = self.job_or_err(id)?;
        if is_active(&job.state) {
            return Err(format!(
                "{ERR_REVIEW_BUSY}: job {id} is {}; send the notes when it ends",
                job.state
            ));
        }
        if job.agent_id.starts_with(TOOL_PREFIX) {
            return Err(format!(
                "{ERR_JOBS}: a catalog tool run has no conversation to resume; start a new run with the notes"
            ));
        }
        let pending: Vec<Note> = self
            .notes(id)
            .into_iter()
            .filter(|n| n.sent_ms.is_none())
            .collect();
        if pending.is_empty() {
            return Err(format!("{ERR_JOBS}: no unsent note on job {id}"));
        }
        let busy: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM jobs WHERE conversation_id=?1 AND state IN ('queued','waiting_workspace','running','waiting_approval') LIMIT 1",
                params![job.conversation_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if let Some(other) = busy {
            return Err(format!(
                "{ERR_REVIEW_BUSY}: job {other} is already working in this conversation"
            ));
        }
        let prompt = review::follow_up_prompt(&pending);
        let mut next = self.new_job(
            "run",
            &job.agent_id,
            &prompt,
            job.workspace.clone(),
            Some(job.conversation_id.clone()),
        )?;
        next.deadline_min = job.deadline_min;
        next.isolation = job.isolation;
        next.base_ref = job.base_ref.clone();
        next.branch = job.branch.clone();
        next.base_oid = job.base_oid.clone();
        next.worktree_path = job.worktree_path.clone();
        if next.worktree_path.is_none() && next.branch.is_some() {
            // The worktree was removed: the follow-up recreates it from the
            // branch (`enter_worktree` → `ensure`), at the owner's path.
            let repo = self.repo_of(&job).await?;
            next.worktree_path = Some(
                job_worktree::path_of(&repo, worktree_owner(&job))
                    .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        next.log.push_str(&format!(
            "↩ {} review note(s) on job {}\n",
            pending.len(),
            job.id
        ));
        self.save_job_checked(&next)?;
        let sent = now_ms();
        for n in &pending {
            let _ = self.db().execute(
                "UPDATE job_notes SET sent_ms=?2, sent_job=?3 WHERE id=?1",
                params![n.id, sent as i64, next.id],
            );
        }
        if job.state == "review" {
            job.state = "done".into();
        }
        job.log.push_str(&format!(
            "↪ {} review note(s) sent to job {}\n",
            pending.len(),
            next.id
        ));
        job.log = clip(&job.log, LOG_MAX);
        self.save_job_checked(&job)?;
        let this = self.clone();
        let nid = next.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&nid, None).await;
        });
        Ok(next)
    }

    // ── integrate / discard ──────────────────────────────────────────

    /// What integrating the job's branch would do: the target branch, what
    /// blocks it, the commits it brings, and what is still uncommitted in
    /// the worktree (saved on the branch first).
    pub async fn integrate_preview(&self, id: &str) -> Result<Value, String> {
        let job = self.job_or_err(id)?;
        let branch = self.branch_of_job(&job)?;
        let repo = self.repo_of(&job).await?;
        let pre = integrate::preview(&repo, &branch)
            .await
            .map_err(|e| e.to_string())?;
        let worktree_dirty = match worktree_root(&job).await {
            Some((_, root)) if root.exists() => {
                job_worktree::dirty_files(&root).await.unwrap_or_default()
            }
            _ => Vec::new(),
        };
        Ok(json!({
            "job_id": job.id,
            "branch": branch,
            "repo": repo,
            "target_branch": pre.target_branch,
            "blocker": pre.blocker(),
            "commits": pre.commits,
            "up_to_date": pre.up_to_date && worktree_dirty.is_empty(),
            "dirty": pre.dirty,
            "in_progress": pre.in_progress,
            "worktree_dirty": worktree_dirty,
        }))
    }

    /// Merges the job's branch into the user's checkout (`merge --no-ff` or
    /// `--squash`), after saving what is left in the worktree (discarded
    /// hunks included) on the branch. On success the job is `done` and its
    /// worktree goes (its branch too once merged; after a squash the branch
    /// is deleted, its content being the new commit). A conflict is undone
    /// and reported; the job stays in review.
    pub async fn integrate(
        &self,
        id: &str,
        strategy: integrate::Strategy,
    ) -> Result<(integrate::Outcome, Job), String> {
        let mut job = self.job_or_err(id)?;
        let branch = self.branch_of_job(&job)?;
        let repo = self.repo_of(&job).await?;
        self.ensure_quiet(&job, &repo)?;
        let root = worktree_root(&job)
            .await
            .map(|(_, r)| r)
            .filter(|r| r.exists());
        if let Some(root) = &root {
            self.ensure_quiet(&job, root)?;
        }
        let _repo_lock = review::lock(&repo).await;
        let _wt_lock = match &root {
            Some(r) => Some(review::lock(r).await),
            None => None,
        };
        if let Some(root) = &root {
            match job_worktree::save_work(root, &format!("omniget job {}: review changes", job.id))
                .await
            {
                Ok(Some(c)) => job
                    .log
                    .push_str(&format!("⎇ review changes saved as {}\n", short(&c))),
                Ok(None) => {}
                Err(e) => {
                    return Err(format!(
                    "{ERR_WORKTREE}: could not save the worktree's changes before integrating: {e}"
                ))
                }
            }
        }
        let out = integrate::integrate(&repo, &branch, strategy, &title_of(&job))
            .await
            .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
        match out.status.as_str() {
            "merged" | "up_to_date" => {
                job.log.push_str(&match &out.commit {
                    Some(c) => format!(
                        "⇲ integrated into {} as {} ({})\n",
                        out.target_branch,
                        short(c),
                        if strategy == integrate::Strategy::Squash {
                            "squash"
                        } else {
                            "merge --no-ff"
                        }
                    ),
                    None => format!("⇲ already in {}\n", out.target_branch),
                });
                if job.state == "review" {
                    job.state = "done".into();
                }
                if root.is_some() {
                    drop(_wt_lock);
                    match remove_job_worktree(&job).await {
                        Ok(r) => {
                            job.log.push_str(&removal_note(&r));
                            job.worktree_path = None;
                        }
                        Err(e) => job.log.push_str(&format!("! worktree kept: {e}\n")),
                    }
                }
                if strategy == integrate::Strategy::Squash && out.status == "merged" {
                    if let Ok(Some(_)) = integrate::delete_branch(&repo, &branch).await {
                        job.log.push_str(&format!(
                            "⎇ branch {branch} deleted (its work is the squash commit)\n"
                        ));
                    }
                }
            }
            status => {
                job.log.push_str(&format!(
                    "⚠ integration into {} {status}{}; your checkout is {}\n",
                    out.target_branch,
                    if out.conflicts.is_empty() {
                        String::new()
                    } else {
                        format!(" in {}", out.conflicts.join(", "))
                    },
                    if out.clean {
                        "as it was"
                    } else {
                        "NOT clean: check git status"
                    }
                ));
            }
        }
        job.log = clip(&job.log, LOG_MAX);
        self.save_job_checked(&job)?;
        Ok((out, job))
    }

    /// Throws the job's work away: its worktree goes (2.3's removal, which
    /// saves what is uncommitted on the branch first) and then the branch.
    /// The log keeps the tip, so `git branch <name> <sha>` brings it back
    /// until git collects it.
    pub async fn discard_work(&self, id: &str) -> Result<Value, String> {
        let mut job = self.job_or_err(id)?;
        let branch = self.branch_of_job(&job)?;
        let repo = self.repo_of(&job).await?;
        let root = worktree_root(&job)
            .await
            .map(|(_, r)| r)
            .filter(|r| r.exists());
        if let Some(root) = &root {
            self.ensure_quiet(&job, root)?;
        } else if is_active(&job.state) {
            return Err(format!("{ERR_REVIEW_BUSY}: job {id} is {}", job.state));
        }
        let removed = match &root {
            Some(r) => {
                let _held = review::lock(r).await;
                let out = remove_job_worktree(&job).await?;
                job.log.push_str(&removal_note(&out));
                job.worktree_path = None;
                Some(out)
            }
            None => None,
        };
        let tip = integrate::delete_branch(&repo, &branch)
            .await
            .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
        job.log.push_str(&match &tip {
            Some(t) => format!(
                "✗ discarded by you: branch {branch} deleted (it was {}; `git branch {branch} {t}` brings it back until git collects it)\n",
                short(t)
            ),
            None => "✗ discarded by you\n".to_string(),
        });
        if job.state == "review" {
            job.state = "cancelled".into();
            job.error = Some("discarded by you".into());
        }
        job.log = clip(&job.log, LOG_MAX);
        self.save_job_checked(&job)?;
        Ok(json!({ "removed": removed, "branch": branch, "tip": tip, "job": job }))
    }

    // ── walkthrough ──────────────────────────────────────────────────

    /// A guided tour of the job's diff by the job's own agent, on demand,
    /// cached by the hunks it covers. Refused, not cut, when the diff is too
    /// large (`ERR_WALKTHROUGH_TOO_LARGE`).
    pub async fn walkthrough(&self, id: &str, refresh: bool) -> Result<Walkthrough, String> {
        let (job, patch, complete) = self.diff(id).await?;
        if !complete {
            return Err(format!("{ERR_REVIEW_BUSY}: job {id} is still running"));
        }
        let input = review::walkthrough_input(&patch.unwrap_or_default(), job.diff_clipped)?;
        if !refresh {
            if let Some(mut w) = review::cache_get(&input.key) {
                w.cached = true;
                return Ok(w);
            }
        }
        let agent =
            if job.agent_id.starts_with(TOOL_PREFIX) || self.llm.agent(&job.agent_id).is_none() {
                self.llm
                    .roster()
                    .first()
                    .map(|a| a.id.clone())
                    .ok_or_else(|| format!("{ERR_JOBS}: no agent to write the walkthrough"))?
            } else {
                job.agent_id.clone()
            };
        let conv = format!("walkthrough-{}", uuid::Uuid::new_v4().simple());
        let text = format!("{}\n\n{}", input.system, input.prompt);
        let limit = std::time::Duration::from_secs((120 + input.aliases.len() as u64).min(15 * 60));
        let (request, cancel, mut stream) = self.llm.turn_stream(&conv, &agent, &text).await?;
        let mut answer = String::new();
        let mut usage: Option<Value> = None;
        let mut failed: Option<String> = None;
        let model = self.llm.model_label(&agent);
        let run = async {
            while let Some(event) = stream.next().await {
                match &event {
                    TurnEvent::TextDelta { text } => answer.push_str(text),
                    TurnEvent::Error { error } => failed = Some(error.to_string()),
                    TurnEvent::Usage { usage: u } => fold_usage(&mut usage, &model, u),
                    _ => {}
                }
            }
        };
        let timed_out = tokio::time::timeout(limit, run).await.is_err();
        if timed_out {
            cancel.cancel();
        }
        self.llm.finish_turn(&request, &agent);
        self.llm.drop_conversation(&conv);
        if timed_out {
            return Err(format!(
                "{ERR_JOBS}: the walkthrough took longer than {} s",
                limit.as_secs()
            ));
        }
        if let Some(e) = failed.filter(|_| answer.trim().is_empty()) {
            return Err(format!("{ERR_JOBS}: the walkthrough failed: {e}"));
        }
        let mut w = review::parse_walkthrough(&answer, &input);
        w.model = model;
        w.created_ms = now_ms();
        w.cost_usd = usage.as_ref().and_then(|u| u["cost_usd"].as_f64());
        w.tokens = usage.as_ref().map(billable_of);
        review::cache_put(&w);
        Ok(w)
    }
}
