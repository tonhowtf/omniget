pub fn app_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("OMNIGET_DATA_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }

    let base = dirs::data_dir()?;
    let new_path = base.join("wtf.tonho.omniget");
    let old_path = base.join("omniget");

    if old_path.exists() {
        let _ = std::fs::create_dir_all(&new_path);

        for dir_name in &["bin", "plugins"] {
            let src = old_path.join(dir_name);
            let dst = new_path.join(dir_name);
            if src.exists() && !dst.exists() {
                let _ = copy_dir_recursive(&src, &dst);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&old_path) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let dest = new_path.join(entry.file_name());
                    if !dest.exists() {
                        let _ = std::fs::copy(entry.path(), &dest);
                    }
                }
            }
        }
    }

    Some(new_path)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let dest = dst.join(entry.file_name());
            copy_dir_recursive(&entry.path(), &dest)?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Spellings of `root` as it may appear in a message: as given and resolved
/// (on macOS `/tmp` and `/var` are `/private/tmp` and `/private/var`).
fn spellings(root: &std::path::Path) -> Vec<String> {
    let mut out = vec![root.display().to_string()];
    if let Ok(real) = std::fs::canonicalize(root) {
        out.push(real.display().to_string());
    }
    let s = out[0].clone();
    if let Some(rest) = s.strip_prefix("/private/") {
        out.push(format!("/{rest}"));
    } else if s.starts_with("/tmp/") || s.starts_with("/var/") {
        out.push(format!("/private{s}"));
    }
    for o in out.iter_mut() {
        while o.len() > 1 && (o.ends_with('/') || o.ends_with('\\')) {
            o.pop();
        }
    }
    out.retain(|o| o.len() > 1);
    out.sort_by_key(|o| std::cmp::Reverse(o.len()));
    out.dedup();
    out
}

/// End of the path token that starts at `start`: whitespace, a quote, or a
/// `:` that ends the path (`reading /a/b.md: not found`).
fn path_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() || matches!(c, b'"' | b'\'' | b'`' | b')' | b'>' | b',' | b';') {
            break;
        }
        if c == b':' && bytes.get(i + 1).map_or(true, |n| n.is_ascii_whitespace()) {
            break;
        }
        i += 1;
    }
    i
}

/// Rewrites every path under `root` in `text` relative to it, prefixed with
/// `label` (`reading /…/skills/x/guide.md` → `reading x/guide.md` with an
/// empty label). For messages a model will read.
pub fn rebase_paths(text: &str, root: &std::path::Path, label: &str) -> String {
    let mut out = text.to_string();
    for s in spellings(root) {
        for sep in ['/', '\\'] {
            out = out.replace(&format!("{s}{sep}"), label);
        }
    }
    out
}

/// Cuts every path under the app's own data directory (the profile: keys,
/// memory, skills, logs) down to its file name, unless it is under `keep`
/// (the turn's workspace). Tool errors pass through this before a model sees
/// them: the profile's absolute path is private and useless to the model.
pub fn redact_private_paths(text: &str, keep: Option<&std::path::Path>) -> String {
    let Some(data) = app_data_dir() else {
        return text.to_string();
    };
    let keep: Vec<String> = keep.map(spellings).unwrap_or_default();
    let prefixes = spellings(&data);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'scan: while i < text.len() {
        for p in &prefixes {
            if text[i..].starts_with(p.as_str()) {
                let after = &text[i + p.len()..];
                // Only the directory itself or something inside it.
                if !(after.is_empty()
                    || after.starts_with('/')
                    || after.starts_with('\\')
                    || path_end(after, 0) == 0)
                {
                    continue;
                }
                let end = path_end(text, i + p.len());
                let token = &text[i..end];
                if keep.iter().any(|k| token.starts_with(k.as_str())) {
                    out.push_str(token);
                } else {
                    let name = token
                        .rsplit(['/', '\\'])
                        .find(|s| !s.is_empty())
                        .unwrap_or("");
                    out.push_str(if token.len() == p.len() {
                        "<app data>"
                    } else {
                        name
                    });
                }
                i = end;
                continue 'scan;
            }
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod redact_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rebase_makes_skill_paths_relative() {
        let root = Path::new("/private/tmp/demo/profile/llm/skills");
        let msg = "reading /private/tmp/demo/profile/llm/skills/curadoria/references/guide.md: No such file or directory (os error 2)";
        assert_eq!(
            rebase_paths(msg, root, ""),
            "reading curadoria/references/guide.md: No such file or directory (os error 2)"
        );
        // The unresolved spelling too.
        let msg2 = "reading /tmp/demo/profile/llm/skills/curadoria/guide.md: x";
        assert_eq!(
            rebase_paths(msg2, root, ""),
            "reading curadoria/guide.md: x"
        );
    }

    #[test]
    fn private_paths_are_cut_to_the_file_name_but_the_workspace_is_kept() {
        let data = app_data_dir().expect("a data dir");
        let d = data.display().to_string();
        let msg = format!("ERR_X: opening {d}/llm/secrets/keys.json: denied; also {d}/memory.db and /Users/x/proj/src/main.rs");
        let out = redact_private_paths(&msg, None);
        assert_eq!(
            out,
            "ERR_X: opening keys.json: denied; also memory.db and /Users/x/proj/src/main.rs"
        );
        assert!(!out.contains(&d));
        let ws = data.join("worktrees").join("w1");
        let msg = format!("cannot write {}/src/lib.rs", ws.display());
        assert_eq!(
            redact_private_paths(&msg, Some(&ws)),
            msg,
            "paths in the turn's workspace stay as they are"
        );
    }
}
