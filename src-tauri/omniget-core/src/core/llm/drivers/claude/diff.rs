//! Unified diffs for the approval card (plan §3.3 "o pedido mostra o diff
//! completo"). Claude's `can_use_tool` carries the tool *input* (`old_string`
//! / `new_string`, or the whole new `content`), never a patch, so the driver
//! replays the edit on the file as it is on disk and diffs before/after.
//!
//! Own line diff (LCS), no crate: the approval only needs a readable patch
//! with 3 lines of context, and files the model edits are small. Past
//! [`MAX_CELLS`] the diff degrades to a one-hunk summary instead of burning
//! memory.

use std::path::Path;

use serde_json::Value;

/// LCS table budget (lines(a) × lines(b)).
const MAX_CELLS: usize = 4_000_000;
/// Files larger than this are not read for a preview.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const CONTEXT: usize = 3;

/// `diff -u` of two texts. Empty when they are equal.
pub fn unified(before: &str, after: &str, path: &str, new_file: bool) -> String {
    if before == after {
        return String::new();
    }
    let a: Vec<&str> = split_lines(before);
    let b: Vec<&str> = split_lines(after);
    let display = path.trim_start_matches('/');
    let mut out = format!(
        "--- {}\n+++ b/{}\n",
        if new_file {
            "/dev/null".to_string()
        } else {
            format!("a/{display}")
        },
        display
    );
    if a.len().saturating_mul(b.len()) > MAX_CELLS {
        out.push_str(&format!(
            "@@ -1,{} +1,{} @@ file too large for a line diff\n",
            a.len(),
            b.len()
        ));
        return out;
    }
    let ops = edit_script(&a, &b);
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            k += 1;
            continue;
        }
        let start = k.saturating_sub(CONTEXT);
        let mut last_change = k;
        let mut end = k;
        while end < ops.len() {
            if ops[end].0 != ' ' {
                last_change = end;
            } else if end - last_change > CONTEXT * 2 {
                break;
            }
            end += 1;
        }
        let end = (last_change + CONTEXT + 1).min(ops.len());
        let a_start = ops[start].1;
        let b_start = ops[start].2;
        let a_len = ops[start..end].iter().filter(|o| o.0 != '+').count();
        let b_len = ops[start..end].iter().filter(|o| o.0 != '-').count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            if a_len == 0 { a_start } else { a_start + 1 },
            a_len,
            if b_len == 0 { b_start } else { b_start + 1 },
            b_len
        ));
        for op in &ops[start..end] {
            let line = match op.0 {
                '+' => b[op.2],
                _ => a[op.1],
            };
            out.push(op.0);
            out.push_str(line);
            out.push('\n');
        }
        k = end;
    }
    out
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.lines().collect()
    }
}

/// `(tag, a_index, b_index)` with tag ` `, `-` or `+`.
fn edit_script(a: &[&str], b: &[&str]) -> Vec<(char, usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    let mut t = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * w + j] = if a[i] == b[j] {
                t[(i + 1) * w + j + 1] + 1
            } else {
                t[(i + 1) * w + j].max(t[i * w + j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            ops.push((' ', i, j));
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i * w + j + 1] > t[(i + 1) * w + j]) {
            ops.push(('+', i, j));
            j += 1;
        } else {
            ops.push(('-', i, j));
            i += 1;
        }
    }
    ops
}

/// Replays one `old_string → new_string` edit. `None` when `old_string` is
/// not in the text (the CLI will refuse the edit anyway; the card then shows
/// the raw strings).
pub fn apply_edit(text: &str, old: &str, new: &str, replace_all: bool) -> Option<String> {
    if old.is_empty() {
        // Claude's Edit with an empty old_string creates the file.
        return if text.is_empty() {
            Some(new.to_string())
        } else {
            None
        };
    }
    if !text.contains(old) {
        return None;
    }
    Some(if replace_all {
        text.replace(old, new)
    } else {
        text.replacen(old, new, 1)
    })
}

