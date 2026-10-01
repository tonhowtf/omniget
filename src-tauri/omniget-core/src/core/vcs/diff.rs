//! Diffs between two tree-ishes: `--numstat -z` for the counts and one patch
//! split per file, with byte limits and a `truncated` flag.

use serde::{Deserialize, Serialize};

use super::repo::GitCtx;
use super::runner::VcsResult;

/// Whole-diff patch budget.
pub const DEFAULT_MAX_BYTES: usize = 2 * 1024 * 1024;
/// A single file's patch past this is dropped (the counts stay).
pub const DEFAULT_MAX_FILE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NumstatEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    /// added | deleted | modified | renamed | binary
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    /// Unified patch for this file (`diff --git` header included). Empty when
    /// binary or when it did not fit the limits (`truncated`).
    pub patch: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DiffResult {
    pub from: String,
    pub to: String,
    pub files: Vec<FileDiff>,
    pub additions: u32,
    pub deletions: u32,
    /// Some file patch was left out because of the limits.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct DiffOptions {
    pub ignore_whitespace: bool,
    pub max_bytes: usize,
    pub max_file_bytes: usize,
    pub context: u32,
    /// Only numstat, no patches.
    pub stat_only: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ignore_whitespace: false,
            max_bytes: DEFAULT_MAX_BYTES,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            context: 3,
            stat_only: false,
        }
    }
}

/// Parses `git diff --numstat -z`. Renames come as `a\td\t\0old\0new\0`.
pub fn parse_numstat_z(raw: &[u8]) -> Vec<NumstatEntry> {
    let parts: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let head = String::from_utf8_lossy(parts[i]).into_owned();
        i += 1;
        let head = head.trim_start_matches('\n');
        if head.is_empty() {
            continue;
        }
        let mut f = head.splitn(3, '\t');
        let a = f.next().unwrap_or("");
        let d = f.next().unwrap_or("");
        let path = f.next().unwrap_or("");
        let binary = a == "-" && d == "-";
        let additions = a.parse().unwrap_or(0);
        let deletions = d.parse().unwrap_or(0);
        if path.is_empty() {
            let old = parts
                .get(i)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let new = parts
                .get(i + 1)
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            i += 2;
            out.push(NumstatEntry {
                path: new,
                old_path: Some(old),
                additions,
                deletions,
                binary,
            });
        } else {
            out.push(NumstatEntry {
                path: path.to_string(),
                old_path: None,
                additions,
                deletions,
                binary,
            });
        }
    }
    out
}

/// Splits a multi-file patch at each `diff --git ` line.
pub fn split_patch(patch: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    let mut pos = 0;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            starts.push(pos);
        }
        pos += line.len();
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(patch.len());
        out.push(&patch[*s..e]);
    }
    out
}

/// Status of one file from its patch header.
pub fn status_from_patch(chunk: &str, binary: bool, renamed: bool) -> &'static str {
    let header: Vec<&str> = chunk
        .lines()
        .take_while(|l| !l.starts_with("@@"))
        .take(8)
        .collect();
    if header.iter().any(|l| l.starts_with("new file mode")) {
        "added"
    } else if header.iter().any(|l| l.starts_with("deleted file mode")) {
        "deleted"
    } else if renamed || header.iter().any(|l| l.starts_with("rename from")) {
        "renamed"
    } else if binary {
        "binary"
    } else {
        "modified"
    }
}

/// One hunk of a unified patch, named by its content (roadmap 3.5, 84#P2):
/// `id = <path>:<sha1(@@ line + body)[..8]>`, computed only here, so the
/// UI, the bridge and the CLI all name a hunk the same way and a hunk whose
/// bytes moved gets another id. Id and hunk layout adapted from
/// openchamber's `walkthrough/hunks.js` (MIT).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hunk {
    pub id: String,
    pub path: String,
    /// Position among the hunks of its file (0-based): the DiffViewer's
    /// hunk index for the same file patch.
    pub index: usize,
    /// The `@@ -a,b +c,d @@` line, section heading included.
    pub header: String,
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub additions: u32,
    pub deletions: u32,
    /// A patch of this hunk alone: the file's header lines and the hunk,
    /// byte for byte as in the diff. What a discard sends back, and what
    /// `git apply -R` takes.
    pub patch: String,
    /// The body holds exactly the lines the header counts. A patch cut at
    /// its byte limit can end in the middle of a hunk: such a hunk is shown,
    /// never acted on.
    pub complete: bool,
}

