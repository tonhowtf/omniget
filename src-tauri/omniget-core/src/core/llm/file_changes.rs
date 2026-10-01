//! Per-file baselines of a CLI turn (3.4, S19, `85#P4`): the "before" of
//! every file a Claude turn wrote with its own tools, straight from the
//! stream. SP3 showed that the `user` event carrying a `Write`/`Edit` result
//! has `tool_use_result.originalFile` (the content before; `null` with
//! `type: "create"` for a new file), so no hook and no snapshot is needed to
//! know what a job touched and what the file held before.
//!
//! The parser hands each touch to [`record`]; the host installs a sink
//! ([`set_sink`]) that files it under the job of that turn (`job_changes` in
//! `jobs.db`, first touch per path wins). [`patch`] rebuilds a unified diff
//! from a baseline and the file as it is now, the fallback when the job's
//! shadow snapshots are not available.
//!
//! Limits: a file changed through the shell (`sed -i`, a generator) is not
//! here, only through the file tools; a baseline above
//! [`MAX_BASELINE_BYTES`] is kept as "too big" (no content).

use std::path::Path;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Largest "before" kept per file.
pub const MAX_BASELINE_BYTES: usize = 1024 * 1024;

/// One file a turn wrote, as the CLI reported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileTouch {
    /// Absolute path, as the CLI saw it.
    pub path: String,
    /// `create` (the file did not exist), `update` (whole-file write) or
    /// `edit` (a replacement).
    pub op: String,
    /// The content before; `None` for a created file or a baseline above
    /// [`MAX_BASELINE_BYTES`] (then `too_big`).
    pub original: Option<String>,
    #[serde(default)]
    pub too_big: bool,
}

/// The touch in a stream-json `tool_use_result` (SP3 shapes), if any.
pub fn from_tool_use_result(result: &Value) -> Option<FileTouch> {
    let path = result
        .get("filePath")
        .or_else(|| result.get("notebook_path"))
        .and_then(Value::as_str)?
        .to_string();
    // A `Read` also has a path (under `file`), never a top-level one with
    // `originalFile`; only writers carry that key.
    let original = result.get("originalFile")?;
    let op = match result.get("type").and_then(Value::as_str) {
        Some("create") => "create",
        Some("update") => "update",
        _ => "edit",
    };
    let (original, too_big) = match original.as_str() {
        Some(text) if text.len() > MAX_BASELINE_BYTES => (None, true),
        Some(text) => (Some(text.to_string()), false),
        None => (None, false),
    };
    Some(FileTouch {
        path,
        op: if original.is_none() && !too_big {
            "create".into()
        } else {
            op.into()
        },
        original,
        too_big,
    })
}

type Sink = Arc<dyn Fn(&str, FileTouch) + Send + Sync>;
static SINK: RwLock<Option<Sink>> = RwLock::new(None);

/// Installs where the touches of every turn go (`request id`, touch).
pub fn set_sink(sink: Option<Sink>) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = sink;
}

/// Hands one touch of turn `request` to the sink, if any.
pub fn record(request: &str, touch: FileTouch) {
    let sink = SINK.read().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(sink) = sink {
        sink(request, touch);
    }
}

fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').collect()
}

/// Longest common subsequence table is quadratic: past this many cells the
/// diff is one hunk that replaces the whole file (still correct).
const LCS_CELLS: usize = 4_000_000;

/// Edit script: (`' '|'-'|'+'`, line).
fn script<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(char, &'a str)> {
    // Common head and tail first: most edits touch a few lines.
    let head = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (ma, mb) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let mut out: Vec<(char, &str)> = a[..head].iter().map(|l| (' ', *l)).collect();
    if (ma.len() + 1) * (mb.len() + 1) > LCS_CELLS {
        out.extend(ma.iter().map(|l| ('-', *l)));
        out.extend(mb.iter().map(|l| ('+', *l)));
    } else {
        let (n, m) = (ma.len(), mb.len());
        let mut t = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i * (m + 1) + j] = if ma[i] == mb[j] {
                    t[(i + 1) * (m + 1) + j + 1] + 1
                } else {
                    t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if ma[i] == mb[j] {
                out.push((' ', ma[i]));
                i += 1;
                j += 1;
            } else if t[(i + 1) * (m + 1) + j] >= t[i * (m + 1) + j + 1] {
                out.push(('-', ma[i]));
                i += 1;
            } else {
                out.push(('+', mb[j]));
                j += 1;
            }
        }
        out.extend(ma[i..].iter().map(|l| ('-', *l)));
        out.extend(mb[j..].iter().map(|l| ('+', *l)));
    }
    out.extend(a[a.len() - tail..].iter().map(|l| (' ', *l)));
    out
}

/// Unified diff of `rel` from `before` to `after` (`None` = absent), git
/// style (`a/` `b/`, 3 lines of context), with its `+`/`-` counts. Empty
/// patch when nothing changed.
pub fn patch(rel: &str, before: Option<&str>, after: Option<&str>) -> (String, u64, u64) {
    if before == after {
        return (String::new(), 0, 0);
    }
    let (a, b) = (lines(before.unwrap_or("")), lines(after.unwrap_or("")));
    let ops = script(&a, &b);
    let adds = ops.iter().filter(|(c, _)| *c == '+').count() as u64;
    let dels = ops.iter().filter(|(c, _)| *c == '-').count() as u64;
    let mut out = format!("diff --git a/{rel} b/{rel}\n");
    match (before, after) {
        (None, _) => out.push_str("new file mode 100644\n--- /dev/null\n"),
        _ => out.push_str(&format!("--- a/{rel}\n")),
    }
    match after {
        None => {
            out = format!(
            "diff --git a/{rel} b/{rel}\ndeleted file mode 100644\n--- a/{rel}\n+++ /dev/null\n"
        )
        }
        Some(_) => out.push_str(&format!("+++ b/{rel}\n")),
    }
    // Hunks: runs of changes with up to 3 lines of context around them.
    const CTX: usize = 3;
    let changed: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, (c, _))| *c != ' ')
        .map(|(i, _)| i)
        .collect();
    let mut k = 0;
    while k < changed.len() {
        let start = changed[k].saturating_sub(CTX);
        let mut end = changed[k];
        while k + 1 < changed.len() && changed[k + 1] <= end + 2 * CTX + 1 {
            k += 1;
            end = changed[k];
        }
        let end = (end + CTX + 1).min(ops.len());
        let a_start = ops[..start].iter().filter(|(c, _)| *c != '+').count();
        let b_start = ops[..start].iter().filter(|(c, _)| *c != '-').count();
        let a_len = ops[start..end].iter().filter(|(c, _)| *c != '+').count();
        let b_len = ops[start..end].iter().filter(|(c, _)| *c != '-').count();
        let at = |s: usize, len: usize| if len == 0 { s } else { s + 1 };
        out.push_str(&format!(
            "@@ -{},{a_len} +{},{b_len} @@\n",
            at(a_start, a_len),
            at(b_start, b_len)
        ));
        for (c, line) in &ops[start..end] {
            out.push(*c);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
        k += 1;
    }
    (out, adds, dels)
}

/// The file as it is now, for [`patch`] (`None` when gone or not text).
pub fn current(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
}
