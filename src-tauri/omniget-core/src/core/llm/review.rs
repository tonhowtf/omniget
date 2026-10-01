//! Review of what a job changed (roadmap 3.5, S20): discard one hunk,
//! notes that go back to the agent, and the walkthrough of the diff. The
//! integration of a job's branch is git only and lives in
//! [`crate::core::vcs::integrate`].
//!
//! - **Hunk discard** (84#P2): the caller sends the hunk's id *and* its
//!   patch; both must match a hunk of the diff as it is *now*, byte for
//!   byte, and `git apply -R --check` must pass, before `git apply -R`
//!   touches a file. All of it runs under the review lock of the folder, so
//!   two discards (or a discard and an integration) never interleave. A hunk
//!   that moved is refused with [`ERR_HUNK_STALE`] ("refresh"), and a hunk
//!   cut by the diff's byte limit with [`ERR_HUNK_PARTIAL`].
//! - **Notes** (78#P5): `File: …` / `Lines: a-b` | `Line: n` | `Scope: file`
//!   / `User comment: "…"`, notes separated by a blank line, the format of
//!   stablyai/orca's `formatDiffComment` (MIT), ported. They become the
//!   prompt of a follow-up turn in the job's conversation, which the CLI
//!   runtime resumes (`--resume`): the turn costs the notes, not the task.
//! - **Walkthrough** (84#P5): on demand only; a diff too large for one
//!   request is refused ([`ERR_WALKTHROUGH_TOO_LARGE`]), never cut. The
//!   prompt and the shape of the answer are adapted from openchamber's
//!   `walkthrough/prompt.js` and `schema.js` (MIT); every anchor the model
//!   returns is resolved against the hunks sent, and what does not resolve
//!   is dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use super::folder_key::FolderKey;
use super::snapshot::{self, JobDiff};
use crate::core::vcs::diff::{parse_hunks, split_patch, Hunk};
use crate::core::vcs::repo::GitCtx;

/// The hunk is not in the diff as it is now (the code moved): refresh.
pub const ERR_HUNK_STALE: &str = "ERR_HUNK_STALE";
/// The hunk was cut by the diff's byte limit: shown, never acted on.
pub const ERR_HUNK_PARTIAL: &str = "ERR_HUNK_PARTIAL";
/// A job is writing in the folder right now.
pub const ERR_REVIEW_BUSY: &str = "ERR_REVIEW_BUSY";
pub const ERR_WALKTHROUGH_TOO_LARGE: &str = "ERR_WALKTHROUGH_TOO_LARGE";
pub const ERR_WALKTHROUGH_EMPTY: &str = "ERR_WALKTHROUGH_EMPTY";

/// Where a job's changes live and what they are measured from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A job in its own worktree (2.3): from `merge-base(base_oid, HEAD)`
    /// to the worktree as it is ([`snapshot::worktree_diff`]).
    Worktree { root: PathBuf, base_oid: String },
    /// A job in the folder itself: from the shadow tree taken before its
    /// first turn to the folder as it is ([`snapshot::live_diff`]).
    Shared {
        workspace: PathBuf,
        base_tree: String,
    },
}

impl Target {
    pub fn folder(&self) -> &Path {
        match self {
            Target::Worktree { root, .. } => root,
            Target::Shared { workspace, .. } => workspace,
        }
    }

    pub async fn live_diff(&self) -> Result<JobDiff, String> {
        match self {
            Target::Worktree { root, base_oid } => snapshot::worktree_diff(root, base_oid).await,
            Target::Shared {
                workspace,
                base_tree,
            } => snapshot::live_diff(workspace, base_tree).await,
        }
    }

    /// The git that applies a patch here, paths relative to the folder's
    /// top: the worktree itself, or the folder's shadow git.
    fn ctx(&self) -> Result<GitCtx, String> {
        match self {
            Target::Worktree { root, .. } => Ok(GitCtx::Repo {
                work_tree: root.clone(),
            }),
            Target::Shared { workspace, .. } => snapshot::shadow_ctx(workspace),
        }
    }
}

