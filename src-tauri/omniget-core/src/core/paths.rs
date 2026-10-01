pub fn app_data_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("OMNIGET_DATA_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }

    if let Some(p) = data_dir_redirect_file() {
        if let Ok(to) = std::fs::read_to_string(&p) {
            let to = std::path::PathBuf::from(to.trim());
            if to.is_absolute() && to.is_dir() {
                return Some(to);
            }
        }
    }

    let base = dirs::data_dir()?;
    let new_path = base.join("wtf.tonho.omniget");
    let old_path = base.join("omniget");

    if old_path.exists() {
        let _ = std::fs::create_dir_all(&new_path);

        let src = old_path.join("bin");
        let dst = new_path.join("bin");
        if src.exists() && !dst.exists() {
            let _ = copy_dir_recursive(&src, &dst);
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

pub fn data_dir_redirect_file() -> Option<std::path::PathBuf> {
    Some(dirs::data_dir()?.join("wtf.tonho.omniget.location"))
}

pub const DEV_REAL_PROFILE_ENV: &str = "OMNIGET_DEV_REAL_PROFILE";

pub const DEV_PROFILE_DIR_NAME: &str = "omniget-dev";

pub fn dev_profile_dir(
    data_dir_set: bool,
    portable: bool,
    real_profile: Option<&str>,
    platform_data_dir: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    if data_dir_set || portable || real_profile.map(str::trim) == Some("1") {
        return None;
    }
    platform_data_dir.map(|d| d.join(DEV_PROFILE_DIR_NAME))
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

fn path_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() || matches!(c, b'"' | b'\'' | b'`' | b')' | b'>' | b',' | b';') {
            break;
        }
        if c == b':' && bytes.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) {
            break;
        }
        i += 1;
    }
    i
}

pub fn rebase_paths(text: &str, root: &std::path::Path, label: &str) -> String {
    let mut out = text.to_string();
    for s in spellings(root) {
        for sep in ['/', '\\'] {
            out = out.replace(&format!("{s}{sep}"), label);
        }
    }
    out
}

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
