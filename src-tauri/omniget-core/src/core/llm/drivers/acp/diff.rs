//! Unified diff of two texts (own LCS, 3 lines of context). Used to turn an
//! ACP `diff` content (`oldText`/`newText`) into the patch a `request.opened`
//! shows. Big inputs get a one-line summary instead of a quadratic table.

pub fn unified_diff(old: &str, new: &str, path: &str) -> String {
    if old == new {
        return String::new();
    }
    let al: Vec<&str> = old.lines().collect();
    let bl: Vec<&str> = new.lines().collect();
    let shown = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let header = format!(
        "--- {}\n+++ b{shown}\n",
        if old.is_empty() {
            "/dev/null".to_string()
        } else {
            format!("a{shown}")
        }
    );
    let (n, m) = (al.len(), bl.len());
    if n.saturating_mul(m) > 4_000_000 {
        return format!("{header}@@ file too large for a line diff: {n} → {m} lines @@\n");
    }
    let mut t = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            t[i * (m + 1) + j] = if al[i] == bl[j] {
                t[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                t[(i + 1) * (m + 1) + j].max(t[i * (m + 1) + j + 1])
            };
        }
    }
    let mut ops: Vec<(char, usize, usize)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && al[i] == bl[j] {
            ops.push((' ', i, j));
            i += 1;
            j += 1;
        } else if j < m && (i == n || t[i * (m + 1) + j + 1] > t[(i + 1) * (m + 1) + j]) {
            ops.push(('+', i, j));
            j += 1;
        } else {
            ops.push(('-', i, j));
            i += 1;
        }
    }
    let mut out = header;
    let ctx = 3;
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            k += 1;
            continue;
        }
        let start = k.saturating_sub(ctx);
        let mut end = k;
        let mut last_change = k;
        while end < ops.len() {
            if ops[end].0 != ' ' {
                last_change = end;
            } else if end - last_change > ctx * 2 {
                break;
            }
            end += 1;
        }
        let end = (last_change + ctx + 1).min(ops.len());
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
        for o in &ops[start..end] {
            let line = match o.0 {
                '+' => bl[o.2],
                _ => al[o.1],
            };
            out.push(o.0);
            out.push_str(line);
            out.push('\n');
        }
        k = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::unified_diff;

    #[test]
    fn diff_marks_changed_and_new_lines() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\nd\n", "/x.rs");
        assert!(d.starts_with("--- a/x.rs\n+++ b/x.rs\n@@"));
        assert!(d.contains("-b\n+B\n"));
        assert!(d.contains("+d\n"));
        let created = unified_diff("", "hi\n", "/n.txt");
        assert!(created.starts_with("--- /dev/null\n"));
        assert!(unified_diff("same", "same", "/s").is_empty());
    }
}