/// `@@ -a[,b] +c[,d] @@…` → (a, b, c, d); a missing count is 1.
fn hunk_range(line: &str) -> Option<(u32, u32, u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let end = rest.find(" @@")?;
    let (old, new) = rest[..end].split_once(" +")?;
    let pair = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (a, b) = pair(old)?;
    let (c, d) = pair(new)?;
    Some((a, b, c, d))
}

/// Every hunk of a multi-file unified patch, in patch order. Binary files
/// and mode-only changes have none.
pub fn parse_hunks(patch: &str) -> Vec<Hunk> {
    use sha1::{Digest, Sha1};
    let mut out: Vec<Hunk> = Vec::new();
    for chunk in split_patch(patch) {
        let Some(path) = chunk_path(chunk) else {
            continue;
        };
        // Byte offsets of each `@@ -` line of this file.
        let mut starts = Vec::new();
        let mut pos = 0;
        for line in chunk.split_inclusive('\n') {
            if line.starts_with("@@ -") {
                starts.push(pos);
            }
            pos += line.len();
        }
        let Some(&first) = starts.first() else {
            continue;
        };
        let file_header = &chunk[..first];
        for (k, s) in starts.iter().enumerate() {
            let e = starts.get(k + 1).copied().unwrap_or(chunk.len());
            let text = &chunk[*s..e];
            let header_line = text.lines().next().unwrap_or("");
            let Some((old_start, old_lines, new_start, new_lines)) = hunk_range(header_line) else {
                continue;
            };
            let (mut ctx, mut add, mut del) = (0u32, 0u32, 0u32);
            for l in text.split_inclusive('\n').skip(1) {
                match l.as_bytes().first() {
                    Some(b'+') => add += 1,
                    Some(b'-') => del += 1,
                    Some(b' ') => ctx += 1,
                    // `\ No newline at end of file` counts nothing.
                    _ => {}
                }
            }
            let complete = ctx + del == old_lines && ctx + add == new_lines && text.ends_with('\n');
            let digest = Sha1::digest(text.as_bytes());
            let mut id = format!("{path}:{}", &hex::encode(digest)[..8]);
            // Two identical hunks of one file would need the same lines at the
            // same place: cannot happen, but an id must stay unique.
            let mut n = 2;
            while out.iter().any(|h| h.id == id) {
                id = format!("{path}:{}~{n}", &hex::encode(digest)[..8]);
                n += 1;
            }
            out.push(Hunk {
                id,
                path: path.clone(),
                index: k,
                header: header_line.to_string(),
                old_start,
                old_lines,
                new_start,
                new_lines,
                additions: add,
                deletions: del,
                patch: format!("{file_header}{text}"),
                complete,
            });
        }
    }
    out
}

/// Path named by a patch chunk's `+++ b/` (or `--- a/` for a deletion).
pub(crate) fn chunk_path(chunk: &str) -> Option<String> {
    let mut minus = None;
    for l in chunk.lines().take(12) {
        if let Some(p) = l.strip_prefix("+++ b/") {
            return Some(p.to_string());
        }
        if let Some(p) = l.strip_prefix("--- a/") {
            minus = Some(p.to_string());
        }
        if let Some(p) = l.strip_prefix("rename to ") {
            return Some(p.to_string());
        }
        if l.starts_with("@@") {
            break;
        }
    }
    minus.or_else(|| {
        let first = chunk.lines().next()?;
        let rest = first.strip_prefix("diff --git a/")?;
        let idx = rest.find(" b/")?;
        Some(rest[..idx].to_string())
    })
}

