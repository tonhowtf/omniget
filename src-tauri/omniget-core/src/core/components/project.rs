use std::path::{Component, Path, PathBuf};

use serde::Serialize;

pub const LOCKFILES: &[&str] = &[
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lock",
    "bun.lockb",
    "package-lock.json",
];

const CSS_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProjectInfo {
    pub path: String,
    pub name: String,
    pub has_package_json: bool,
    pub lockfiles: Vec<String>,
    pub has_components_json: bool,
    pub components_json_error: bool,
    pub css_path: Option<String>,
    pub css_exists: bool,
    pub tailwind_config: Option<String>,
    pub yarn_classic: bool,
}

pub fn yarn_classic(root: &Path) -> bool {
    let manager = std::fs::read(root.join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| {
            v.get("packageManager")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        });
    if let Some(version) = manager.as_deref().and_then(|m| m.strip_prefix("yarn@")) {
        return version.starts_with("1.") || version == "1";
    }
    if root.join(".yarnrc.yml").is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(root.join("yarn.lock")) else {
        return false;
    };
    let mut head = Vec::with_capacity(4096);
    use std::io::Read;
    if file.take(4096).read_to_end(&mut head).is_err() {
        return false;
    }
    !String::from_utf8_lossy(&head).contains("__metadata:")
}

pub fn safe_relative(root: &Path, rel: &str) -> Result<PathBuf, &'static str> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        return Err("css_path_invalid");
    }
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("css_path_outside");
    }
    if rel_path.extension().and_then(|e| e.to_str()) != Some("css") {
        return Err("css_path_not_css");
    }
    let joined = root.join(rel_path);
    let root_real = std::fs::canonicalize(root).map_err(|_| "project_missing")?;
    let parent = joined.parent().ok_or("css_path_invalid")?;
    let parent_real = std::fs::canonicalize(parent).map_err(|_| "css_path_missing")?;
    if !parent_real.starts_with(&root_real) {
        return Err("css_path_outside");
    }
    if let Ok(meta) = std::fs::symlink_metadata(&joined) {
        if meta.file_type().is_symlink() {
            let real = std::fs::canonicalize(&joined).map_err(|_| "css_path_missing")?;
            if !real.starts_with(&root_real) {
                return Err("css_path_outside");
            }
        }
    }
    Ok(joined)
}

pub fn inspect(root: &Path) -> Result<ProjectInfo, &'static str> {
    if !root.is_dir() {
        return Err("project_missing");
    }
    let lockfiles = LOCKFILES
        .iter()
        .filter(|f| root.join(f).is_file())
        .map(|f| f.to_string())
        .collect();
    let components_path = root.join("components.json");
    let has_components_json = components_path.is_file();
    let mut components_json_error = false;
    let mut css_path = None;
    let mut tailwind_config = None;
    if has_components_json {
        match std::fs::read(&components_path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        {
            Some(v) => {
                let tw = v.get("tailwind");
                css_path = tw
                    .and_then(|t| t.get("css"))
                    .and_then(|c| c.as_str())
                    .filter(|c| safe_relative(root, c).is_ok())
                    .map(|c| c.trim_start_matches("./").to_string());
                tailwind_config = tw
                    .and_then(|t| t.get("config"))
                    .and_then(|c| c.as_str())
                    .filter(|c| !c.is_empty())
                    .map(|c| c.to_string());
            }
            None => components_json_error = true,
        }
    }
    let css_exists = css_path
        .as_deref()
        .map(|c| root.join(c).is_file())
        .unwrap_or(false);
    Ok(ProjectInfo {
        path: root.to_string_lossy().to_string(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        has_package_json: root.join("package.json").is_file(),
        lockfiles,
        has_components_json,
        components_json_error,
        css_path,
        css_exists,
        tailwind_config,
        yarn_classic: yarn_classic(root),
    })
}

pub fn read_css(root: &Path, rel: &str) -> Result<String, &'static str> {
    let path = safe_relative(root, rel)?;
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(_) => return Err("css_read_failed"),
    };
    if meta.len() > CSS_LIMIT {
        return Err("css_too_large");
    }
    std::fs::read_to_string(&path).map_err(|_| "css_read_failed")
}