type FolderLock = Arc<tokio::sync::Mutex<()>>;
static LOCKS: LazyLock<Mutex<HashMap<FolderKey, FolderLock>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The review lock of `folder` (canonical: a symlink or another letter case
/// is the same folder). Held across the check and the write of a discard
/// or an integration.
pub async fn lock(folder: &Path) -> tokio::sync::OwnedMutexGuard<()> {
    let key = FolderKey::of(folder);
    let m = LOCKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .clone();
    m.lock_owned().await
}

/// The hunk `id` of `live` (the diff as it is now), if `sent` is its patch
/// byte for byte and it is whole.
pub fn find_hunk(live: &str, id: &str, sent: &str) -> Result<Hunk, String> {
    let hunk = parse_hunks(live)
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| {
            format!("{ERR_HUNK_STALE}: hunk {id} is not in the diff any more: the code changed, refresh")
        })?;
    if !hunk.complete {
        return Err(format!(
            "{ERR_HUNK_PARTIAL}: hunk {id} was cut by the diff's size limit and cannot be discarded"
        ));
    }
    if hunk.patch.as_bytes() != sent.as_bytes() {
        return Err(format!(
            "{ERR_HUNK_STALE}: the patch sent for hunk {id} is not the hunk as it is now: refresh"
        ));
    }
    Ok(hunk)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discarded {
    pub hunk: Hunk,
    /// The diff after the discard (its `patch` is not serialized; the
    /// caller stores it).
    pub diff: JobDiff,
    #[serde(skip)]
    pub patch: String,
}

/// Puts one hunk back as it was before the job (see the module doc).
pub async fn discard_hunk(target: &Target, id: &str, sent: &str) -> Result<Discarded, String> {
    let _held = lock(target.folder()).await;
    let live = target.live_diff().await?;
    let hunk = find_hunk(&live.patch, id, sent)?;
    let ctx = target.ctx()?;
    let apply = |check: bool| {
        let mut inv = ctx.git().args([
            "-c",
            "core.autocrlf=false",
            "apply",
            "-R",
            "--whitespace=nowarn",
        ]);
        if check {
            inv = inv.arg("--check");
        }
        inv.arg("-")
            .stdin(hunk.patch.clone().into_bytes())
            .unchecked()
    };
    let checked = apply(true).run().await.map_err(|e| e.to_string())?;
    if !checked.ok() {
        return Err(format!(
            "{ERR_HUNK_STALE}: git apply --check refused hunk {id}: {}",
            checked.err_text().trim()
        ));
    }
    let done = apply(false).run().await.map_err(|e| e.to_string())?;
    if !done.ok() {
        return Err(format!(
            "{ERR_HUNK_STALE}: git apply refused hunk {id}: {}",
            done.err_text().trim()
        ));
    }
    let mut diff = target.live_diff().await?;
    let patch = std::mem::take(&mut diff.patch);
    Ok(Discarded { hunk, diff, patch })
}

/// `diff` cut down to `paths` (files, totals and patch): a shared job's diff
/// taken against the folder as it is holds other people's later changes too;
/// what the job shows stays its own files.
pub fn restrict_to_paths(mut diff: JobDiff, paths: &[String]) -> JobDiff {
    let keep = |p: &str| paths.iter().any(|k| k == p);
    diff.files.retain(|f| keep(&f.path));
    diff.additions = diff.files.iter().filter_map(|f| f.additions).sum();
    diff.deletions = diff.files.iter().filter_map(|f| f.deletions).sum();
    diff.patch = split_patch(&diff.patch)
        .into_iter()
        .filter(|c| {
            crate::core::vcs::diff::chunk_path(c)
                .as_deref()
                .is_some_and(keep)
        })
        .collect();
    diff
}

// ── notes ────────────────────────────────────────────────────────────

/// A review note on a job's diff. `start_line`/`end_line` are lines of the
/// new version; both `None` = a note on the whole file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Note {
    pub id: String,
    pub job_id: String,
    pub path: String,
    #[serde(default)]
    pub start_line: Option<u32>,
    #[serde(default)]
    pub end_line: Option<u32>,
    pub body: String,
    pub created_ms: u64,
    /// When it went to the agent, and in which follow-up job. Sent notes
    /// stay, marked (orca drops them, against its own doc).
    #[serde(default)]
    pub sent_ms: Option<u64>,
    #[serde(default)]
    pub sent_job: Option<String>,
    /// The job's head when the note was written: a later head means the note
    /// may be about an earlier version of the lines.
    #[serde(default)]
    pub head_oid: Option<String>,
}

/// One note, orca's contract: deterministic and quote-safe.
pub fn format_note(n: &Note) -> String {
    let body = n
        .body
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    let location = match (n.start_line, n.end_line) {
        (None, None) => "Scope: file".to_string(),
        (Some(a), Some(b)) if a != b => format!("Lines: {}-{}", a.min(b), a.max(b)),
        (Some(a), _) | (None, Some(a)) => format!("Line: {a}"),
    };
    format!("File: {}\n{location}\nUser comment: \"{body}\"", n.path)
}

