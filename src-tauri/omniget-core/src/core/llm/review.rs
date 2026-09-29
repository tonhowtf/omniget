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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_use_orcas_format() {
        let n = |start, end, body: &str| Note {
            path: "src/a.rs".into(),
            start_line: start,
            end_line: end,
            body: body.into(),
            ..Default::default()
        };
        assert_eq!(
            format_note(&n(Some(3), Some(9), "rename x")),
            "File: src/a.rs\nLines: 3-9\nUser comment: \"rename x\""
        );
        assert_eq!(
            format_note(&n(Some(4), Some(4), "a")),
            "File: src/a.rs\nLine: 4\nUser comment: \"a\""
        );
        assert_eq!(
            format_note(&n(None, None, "say \"hi\"\\\nnext\r")),
            "File: src/a.rs\nScope: file\nUser comment: \"say \\\"hi\\\"\\\\\\nnext\\r\""
        );
        let two = format_notes(&[n(Some(1), None, "a"), n(None, None, "b")]);
        assert_eq!(
            two,
            "File: src/a.rs\nLine: 1\nUser comment: \"a\"\n\nFile: src/a.rs\nScope: file\nUser comment: \"b\""
        );
        assert!(follow_up_prompt(&[n(None, None, "b")]).ends_with("User comment: \"b\""));
    }

    const PATCH: &str = "diff --git a/a.txt b/a.txt\nindex 1..2 100644\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n\
diff --git a/Cargo.lock b/Cargo.lock\nindex 1..2 100644\n--- a/Cargo.lock\n+++ b/Cargo.lock\n@@ -1 +1 @@\n-x\n+y\n";

    #[test]
    fn the_walkthrough_request_leaves_lockfiles_out_and_refuses_what_is_too_big() {
        let input = walkthrough_input(PATCH, false).unwrap();
        assert_eq!(input.aliases.len(), 1);
        assert_eq!(input.skipped, vec!["Cargo.lock".to_string()]);
        assert!(input
            .prompt
            .contains("### h1 a.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"));
        assert!(!input.prompt.contains("Cargo.lock"));
        assert_eq!(walkthrough_input(PATCH, false).unwrap().key, input.key);
        let err = walkthrough_input(PATCH, true).unwrap_err();
        assert!(err.starts_with(ERR_WALKTHROUGH_TOO_LARGE), "{err}");
        let big: String = (0..30_000).map(|i| format!("+line {i}\n")).collect();
        let huge = format!(
            "diff --git a/b.txt b/b.txt\nnew file mode 100644\n--- /dev/null\n+++ b/b.txt\n@@ -0,0 +1,30000 @@\n{big}"
        );
        let err = walkthrough_input(&huge, false).unwrap_err();
        assert!(err.starts_with(ERR_WALKTHROUGH_TOO_LARGE), "{err}");
        let only_lock = &PATCH[PATCH.find("diff --git a/Cargo.lock").unwrap()..];
        assert!(walkthrough_input(only_lock, false)
            .unwrap_err()
            .starts_with(ERR_WALKTHROUGH_EMPTY));
    }

    #[test]
    fn the_answer_is_anchored_to_the_hunks_sent_and_nothing_else() {
        let two = format!(
            "{PATCH}diff --git a/c.txt b/c.txt\nindex 1..2 100644\n--- a/c.txt\n+++ b/c.txt\n@@ -1 +1 @@\n-p\n+q\n"
        );
        let input = walkthrough_input(&two, false).unwrap();
        assert_eq!(input.aliases.len(), 2);
        let text = r#"Sure: ```{"title":"T","focus":"F","stops":[
            {"title":"one","hunks":["h1","h9"],"importance":"critical","prose":"p"},
            {"title":"dup","hunks":["h1"],"importance":"weird","prose":"x"}]}```"#;
        let w = parse_walkthrough(text, &input);
        assert_eq!(w.title, "T");
        assert_eq!(w.stops.len(), 1);
        assert_eq!(w.stops[0].hunks, vec![input.aliases[0].1.clone()]);
        assert_eq!(w.stops[0].importance, "critical");
        assert_eq!(w.uncovered, vec![input.aliases[1].1.clone()]);
        let raw = parse_walkthrough("no json here", &input);
        assert_eq!(raw.raw.as_deref(), Some("no json here"));
        assert_eq!(raw.uncovered.len(), 2);
    }

    #[test]
    fn a_hunk_is_found_only_whole_and_byte_for_byte() {
        let hunks = parse_hunks(PATCH);
        let h = &hunks[0];
        assert_eq!(find_hunk(PATCH, &h.id, &h.patch).unwrap(), *h);
        let err = find_hunk(PATCH, &h.id, &h.patch.replace("+B", "+b")).unwrap_err();
        assert!(err.starts_with(ERR_HUNK_STALE), "{err}");
        let err = find_hunk(PATCH, "a.txt:00000000", &h.patch).unwrap_err();
        assert!(err.starts_with(ERR_HUNK_STALE), "{err}");
        let cut = &PATCH[..PATCH.find(" c\n").unwrap()];
        let partial = &parse_hunks(cut)[0];
        let err = find_hunk(cut, &partial.id, &partial.patch).unwrap_err();
        assert!(err.starts_with(ERR_HUNK_PARTIAL), "{err}");
    }

    #[test]
    fn a_diff_restricted_to_the_jobs_paths_keeps_only_them() {
        use super::snapshot::FileChange;
        let fc = |p: &str, a: u64| FileChange {
            path: p.into(),
            status: "M".into(),
            additions: Some(a),
            deletions: Some(1),
            binary: false,
        };
        let d = JobDiff {
            files: vec![fc("a.txt", 1), fc("Cargo.lock", 5)],
            additions: 6,
            deletions: 2,
            patch: PATCH.into(),
            ..Default::default()
        };
        let r = restrict_to_paths(d, &["a.txt".into()]);
        assert_eq!(r.files.len(), 1);
        assert_eq!((r.additions, r.deletions), (1, 1));
        assert!(r.patch.starts_with("diff --git a/a.txt") && !r.patch.contains("Cargo.lock"));
    }
}

/// BN4, onda 3 (3.5): hunk discards, integration and the notes follow-up,
/// against real git repositories, the real job worktrees and (for notes) the
/// real Claude argv and parser over `fake_claude.py`. Metrics go to
/// `OMNIGET_BENCH_OUT`; `scripts/llm/diff_eval.py` turns them into goals.
#[cfg(test)]
mod bn4 {
    use super::*;
    use crate::core::llm::cli_runtime::claude::{self as cc, ClaudeArgs, ClaudeParser};
    use crate::core::llm::cli_runtime::session::{spawn_turn_with, SpawnSpec, TurnHooks};
    use crate::core::llm::types::{FinishReason, TurnEvent};
    use crate::core::vcs::{integrate, job_worktree};
    use futures::StreamExt;
    use std::time::Instant;
    use tokio_util::sync::CancellationToken;

    fn bench(metric: &str, value: serde_json::Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{}",
                    serde_json::json!({ "bench": "bn4", "metric": metric, "value": value, "level": "review (3.5)" })
                );
            }
        }
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "u")
            .env("GIT_AUTHOR_EMAIL", "u@u")
            .env("GIT_COMMITTER_NAME", "u")
            .env("GIT_COMMITTER_EMAIL", "u@u")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// The git tests' data dir, held for the whole test (`test_env`).
    fn data_dir() -> crate::core::test_env::Shared {
        crate::core::test_env::shared_data_dir()
    }

    fn seed_body(i: usize) -> String {
        (0..80).map(|l| format!("f{i} line {l}\n")).collect()
    }

    /// 5 files × 4 changes 20 lines apart (3 lines of context never join
    /// them): a replaced line, a removed line, an added line, two replaced
    /// lines. 20 hunks.
    fn edited_body(i: usize) -> String {
        let mut out = String::new();
        for l in 0..80 {
            let line = format!("f{i} line {l}\n");
            match l {
                5 => out.push_str(&format!("f{i} line 5 EDITED\n")),
                25 => {}
                45 => {
                    out.push_str(&line);
                    out.push_str(&format!("f{i} added after 45\n"));
                }
                65 | 66 => out.push_str(&format!("f{i} line {l} TWICE\n")),
                _ => out.push_str(&line),
            }
        }
        out
    }

    fn files_of(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        (0..5)
            .map(|i| {
                let rel = format!("src/f{i}.txt");
                (
                    rel.clone(),
                    std::fs::read(dir.join(&rel)).unwrap_or_default(),
                )
            })
            .collect()
    }

    /// Every hunk other than `id`, by what does not move when another hunk
    /// goes: its file, where it sits in the base, and its lines.
    fn neighbours(patch: &str, but: &str) -> Vec<(String, u32, u32, String)> {
        let mut v: Vec<_> = parse_hunks(patch)
            .into_iter()
            .filter(|h| h.id != but)
            .map(|h| {
                let body = h.patch[h.patch.find("@@ -").unwrap()..]
                    .split_once('\n')
                    .map(|(_, b)| b.to_string())
                    .unwrap_or_default();
                (h.path, h.old_start, h.old_lines, body)
            })
            .collect();
        v.sort();
        v
    }

    /// 20 discards one hunk at a time, in a scattered order, each checked
    /// against the diff before it: the hunk gone, every other hunk the same
    /// lines at the same place in the base, the other files byte for byte.
    async fn twenty_discards(target: &Target, mode: &str) -> (usize, usize, Vec<f64>) {
        let (mut clean, mut total) = (0, 0);
        let mut times = Vec::new();
        for k in 0..20 {
            let before = target.live_diff().await.unwrap();
            let hunks = parse_hunks(&before.patch);
            assert_eq!(hunks.len(), 20 - k, "{mode}: hunks before discard {k}");
            let pick = &hunks[(k * 7) % hunks.len()];
            let files_before = files_of(target.folder());
            let t = Instant::now();
            let out = discard_hunk(target, &pick.id, &pick.patch).await.unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            total += 1;
            let files_after = files_of(target.folder());
            let others_same = files_before
                .iter()
                .filter(|(p, _)| **p != pick.path)
                .all(|(p, b)| files_after.get(p) == Some(b));
            let gone = !parse_hunks(&out.patch).iter().any(|h| h.id == pick.id);
            let same_neighbours = neighbours(&before.patch, &pick.id) == neighbours(&out.patch, "");
            if gone && others_same && same_neighbours {
                clean += 1;
            } else {
                eprintln!("{mode}: discard {k} of {}: gone={gone} others={others_same} neighbours={same_neighbours}", pick.id);
            }
        }
        (clean, total, times)
    }

    fn p50_max(mut v: Vec<f64>) -> serde_json::Value {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        serde_json::json!({ "p50_ms": v[v.len() / 2], "max_ms": v[v.len() - 1] })
    }

    #[tokio::test]
    async fn bn4_hunk_discards_touch_no_neighbour_and_refuse_a_stale_hunk() {
        if !snapshot::git_available() {
            return eprintln!("git missing: skipped");
        }
        let _data = data_dir();
        // ── worktree mode ──
        let repo = std::env::temp_dir().join(format!("omniget-bn4h-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(repo.join("src")).unwrap();
        for i in 0..5 {
            std::fs::write(repo.join(format!("src/f{i}.txt")), seed_body(i)).unwrap();
        }
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "seed"]);
        let id = format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let wt = job_worktree::create(&repo, &id, None).await.unwrap();
        for i in 0..5 {
            std::fs::write(wt.path.join(format!("src/f{i}.txt")), edited_body(i)).unwrap();
        }
        job_worktree::save_work(&wt.path, "job work").await.unwrap();
        let target = Target::Worktree {
            root: wt.path.clone(),
            base_oid: wt.base_oid.clone(),
        };
        // A stale hunk: its lines changed after the diff was read.
        let before = target.live_diff().await.unwrap();
        let h = parse_hunks(&before.patch)
            .into_iter()
            .find(|h| h.path == "src/f2.txt")
            .unwrap();
        let file = wt.path.join("src/f2.txt");
        let original = std::fs::read_to_string(&file).unwrap();
        std::fs::write(
            &file,
            original.replace("f2 line 5 EDITED", "f2 line 5 EDITED AGAIN"),
        )
        .unwrap();
        let moved = std::fs::read(&file).unwrap();
        let stale = discard_hunk(&target, &h.id, &h.patch).await.unwrap_err();
        let stale_untouched = std::fs::read(&file).unwrap() == moved;
        // Same id, one byte different in the patch sent: refused too.
        std::fs::write(&file, &original).unwrap();
        let forged = discard_hunk(&target, &h.id, &h.patch.replace("EDITED", "EDITEd"))
            .await
            .unwrap_err();
        bench(
            "hunk_stale_refused",
            serde_json::json!({
                "moved_lines": stale.split(':').next(), "forged_patch": forged.split(':').next(),
                "file_untouched": stale_untouched,
            }),
        );
        assert!(stale.starts_with(ERR_HUNK_STALE), "{stale}");
        assert!(forged.starts_with(ERR_HUNK_STALE), "{forged}");
        assert!(stale_untouched);
        let (clean, total, times) = twenty_discards(&target, "worktree").await;
        bench(
            "hunk_discards_worktree",
            serde_json::json!({ "clean": clean, "total": total }),
        );
        bench("hunk_discard_ms_worktree", p50_max(times));
        let rest = target.live_diff().await.unwrap();
        bench("hunk_worktree_back_to_base", (rest.files.is_empty()).into());
        assert_eq!((clean, total), (20, 20));
        assert!(rest.files.is_empty(), "{:?}", rest.files);

        // ── shared mode: the folder itself, measured from its shadow snapshot ──
        let ws = snapshot::undo_tests::folder("bn4h");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        for i in 0..5 {
            std::fs::write(ws.join(format!("src/f{i}.txt")), seed_body(i)).unwrap();
        }
        let (conv, req) = ("bn4h-conv", "bn4h-req");
        snapshot::before_write(&ws, conv, req).await.unwrap();
        for i in 0..5 {
            std::fs::write(ws.join(format!("src/f{i}.txt")), edited_body(i)).unwrap();
        }
        snapshot::after_turn(&ws, conv, req).await.unwrap();
        let job = snapshot::job_diff(&ws, conv, &[req.to_string()])
            .await
            .unwrap();
        let target = Target::Shared {
            workspace: ws.clone(),
            base_tree: job.base.clone().unwrap(),
        };
        // Two discards at once, in different files: the lock serialises them
        // and both land.
        let hunks = parse_hunks(&job.patch);
        let (a, b) = (
            hunks
                .iter()
                .find(|h| h.path == "src/f0.txt")
                .unwrap()
                .clone(),
            hunks
                .iter()
                .find(|h| h.path == "src/f4.txt")
                .unwrap()
                .clone(),
        );
        let (ra, rb) = tokio::join!(
            discard_hunk(&target, &a.id, &a.patch),
            discard_hunk(&target, &b.id, &b.patch)
        );
        let both = ra.is_ok() && rb.is_ok();
        bench("hunk_concurrent_discards_both_landed", both.into());
        assert!(both, "{ra:?} {rb:?}");
        // Put them back for the 20-discard run.
        for i in [0, 4] {
            std::fs::write(ws.join(format!("src/f{i}.txt")), edited_body(i)).unwrap();
        }
        let (clean, total, times) = twenty_discards(&target, "shared").await;
        bench(
            "hunk_discards_shared",
            serde_json::json!({ "clean": clean, "total": total }),
        );
        bench("hunk_discard_ms_shared", p50_max(times));
        assert_eq!((clean, total), (20, 20));
        for i in 0..5 {
            assert_eq!(
                std::fs::read_to_string(ws.join(format!("src/f{i}.txt"))).unwrap(),
                seed_body(i)
            );
        }
    }

    #[tokio::test]
    async fn bn4_integrate_two_jobs_then_a_conflict_leaves_the_repo_clean() {
        if !snapshot::git_available() {
            return eprintln!("git missing: skipped");
        }
        let _data = data_dir();
        let repo = std::env::temp_dir().join(format!("omniget-bn4i-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("a.txt"), "a1\na2\na3\n").unwrap();
        std::fs::write(repo.join("b.txt"), "b1\nb2\nb3\n").unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "User"]);
        git(&repo, &["config", "user.email", "user@example.com"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-qm", "seed"]);
        let job = |file: &'static str, body: &'static str| {
            let repo = repo.clone();
            async move {
                let id = format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
                let wt = job_worktree::create(&repo, &id, None).await.unwrap();
                std::fs::write(wt.path.join(file), body).unwrap();
                job_worktree::save_work(&wt.path, "job work").await.unwrap();
                wt
            }
        };
        // Scenario A: two jobs, different files → two merges.
        let (j1, j2) = (
            job("a.txt", "a1\nA2\na3\n").await,
            job("b.txt", "b1\nB2\nb3\n").await,
        );
        let mut times = Vec::new();
        let mut merged = 0;
        for wt in [&j1, &j2] {
            let t = Instant::now();
            let out = integrate::integrate(&repo, &wt.branch, integrate::Strategy::Merge, "job")
                .await
                .unwrap();
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            merged += usize::from(out.status == "merged");
        }
        let merges = git(&repo, &["log", "--merges", "--oneline", "main"])
            .lines()
            .count();
        bench(
            "integrate_no_conflict",
            serde_json::json!({ "merged": merged, "merge_commits": merges }),
        );
        bench("integrate_ms", p50_max(times));
        assert_eq!((merged, merges), (2, 2));
        // Scenario B: two jobs on the same line → the second is a conflict,
        // undone: no MERGE_HEAD, status clean, HEAD where it was.
        let (j3, j4) = (
            job("a.txt", "a1\nTHREE\na3\n").await,
            job("a.txt", "a1\nFOUR\na3\n").await,
        );
        let first = integrate::integrate(&repo, &j3.branch, integrate::Strategy::Merge, "j3")
            .await
            .unwrap();
        assert_eq!(first.status, "merged");
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let mut conflict_clean = 0;
        for strategy in [integrate::Strategy::Merge, integrate::Strategy::Squash] {
            let out = integrate::integrate(&repo, &j4.branch, strategy, "j4")
                .await
                .unwrap();
            let ok = out.status == "conflict"
                && out.conflicts == vec!["a.txt".to_string()]
                && out.clean
                && !repo.join(".git/MERGE_HEAD").exists()
                && git(&repo, &["status", "--porcelain"]).is_empty()
                && git(&repo, &["rev-parse", "HEAD"]) == head;
            conflict_clean += usize::from(ok);
        }
        bench(
            "integrate_conflict_clean",
            serde_json::json!({ "clean": conflict_clean, "of": 2, "merge_head": repo.join(".git/MERGE_HEAD").exists() }),
        );
        assert_eq!(conflict_clean, 2);
        // Discard: the worktree goes (2.3's removal) and so does the branch.
        let out = job_worktree::remove(&repo, &j4.path, &j4.branch, Some(&j4.base_oid))
            .await
            .unwrap();
        let tip = integrate::delete_branch(&repo, &j4.branch).await.unwrap();
        let branch_gone = git(&repo, &["branch", "--list", &j4.branch]).is_empty();
        for wt in [&j1, &j2, &j3] {
            let _ = job_worktree::remove(&repo, &wt.path, &wt.branch, Some(&wt.base_oid)).await;
        }
        let listed = git(&repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|l| l.starts_with("worktree "))
            .count();
        bench(
            "discard_job",
            serde_json::json!({ "branch_gone": branch_gone, "tip_reported": tip.is_some(),
                "worktrees_left": listed - 1, "saved_before_removal": out.saved.is_some() }),
        );
        assert!(branch_gone && tip.is_some());
        assert_eq!(listed, 1);
    }

    fn fake() -> Option<PathBuf> {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/llm/fake_claude.py");
        let python = std::process::Command::new("python3")
            .arg("--version")
            .output()
            .ok()?;
        (python.status.success() && script.is_file()).then_some(script)
    }

    /// One turn of the real Claude argv (`resume` or not) over the fake CLI.
    async fn turn(
        fake: &Path,
        cwd: &Path,
        resume: Option<&str>,
        stdin: &str,
        env: &[(&str, String)],
    ) -> (bool, Option<String>) {
        let argv = cc::argv(&ClaudeArgs {
            partial_messages: true,
            resume: resume.map(String::from),
            permission_mode: Some("acceptEdits"),
            ..ClaudeArgs::default()
        });
        let spec = SpawnSpec {
            program: PathBuf::from("python3"),
            args: std::iter::once(fake.display().to_string())
                .chain(argv)
                .collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            cwd: Some(cwd.to_path_buf()),
            stdin: Some(stdin.to_string()),
            ..SpawnSpec::default()
        };
        let seen: Arc<Mutex<Option<String>>> = Arc::default();
        let slot = seen.clone();
        let hooks = TurnHooks {
            session: Some(Arc::new(move |handle: String| {
                *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
            })),
            ..TurnHooks::default()
        };
        let mut stream = spawn_turn_with(
            spec,
            Arc::new(ClaudeParser::new(true)),
            CancellationToken::new(),
            hooks,
        )
        .unwrap();
        let mut ok = false;
        while let Some(ev) = stream.next().await {
            if matches!(
                ev,
                TurnEvent::Finished {
                    reason: FinishReason::Stop
                }
            ) {
                ok = true;
            }
        }
        let session = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        (ok, session)
    }

    /// Notes → follow-up (78#P5), `--fake`: the job's first turn edits
    /// in its worktree; a note ("rename calc_total to compute_total" on the
    /// lines) becomes the follow-up prompt, sent with `--resume <session of
    /// the first turn>`, alone on stdin (what the runtime sends a resumed
    /// session), and the fake agent applies what the note asks. 20 reps.
    #[tokio::test]
    async fn bn4_notes_follow_up_resumes_the_session_with_only_the_notes() {
        let Some(fake) = fake() else {
            return eprintln!("python3 or fake_claude.py missing: skipped");
        };
        if !snapshot::git_available() {
            return eprintln!("git missing: skipped");
        }
        let _data = data_dir();
        let task = "Add a function calc_total(items) to shop.py that sums price * qty, with a test. Keep the style of the file.";
        let (mut ok, mut resumed, mut only_notes, mut applied) = (0, 0, 0, 0);
        let mut times = Vec::new();
        let reps = 20;
        for _ in 0..reps {
            let repo = std::env::temp_dir().join(format!("omniget-bn4n-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&repo).unwrap();
            std::fs::write(repo.join("shop.py"), "# shop\n").unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
            git(&repo, &["add", "-A"]);
            git(&repo, &["commit", "-qm", "seed"]);
            let id = format!("j{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
            let wt = job_worktree::create(&repo, &id, None).await.unwrap();
            let changes = repo.with_extension("changes.json");
            std::fs::write(
                &changes,
                serde_json::json!({"write": {"shop.py": "# shop\n\ndef calc_total(items):\n    return sum(i.price * i.qty for i in items)\n"}}).to_string(),
            )
            .unwrap();
            let (first_ok, session) = turn(
                &fake,
                &wt.path,
                None,
                &format!("User: {task}"),
                &[
                    ("FAKE_CLAUDE_MODE", "apply".into()),
                    ("FAKE_CLAUDE_CHANGES", changes.display().to_string()),
                ],
            )
            .await;
            assert!(first_ok);
            job_worktree::save_work(&wt.path, "job work").await.unwrap();
            let note = Note {
                path: "shop.py".into(),
                start_line: Some(3),
                end_line: Some(4),
                body: "rename calc_total to compute_total".into(),
                ..Default::default()
            };
            let prompt = follow_up_prompt(std::slice::from_ref(&note));
            // A resumed session gets only the newest user message.
            use crate::core::llm::types::{Message, Role};
            let history = vec![
                Message::text(Role::User, task),
                Message::text(Role::Assistant, "applied the change"),
                Message::text(Role::User, prompt.clone()),
            ];
            let stdin = cc::last_user_text(&history);
            let trace = repo.with_extension("trace.json");
            let t = Instant::now();
            let (second_ok, _) = turn(
                &fake,
                &wt.path,
                session.as_deref(),
                &stdin,
                &[
                    ("FAKE_CLAUDE_MODE", "notes".into()),
                    ("FAKE_CLAUDE_TRACE", trace.display().to_string()),
                ],
            )
            .await;
            times.push(t.elapsed().as_secs_f64() * 1000.0);
            let seen: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&trace).unwrap_or_default())
                    .unwrap_or_default();
            let argv: Vec<String> = seen["argv"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(String::from))
                .collect();
            let resume_ok = session.is_some()
                && argv
                    .windows(2)
                    .any(|w| w[0] == "--resume" && Some(&w[1]) == session.as_ref());
            let notes_ok =
                seen["stdin"].as_str() == Some(prompt.as_str()) && !prompt.contains(task);
            let body = std::fs::read_to_string(wt.path.join("shop.py")).unwrap_or_default();
            let applied_ok =
                body.contains("def compute_total(items):") && !body.contains("calc_total");
            ok += usize::from(second_ok);
            resumed += usize::from(resume_ok);
            only_notes += usize::from(notes_ok);
            applied += usize::from(second_ok && resume_ok && notes_ok && applied_ok);
            let _ = job_worktree::remove(&repo, &wt.path, &wt.branch, Some(&wt.base_oid)).await;
        }
        let prompt_bytes = follow_up_prompt(&[Note {
            path: "shop.py".into(),
            start_line: Some(3),
            end_line: Some(4),
            body: "rename calc_total to compute_total".into(),
            ..Default::default()
        }])
        .len();
        bench(
            "notes_follow_up",
            serde_json::json!({
                "reps": reps, "turn_ok": ok, "resumed": resumed, "only_notes_on_stdin": only_notes,
                "success": applied, "success_rate": applied as f64 / reps as f64,
                "follow_up_stdin_bytes": prompt_bytes, "task_prompt_bytes": task.len(),
            }),
        );
        bench("notes_follow_up_ms", p50_max(times));
        assert_eq!(applied, reps);
    }
}