pub fn write_css(
    root: &Path,
    rel: &str,
    expected: &str,
    content: &str,
) -> Result<String, &'static str> {
    if content.len() as u64 > CSS_LIMIT {
        return Err("css_too_large");
    }
    let current = read_css(root, rel)?;
    if current != expected {
        return Err("css_changed");
    }
    let path = safe_relative(root, rel)?;
    let tmp = path.with_extension("css.omniget-tmp");
    std::fs::write(&tmp, content).map_err(|_| "css_write_failed")?;
    std::fs::rename(&tmp, &path).map_err(|_| {
        let _ = std::fs::remove_file(&tmp);
        "css_write_failed"
    })?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_yarn_classic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!yarn_classic(dir.path()));
        std::fs::write(
            dir.path().join("yarn.lock"),
            "# yarn lockfile v1\n\nclsx@^2:\n  version \"2.1.1\"\n",
        )
        .unwrap();
        assert!(yarn_classic(dir.path()));
        std::fs::write(dir.path().join("yarn.lock"), "__metadata:\n  version: 8\n").unwrap();
        assert!(!yarn_classic(dir.path()));
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"packageManager":"yarn@1.22.22"}"#,
        )
        .unwrap();
        assert!(yarn_classic(dir.path()));
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"packageManager":"yarn@4.5.0"}"#,
        )
        .unwrap();
        assert!(!yarn_classic(dir.path()));
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("app")).unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        std::fs::write(
            dir.path().join("components.json"),
            r#"{"tailwind":{"css":"app/globals.css","config":""}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("app/globals.css"), ":root {}\n").unwrap();
        dir
    }

    #[test]
    fn inspect_reads_markers() {
        let dir = project();
        let info = inspect(dir.path()).unwrap();
        assert!(info.has_package_json);
        assert!(info.has_components_json);
        assert_eq!(info.lockfiles, vec!["pnpm-lock.yaml".to_string()]);
        assert_eq!(info.css_path.as_deref(), Some("app/globals.css"));
        assert!(info.css_exists);
        assert!(info.tailwind_config.is_none());
    }

    #[test]
    fn inspect_without_components_json() {
        let dir = tempfile::tempdir().unwrap();
        let info = inspect(dir.path()).unwrap();
        assert!(!info.has_components_json);
        assert!(info.lockfiles.is_empty());
        assert!(inspect(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn relative_paths_stay_inside() {
        let dir = project();
        assert!(safe_relative(dir.path(), "app/globals.css").is_ok());
        assert!(safe_relative(dir.path(), "./app/globals.css").is_ok());
        assert_eq!(
            safe_relative(dir.path(), "../x.css").unwrap_err(),
            "css_path_outside"
        );
        assert_eq!(
            safe_relative(dir.path(), "/etc/x.css").unwrap_err(),
            "css_path_outside"
        );
        assert_eq!(
            safe_relative(dir.path(), "app/../../x.css").unwrap_err(),
            "css_path_outside"
        );
        assert_eq!(
            safe_relative(dir.path(), "app/globals.js").unwrap_err(),
            "css_path_not_css"
        );
    }

    #[test]
    fn write_requires_unchanged_content() {
        let dir = project();
        assert_eq!(
            write_css(dir.path(), "app/globals.css", "other", ":root{}").unwrap_err(),
            "css_changed"
        );
        write_css(
            dir.path(),
            "app/globals.css",
            ":root {}\n",
            ":root { --a: 1; }\n",
        )
        .unwrap();
        assert_eq!(
            read_css(dir.path(), "app/globals.css").unwrap(),
            ":root { --a: 1; }\n"
        );
    }
}