pub fn format_notes(notes: &[Note]) -> String {
    notes
        .iter()
        .map(format_note)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The prompt of the follow-up turn: one line of instruction and the
/// notes. With `--resume` the task and the earlier turns are already in the
/// session; only this travels.
pub fn follow_up_prompt(notes: &[Note]) -> String {
    format!(
        "Review notes on the changes you made. Address each note in the files it names, then stop.\n\n{}",
        format_notes(notes)
    )
}

// ── walkthrough ──────────────────────────────────────────────────────

/// Most bytes of hunks one walkthrough request carries (~24k tokens). Past
/// it the walkthrough is refused, never cut: a tour of part of a change
/// would read as the whole.
pub const WALKTHROUGH_MAX_BYTES: usize = 96 * 1024;
/// Bumping it invalidates every cached walkthrough (a changed prompt).
pub const WALKTHROUGH_PROMPT_VERSION: u32 = 1;
const MAX_STOPS: usize = 16;

/// Lockfiles and generated files: left out of the tour by name.
pub fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    matches!(
        name.as_str(),
        "cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lockb"
            | "poetry.lock"
            | "pipfile.lock"
            | "composer.lock"
            | "gemfile.lock"
            | "go.sum"
            | "uv.lock"
    ) || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".map")
        || path.starts_with("dist/")
        || path.contains("/dist/")
        || path.contains("/generated/")
}

#[derive(Debug, Clone, PartialEq)]
pub struct WalkthroughInput {
    pub system: String,
    pub prompt: String,
    /// `h1` … in the prompt → hunk id.
    pub aliases: Vec<(String, String)>,
    pub skipped: Vec<String>,
    /// Cache key: the hunks sent and the prompt version.
    pub key: String,
    pub bytes: usize,
}

const WALKTHROUGH_SYSTEM: &str = "You are writing a guided review of a code change for the engineer who is about to read it.\n\nYour job is to impose a reading order the diff itself does not have. A diff is ordered by file path, which is almost never the order in which the change makes sense. Group related hunks, across files, into stops, and order the stops so that each one is understandable given the ones before it.\n\nWhat a good stop says: what this code now does differently, in terms of behavior, not syntax; why the hunks belong together; what a reviewer should check or be suspicious about, when there is something.\n\nWhat a bad stop says: \"Renamed X to Y\", \"Added a parameter\", \"Updated the imports\". Restating the diff in prose is worthless. Do not speculate about intent you cannot support from the code.\n\nRules:\n- Anchor every stop to hunk aliases exactly as given (h1, h2, ...). Never invent an alias.\n- Anchor each hunk at most once. You do not have to cover every hunk; what you omit is still shown to the reader.\n- Order stops so the reader builds understanding: entry points and data shape before the code that consumes them.\n- importance: \"critical\" for changes that carry real risk, \"context\" for supporting changes, \"normal\" otherwise.\n- Plain sentences, no markdown.\n- Do not use any tool. Answer from the diff below.\n\nReturn ONLY a JSON object, no prose around it and no code fences, in exactly this shape:\n{\"title\": string, \"focus\": string, \"stops\": [{\"title\": string, \"hunks\": [string], \"importance\": \"critical\"|\"normal\"|\"context\", \"prose\": string}]}";