/// Joins numstat entries with patch chunks, applying the limits.
pub fn assemble(
    from: &str,
    to: &str,
    stats: Vec<NumstatEntry>,
    patch: &str,
    opts: &DiffOptions,
) -> DiffResult {
    let chunks = split_patch(patch);
    let by_order = chunks.len() == stats.len();
    let mut used = 0usize;
    let mut res = DiffResult {
        from: from.into(),
        to: to.into(),
        ..Default::default()
    };
    for (k, s) in stats.into_iter().enumerate() {
        let chunk: Option<&str> = if by_order {
            chunks.get(k).copied()
        } else {
            chunks
                .iter()
                .copied()
                .find(|c| chunk_path(c).as_deref() == Some(s.path.as_str()))
        };
        let status = chunk
            .map(|c| status_from_patch(c, s.binary, s.old_path.is_some()))
            .unwrap_or(if s.binary {
                "binary"
            } else if s.old_path.is_some() {
                "renamed"
            } else {
                "modified"
            });
        let mut truncated = false;
        let text = if opts.stat_only {
            String::new()
        } else {
            match chunk {
                Some(c) if s.binary => {
                    // Keep the header so the viewer can say "binary".
                    c.lines()
                        .take_while(|l| !l.starts_with("GIT binary patch"))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                Some(c) if c.len() > opts.max_file_bytes || used + c.len() > opts.max_bytes => {
                    truncated = true;
                    String::new()
                }
                Some(c) => {
                    used += c.len();
                    c.to_string()
                }
                None => {
                    truncated = !s.binary && (s.additions + s.deletions) > 0 && !patch.is_empty();
                    String::new()
                }
            }
        };
        res.additions += s.additions;
        res.deletions += s.deletions;
        res.truncated |= truncated;
        res.files.push(FileDiff {
            path: s.path,
            old_path: s.old_path,
            status: status.into(),
            additions: s.additions,
            deletions: s.deletions,
            binary: s.binary,
            patch: text,
            truncated,
        });
    }
    res
}

/// `git diff --name-status -z`: path → `added|deleted|modified|renamed`
/// (copies count as added, type changes as modified). Renames are keyed by
/// the new path.
pub fn parse_name_status_z(raw: &[u8]) -> std::collections::HashMap<String, &'static str> {
    let parts: Vec<String> = raw
        .split(|b| *b == 0)
        .map(|b| {
            String::from_utf8_lossy(b)
                .trim_start_matches('\n')
                .to_string()
        })
        .collect();
    let mut out = std::collections::HashMap::new();
    let mut i = 0;
    while i < parts.len() {
        let code = parts[i].clone();
        i += 1;
        let Some(letter) = code.chars().next() else {
            continue;
        };
        let status = match letter {
            'A' | 'C' => "added",
            'D' => "deleted",
            'R' => "renamed",
            _ => "modified",
        };
        if matches!(letter, 'R' | 'C') {
            // `R100\0old\0new`
            if let Some(new) = parts.get(i + 1) {
                out.insert(new.clone(), status);
            }
            i += 2;
        } else {
            if let Some(path) = parts.get(i) {
                if !path.is_empty() {
                    out.insert(path.clone(), status);
                }
            }
            i += 1;
        }
    }
    out
}

/// Corrects the status of stat-only files with `--name-status`: added and
/// deleted win over binary/modified, like `status_from_patch` does.
pub fn apply_name_status(
    res: &mut DiffResult,
    names: &std::collections::HashMap<String, &'static str>,
) {
    for f in &mut res.files {
        match names.get(&f.path).copied() {
            Some(s @ ("added" | "deleted")) => f.status = s.into(),
            Some("renamed") if f.status != "renamed" => f.status = "renamed".into(),
            _ => {}
        }
    }
}

fn diff_base(ctx: &GitCtx, opts: &DiffOptions) -> super::runner::Invocation {
    let mut inv = ctx.git().args([
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
    ]);
    if opts.ignore_whitespace {
        inv = inv.arg("--ignore-all-space");
    }
    inv
}

/// Diff between two commits/trees (`from` → `to`).
pub async fn diff_trees(
    ctx: &GitCtx,
    from: &str,
    to: &str,
    opts: &DiffOptions,
) -> VcsResult<DiffResult> {
    let stat = diff_base(ctx, opts)
        .args(["--numstat", "-z", from, to, "--"])
        .timeout(Some(std::time::Duration::from_secs(120)))
        .run()
        .await?;
    let stats = parse_numstat_z(&stat.stdout);
    if opts.stat_only || stats.is_empty() {
        let mut res = assemble(
            from,
            to,
            stats,
            "",
            &DiffOptions {
                stat_only: true,
                ..*opts
            },
        );
        // Without patches the status only knows binary/renamed; ask git for
        // the real letters so added/deleted files are not shown as modified.
        if !res.files.is_empty() {
            let names = diff_base(ctx, opts)
                .args(["--name-status", "-z", from, to, "--"])
                .timeout(Some(std::time::Duration::from_secs(120)))
                .run()
                .await?;
            apply_name_status(&mut res, &parse_name_status_z(&names.stdout));
        }
        return Ok(res);
    }
    let patch = diff_base(ctx, opts)
        .arg(format!("--unified={}", opts.context.min(1000)))
        .args(["--patch", from, to, "--"])
        .max_output(64 * 1024 * 1024)
        .timeout(Some(std::time::Duration::from_secs(120)))
        .run()
        .await?;
    Ok(assemble(from, to, stats, &patch.text(), opts))
}
