use std::path::Path;

use serde_json::Value;

const MAX_CELLS: usize = 4_000_000;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const CONTEXT: usize = 3;

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

pub fn apply_edit(text: &str, old: &str, new: &str, replace_all: bool) -> Option<String> {
    if old.is_empty() {
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

pub fn target_path(input: &Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

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

pub fn read_for_preview(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}
