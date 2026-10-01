use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use super::folder_key::FolderKey;
use super::snapshot::{self, JobDiff};
use crate::core::vcs::diff::{parse_hunks, split_patch, Hunk};
use crate::core::vcs::repo::GitCtx;

pub const ERR_HUNK_STALE: &str = "ERR_HUNK_STALE";
pub const ERR_HUNK_PARTIAL: &str = "ERR_HUNK_PARTIAL";
pub const ERR_REVIEW_BUSY: &str = "ERR_REVIEW_BUSY";
pub const ERR_WALKTHROUGH_TOO_LARGE: &str = "ERR_WALKTHROUGH_TOO_LARGE";
pub const ERR_WALKTHROUGH_EMPTY: &str = "ERR_WALKTHROUGH_EMPTY";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Worktree {
        root: PathBuf,
        base_oid: String,
    },
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
    pub diff: JobDiff,
    #[serde(skip)]
    pub patch: String,
}

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
    #[serde(default)]
    pub sent_ms: Option<u64>,
    #[serde(default)]
    pub sent_job: Option<String>,
    #[serde(default)]
    pub head_oid: Option<String>,
}

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

pub fn follow_up_prompt(notes: &[Note]) -> String {
    format!(
        "Review notes on the changes you made. Address each note in the files it names, then stop.\n\n{}",
        format_notes(notes)
    )
}

pub const WALKTHROUGH_MAX_BYTES: usize = 96 * 1024;
pub const WALKTHROUGH_PROMPT_VERSION: u32 = 1;
const MAX_STOPS: usize = 16;

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
    pub aliases: Vec<(String, String)>,
    pub skipped: Vec<String>,
    pub key: String,
    pub bytes: usize,
}

const WALKTHROUGH_SYSTEM: &str = "You are writing a guided review of a code change for the engineer who is about to read it.\n\nYour job is to impose a reading order the diff itself does not have. A diff is ordered by file path, which is almost never the order in which the change makes sense. Group related hunks, across files, into stops, and order the stops so that each one is understandable given the ones before it.\n\nWhat a good stop says: what this code now does differently, in terms of behavior, not syntax; why the hunks belong together; what a reviewer should check or be suspicious about, when there is something.\n\nWhat a bad stop says: \"Renamed X to Y\", \"Added a parameter\", \"Updated the imports\". Restating the diff in prose is worthless. Do not speculate about intent you cannot support from the code.\n\nRules:\n- Anchor every stop to hunk aliases exactly as given (h1, h2, ...). Never invent an alias.\n- Anchor each hunk at most once. You do not have to cover every hunk; what you omit is still shown to the reader.\n- Order stops so the reader builds understanding: entry points and data shape before the code that consumes them.\n- importance: \"critical\" for changes that carry real risk, \"context\" for supporting changes, \"normal\" otherwise.\n- Plain sentences, no markdown.\n- Do not use any tool. Answer from the diff below.\n\nReturn ONLY a JSON object, no prose around it and no code fences, in exactly this shape:\n{\"title\": string, \"focus\": string, \"stops\": [{\"title\": string, \"hunks\": [string], \"importance\": \"critical\"|\"normal\"|\"context\", \"prose\": string}]}";

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
    pub uncovered: Vec<String>,
    pub skipped: Vec<String>,
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

fn json_in(text: &str) -> Option<serde_json::Value> {
    let (a, b) = (text.find('{')?, text.rfind('}')?);
    (a < b)
        .then(|| serde_json::from_str(&text[a..=b]).ok())
        .flatten()
}

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