/// The request of a walkthrough of `patch`, or why there is none.
pub fn walkthrough_input(patch: &str, clipped: bool) -> Result<WalkthroughInput, String> {
    use sha2::{Digest, Sha256};
    if clipped {
        return Err(format!(
            "{ERR_WALKTHROUGH_TOO_LARGE}: the job's diff was cut at {} KB; a walkthrough of part of it would read as the whole",
            snapshot::DIFF_MAX_BYTES / 1024
        ));
    }
    let hunks = parse_hunks(patch);
    let (skip, keep): (Vec<&Hunk>, Vec<&Hunk>) = hunks.iter().partition(|h| is_generated(&h.path));
    let mut skipped: Vec<String> = skip.iter().map(|h| h.path.clone()).collect();
    skipped.dedup();
    if keep.is_empty() {
        return Err(format!(
            "{ERR_WALKTHROUGH_EMPTY}: nothing to walk through (no text hunk outside lockfiles and generated files)"
        ));
    }
    if let Some(h) = keep.iter().find(|h| !h.complete) {
        return Err(format!(
            "{ERR_WALKTHROUGH_TOO_LARGE}: hunk {} was cut by the diff's size limit",
            h.id
        ));
    }
    let mut body = String::new();
    let mut aliases = Vec::new();
    let mut hasher = Sha256::new();
    hasher.update(format!("v{WALKTHROUGH_PROMPT_VERSION}\n"));
    for (i, h) in keep.iter().enumerate() {
        let alias = format!("h{}", i + 1);
        // The hunk without its file header: `@@` onwards.
        let text = &h.patch[h.patch.find("@@ -").unwrap_or(0)..];
        body.push_str(&format!("### {alias} {}\n{text}", h.path));
        if !text.ends_with('\n') {
            body.push('\n');
        }
        hasher.update(h.id.as_bytes());
        hasher.update([0]);
        aliases.push((alias, h.id.clone()));
    }
    if body.len() > WALKTHROUGH_MAX_BYTES {
        return Err(format!(
            "{ERR_WALKTHROUGH_TOO_LARGE}: {} KB of hunks, the limit is {} KB; review it by file instead",
            body.len().div_ceil(1024),
            WALKTHROUGH_MAX_BYTES / 1024
        ));
    }
    let files = {
        let mut p: Vec<&str> = keep.iter().map(|h| h.path.as_str()).collect();
        p.dedup();
        p.len()
    };
    let target = (keep.len() as f64 / 2.5)
        .round()
        .clamp(1.0, MAX_STOPS as f64) as usize;
    let prompt = format!(
        "This change has {files} file(s) and {} hunk(s). Aim for about {target} stop(s); never more than {MAX_STOPS}.\n\nHunks:\n{body}",
        keep.len()
    );
    Ok(WalkthroughInput {
        system: WALKTHROUGH_SYSTEM.to_string(),
        bytes: body.len(),
        prompt,
        aliases,
        skipped,
        key: hex::encode(hasher.finalize())[..32].to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Stop {
    pub title: String,
    /// Hunk ids (resolved from the aliases).
    pub hunks: Vec<String>,
    pub importance: String,
    pub prose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Walkthrough {
    pub key: String,
    pub title: String,
    pub focus: String,
    pub stops: Vec<Stop>,
    /// Hunks no stop covers (shown in a folded tail).
    pub uncovered: Vec<String>,
    /// Files left out by name (lockfiles, generated).
    pub skipped: Vec<String>,
    /// The model did not answer JSON: its text, shown as is.
    #[serde(default)]
    pub raw: Option<String>,
    pub model: String,
    pub created_ms: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub cached: bool,
}

/// The first `{` … last `}` of `text`, parsed.
fn json_in(text: &str) -> Option<serde_json::Value> {
    let (a, b) = (text.find('{')?, text.rfind('}')?);
    (a < b)
        .then(|| serde_json::from_str(&text[a..=b]).ok())
        .flatten()
}

/// What the model answered, anchors resolved against `input`: an alias
/// that is not one of the hunks sent is dropped, a stop left with no hunk
/// too, a hunk anchored twice stays in its first stop.
pub fn parse_walkthrough(text: &str, input: &WalkthroughInput) -> Walkthrough {
    let alias: HashMap<&str, &str> = input
        .aliases
        .iter()
        .map(|(a, id)| (a.as_str(), id.as_str()))
        .collect();
    let mut out = Walkthrough {
        key: input.key.clone(),
        skipped: input.skipped.clone(),
        ..Default::default()
    };
    let mut used: Vec<String> = Vec::new();
    match json_in(text) {
        Some(v) => {
            let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
            out.title = s(&v, "title");
            out.focus = s(&v, "focus");
            for stop in v["stops"].as_array().into_iter().flatten().take(MAX_STOPS) {
                let hunks: Vec<String> = stop["hunks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|h| h.as_str())
                    .filter_map(|a| alias.get(a.trim()).map(|id| id.to_string()))
                    .filter(|id| !used.contains(id))
                    .collect();
                if hunks.is_empty() {
                    continue;
                }
                used.extend(hunks.iter().cloned());
                let importance = match s(stop, "importance").as_str() {
                    i @ ("critical" | "context") => i.to_string(),
                    _ => "normal".to_string(),
                };
                out.stops.push(Stop {
                    title: s(stop, "title"),
                    hunks,
                    importance,
                    prose: s(stop, "prose"),
                });
            }
        }
        None => out.raw = Some(text.trim().to_string()),
    }
    out.uncovered = input
        .aliases
        .iter()
        .map(|(_, id)| id.clone())
        .filter(|id| !used.contains(id))
        .collect();
    out
}

/// `<llm dir>/walkthrough/<key>.json`.
pub fn cache_path(key: &str) -> Option<PathBuf> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    super::roster_store::llm_dir().map(|d| d.join("walkthrough").join(format!("{key}.json")))
}

pub fn cache_get(key: &str) -> Option<Walkthrough> {
    let bytes = std::fs::read(cache_path(key)?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Written through a temp file and a rename: a reader never sees half.
pub fn cache_put(w: &Walkthrough) {
    let Some(path) = cache_path(&w.key) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    if let Ok(bytes) = serde_json::to_vec(w) {
        if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