/// The file an edit tool touches, if any (`file_path`, `notebook_path`).
pub fn target_path(input: &Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Diff of what `tool` would do to the file, given the current content
/// (`None` = the file does not exist). Handles `Write`, `Edit` and
/// `MultiEdit`; anything else yields `None`.
pub fn preview_edit(tool: &str, input: &Value, current: Option<&str>) -> Option<String> {
    let path = target_path(input)?;
    let before = current.unwrap_or("");
    let new_file = current.is_none();
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let after = match tool {
        "Write" => input.get("content").and_then(Value::as_str)?.to_string(),
        "Edit" => {
            let replace_all = input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match apply_edit(
                before,
                &str_of(input, "old_string"),
                &str_of(input, "new_string"),
                replace_all,
            ) {
                Some(t) => t,
                // The old text is not in the file: show the intent instead of
                // a diff against the wrong base.
                None => {
                    return Some(unified(
                        &str_of(input, "old_string"),
                        &str_of(input, "new_string"),
                        &path,
                        false,
                    ))
                }
            }
        }
        "MultiEdit" => {
            let mut text = before.to_string();
            for edit in input.get("edits").and_then(Value::as_array)? {
                let replace_all = edit
                    .get("replace_all")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                text = apply_edit(
                    &text,
                    &str_of(edit, "old_string"),
                    &str_of(edit, "new_string"),
                    replace_all,
                )?;
            }
            text
        }
        _ => return None,
    };
    Some(unified(before, &after, &path, new_file))
}

/// Reads a file for a preview; `None` when missing, unreadable or too big.
pub fn read_for_preview(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_one_line_change_has_one_hunk_with_context() {
        let before = "a\nb\nc\nd\ne\nf\ng\n";
        let after = "a\nb\nc\nD\ne\nf\ng\n";
        let d = unified(before, after, "/x/y.txt", false);
        assert!(d.starts_with("--- a/x/y.txt\n+++ b/x/y.txt\n"), "{d}");
        assert!(d.contains("@@ -1,7 +1,7 @@"), "{d}");
        assert!(d.contains("-d\n+D\n"), "{d}");
        assert_eq!(d.matches("@@ -").count(), 1);
    }

    #[test]
    fn a_new_file_diffs_against_dev_null() {
        let d = unified("", "oi", "a.txt", true);
        assert_eq!(d, "--- /dev/null\n+++ b/a.txt\n@@ -0,0 +1,1 @@\n+oi\n");
    }

    #[test]
    fn edit_and_multiedit_replay_on_the_current_text() {
        let input = json!({"file_path": "/t/a.txt", "old_string": "oi", "new_string": "ola"});
        let d = preview_edit("Edit", &input, Some("oi")).unwrap();
        assert!(d.contains("-oi\n+ola\n"), "{d}");
        let multi = json!({"file_path": "/t/a.txt", "edits": [
            {"old_string": "one", "new_string": "1"},
            {"old_string": "two", "new_string": "2", "replace_all": true}
        ]});
        let d = preview_edit("MultiEdit", &multi, Some("one\ntwo\ntwo\n")).unwrap();
        assert!(d.contains("-one\n-two\n-two\n+1\n+2\n+2\n"), "{d}");
        let write = json!({"file_path": "/t/n.txt", "content": "x\n"});
        assert!(preview_edit("Write", &write, None)
            .unwrap()
            .starts_with("--- /dev/null"));
        assert!(preview_edit("Bash", &json!({"command": "ls"}), None).is_none());
    }

    #[test]
    fn an_edit_whose_old_text_is_gone_still_shows_the_intent() {
        let input = json!({"file_path": "/t/a.txt", "old_string": "zzz", "new_string": "y"});
        let d = preview_edit("Edit", &input, Some("abc")).unwrap();
        assert!(d.contains("-zzz\n+y\n"), "{d}");
    }
}
