use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{self, SkillManifest, SKILL_FILE};
use super::scan::ScanStatus;
use super::{
    SkillError, SkillSource, ERR_SKILL_GIT, ERR_SKILL_NOT_FOUND, ERR_SKILL_PARSE, ERR_SKILL_PATH,
    ERR_SKILL_TOO_BIG,
};

pub const SIDECAR: &str = ".omniget-skill.json";
pub const MAX_FILES: usize = 2_000;
pub const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_SEARCH_DEPTH: usize = 3;
pub const PENDING_PREFIX: &str = ".pending-";
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
const GIT_POLL: std::time::Duration = std::time::Duration::from_millis(25);
const GIT_STDERR_MAX: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default)]
    pub installed_at: String,
    #[serde(default)]
    pub scan: ScanStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub manifest: SkillManifest,
    pub scan: ScanStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_confirm: Option<String>,
}

impl InstallOutcome {
    pub fn is_pending(&self) -> bool {
        self.needs_confirm.is_some()
    }
}

pub type ScanFn<'a> = dyn Fn(&Path) -> ScanStatus + 'a;

fn default_scan(dir: &Path) -> ScanStatus {
    super::scan::scan_dir(dir)
}

pub fn skills_dir() -> Result<PathBuf, SkillError> {
    let base = crate::core::paths::app_data_dir().ok_or_else(|| {
        SkillError::new(
            ERR_SKILL_PATH,
            "no application data directory on this system",
        )
    })?;
    Ok(base.join("llm").join("skills"))
}

fn ensure_root(root: &Path) -> Result<(), SkillError> {
    std::fs::create_dir_all(root)
        .map_err(|e| SkillError::io(&format!("creating {}", root.display()), &e))
}

pub fn install_from_dir_in_with(
    root: &Path,
    src: &Path,
    scan: &ScanFn<'_>,
) -> Result<InstallOutcome, SkillError> {
    ensure_root(root)?;
    if is_inside(root, src) {
        return Err(SkillError::new(
            ERR_SKILL_PATH,
            format!("{} is already inside the skills directory", src.display()),
        ));
    }
    let source = SkillSource::Dir {
        from: src.display().to_string(),
    };
    install_copy(root, src, source, scan)
}

pub fn install_from_git_with(
    root: &Path,
    program: &str,
    url: &str,
    rev: Option<&str>,
    subdir: Option<&str>,
) -> Result<InstallOutcome, SkillError> {
    install_from_git_with_scan(root, program, url, rev, subdir, &default_scan)
}

pub fn install_from_git_with_scan(
    root: &Path,
    program: &str,
    url: &str,
    rev: Option<&str>,
    subdir: Option<&str>,
    scan: &ScanFn<'_>,
) -> Result<InstallOutcome, SkillError> {
    ensure_root(root)?;
    check_git_url(url)?;
    if let Some(rev) = rev {
        check_git_rev(rev)?;
    }
    let clone = TempDir::new(root, "staging-git")?;
    let mut args: Vec<String> = vec![
        "-c".into(),
        "protocol.ext.allow=never".into(),
        "clone".into(),
        "--depth".into(),
        "1".into(),
        "--single-branch".into(),
        "--no-tags".into(),
        "--filter=blob:limit=32m".into(),
    ];
    if let Some(rev) = rev {
        if !is_full_sha(rev) {
            args.push("--branch".into());
            args.push(rev.to_string());
        }
    }
    args.push("--".into());
    args.push(url.to_string());
    args.push(clone.path().display().to_string());
    run_git(program, &args, None)?;

    if let Some(rev) = rev {
        if is_full_sha(rev) {
            run_git(
                program,
                &[
                    "fetch".to_string(),
                    "--depth".to_string(),
                    "1".to_string(),
                    "origin".to_string(),
                    rev.to_string(),
                ],
                Some(clone.path()),
            )?;
            run_git(
                program,
                &[
                    "checkout".to_string(),
                    "--detach".to_string(),
                    "FETCH_HEAD".to_string(),
                ],
                Some(clone.path()),
            )?;
        }
    }

    let src = match subdir {
        Some(sub) => {
            let joined = join_inside(clone.path(), sub)?;
            if !joined.join(SKILL_FILE).is_file() {
                find_skill_dir(&joined)?
            } else {
                joined
            }
        }
        None => find_skill_dir(clone.path())?,
    };
    let source = SkillSource::Git {
        url: url.to_string(),
        rev: rev.map(str::to_string),
        subdir: subdir.map(str::to_string),
    };
    install_copy(root, &src, source, scan)
}

pub fn list_with_errors_in(root: &Path) -> (Vec<SkillManifest>, Vec<(String, SkillError)>) {
    let mut ok: Vec<SkillManifest> = Vec::new();
    let mut bad: Vec<(String, SkillError)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return (ok, bad);
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        match manifest::parse(&entry.path()) {
            Ok(m) => ok.push(m),
            Err(e) => bad.push((name, e)),
        }
    }
    ok.sort_by(|a, b| a.name.cmp(&b.name));
    bad.sort_by(|a, b| a.0.cmp(&b.0));
    (ok, bad)
}

pub fn skill_path(root: &Path, name: &str) -> Result<PathBuf, SkillError> {
    let name = manifest::validate_name(name)?;
    let dir = root.join(&name);
    if !dir.is_dir() {
        return Err(SkillError::new(
            ERR_SKILL_NOT_FOUND,
            format!("no skill named `{name}` is installed"),
        ));
    }
    Ok(dir)
}

fn install_copy(
    root: &Path,
    src: &Path,
    source: SkillSource,
    scan: &ScanFn<'_>,
) -> Result<InstallOutcome, SkillError> {
    let parsed = manifest::parse(src)?;
    let staging = TempDir::new(root, "staging")?;
    let mut budget = Budget::default();
    copy_tree(src, staging.path(), &mut budget)?;

    let verdict = scan(staging.path());

    let sidecar = Sidecar {
        source,
        installed_at: chrono::Utc::now().to_rfc3339(),
        scan: verdict.clone(),
    };
    let json = serde_json::to_vec_pretty(&sidecar)
        .map_err(|e| SkillError::new(ERR_SKILL_PARSE, format!("writing the sidecar: {e}")))?;
    std::fs::write(staging.path().join(SIDECAR), json)
        .map_err(|e| SkillError::io("writing the install sidecar", &e))?;

    if verdict.is_high_risk() || matches!(verdict, ScanStatus::Failed { .. }) {
        let token = format!("{PENDING_PREFIX}{}", uuid::Uuid::new_v4());
        let parked = root.join(&token);
        std::fs::rename(staging.path(), &parked).map_err(|e| {
            SkillError::io(
                &format!("parking the staged skill at {}", parked.display()),
                &e,
            )
        })?;
        staging.forget();
        return Ok(InstallOutcome {
            manifest: manifest::parse(&parked)?,
            scan: verdict,
            needs_confirm: Some(token),
        });
    }

    let dest = root.join(&parsed.name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| SkillError::io(&format!("replacing {}", dest.display()), &e))?;
    }
    std::fs::rename(staging.path(), &dest).map_err(|e| {
        SkillError::io(
            &format!("moving the staged skill to {}", dest.display()),
            &e,
        )
    })?;
    staging.forget();
    Ok(InstallOutcome {
        manifest: manifest::parse(&dest)?,
        scan: verdict,
        needs_confirm: None,
    })
}

#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

impl Budget {
    fn take(&mut self, bytes: u64) -> Result<(), SkillError> {
        self.files += 1;
        self.bytes += bytes;
        if self.files > MAX_FILES {
            return Err(SkillError::new(
                ERR_SKILL_TOO_BIG,
                format!("a skill may not carry more than {MAX_FILES} files"),
            ));
        }
        if self.bytes > MAX_TOTAL_BYTES {
            return Err(too_many_bytes());
        }
        Ok(())
    }
}

fn too_many_bytes() -> SkillError {
    SkillError::new(
        ERR_SKILL_TOO_BIG,
        format!("a skill may not carry more than {MAX_TOTAL_BYTES} bytes"),
    )
}

fn copy_tree(src: &Path, dst: &Path, budget: &mut Budget) -> Result<(), SkillError> {
    std::fs::create_dir_all(dst)
        .map_err(|e| SkillError::io(&format!("creating {}", dst.display()), &e))?;
    let entries = std::fs::read_dir(src)
        .map_err(|e| SkillError::io(&format!("reading {}", src.display()), &e))?;
    for entry in entries {
        let entry = entry.map_err(|e| SkillError::io("reading a directory entry", &e))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".git" || name_str == SIDECAR {
            continue;
        }
        let from = entry.path();
        let meta = std::fs::symlink_metadata(&from)
            .map_err(|e| SkillError::io(&format!("reading {}", from.display()), &e))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let to = dst.join(&name);
        if meta.is_dir() {
            copy_tree(&from, &to, budget)?;
        } else {
            budget.take(meta.len())?;
            std::fs::copy(&from, &to)
                .map_err(|e| SkillError::io(&format!("copying {}", from.display()), &e))?;
        }
    }
    Ok(())
}

fn find_skill_dir(root: &Path) -> Result<PathBuf, SkillError> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..=MAX_SEARCH_DEPTH {
        let mut hits: Vec<PathBuf> = level
            .iter()
            .filter(|dir| dir.join(SKILL_FILE).is_file())
            .cloned()
            .collect();
        hits.sort();
        if let Some(first) = hits.first() {
            return Ok(first.clone());
        }
        let mut next: Vec<PathBuf> = Vec::new();
        for dir in &level {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name == ".git" {
                        continue;
                    }
                    next.push(entry.path());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        level = next;
    }
    Err(SkillError::new(
        ERR_SKILL_PARSE,
        format!("no {SKILL_FILE} within {MAX_SEARCH_DEPTH} levels of the source"),
    ))
}

pub fn join_inside(base: &Path, rel: &str) -> Result<PathBuf, SkillError> {
    let rel = rel.trim_matches('/');
    let mut out = base.to_path_buf();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => {
                return Err(SkillError::new(
                    ERR_SKILL_PATH,
                    format!("`{rel}` walks out of {}", base.display()),
                ))
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

fn is_inside(root: &Path, candidate: &Path) -> bool {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let candidate = std::fs::canonicalize(candidate).unwrap_or_else(|_| candidate.to_path_buf());
    candidate.starts_with(&root)
}

fn is_full_sha(rev: &str) -> bool {
    rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn check_git_url(url: &str) -> Result<(), SkillError> {
    let bad = |why: &str| SkillError::new(ERR_SKILL_GIT, format!("`{url}` was refused: {why}"));
    let url = url.trim();
    if url.is_empty() {
        return Err(bad("empty URL"));
    }
    if url.starts_with('-') {
        return Err(bad("starts with a dash"));
    }
    if url.contains(['\n', '\r', '\0']) {
        return Err(bad("contains a control character"));
    }
    if url.starts_with("ext::") || url.contains("::") {
        return Err(bad("uses a git helper transport"));
    }
    const SCHEMES: [&str; 5] = ["https://", "http://", "git://", "ssh://", "file://"];
    if SCHEMES.iter().any(|s| url.starts_with(s)) {
        return Ok(());
    }
    let scp_like = url
        .split_once('@')
        .is_some_and(|(user, rest)| !user.is_empty() && rest.contains(':'));
    if scp_like || Path::new(url).is_absolute() {
        return Ok(());
    }
    Err(bad("not an https, ssh, git, file or absolute-path remote"))
}

fn check_git_rev(rev: &str) -> Result<(), SkillError> {
    let ok = !rev.is_empty()
        && !rev.starts_with('-')
        && rev
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if ok {
        Ok(())
    } else {
        Err(SkillError::new(
            ERR_SKILL_GIT,
            format!("`{rev}` is not a branch, tag or commit name"),
        ))
    }
}

fn run_git(program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), SkillError> {
    run_git_within(program, args, cwd, GIT_TIMEOUT)
}

fn run_git_within(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout: std::time::Duration,
) -> Result<(), SkillError> {
    use std::io::Read;
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GCM_INTERACTIVE", "never")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn()).map_err(|e| {
        SkillError::new(
            ERR_SKILL_GIT,
            format!("could not run `{program}`: {e}. Install git and try again."),
        )
    })?;
    let err_thread = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let _ = (&mut pipe).take(GIT_STDERR_MAX).read_to_end(&mut kept);
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            kept
        })
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                super::scan::kill_group(&mut child);
                return Err(SkillError::new(
                    ERR_SKILL_GIT,
                    format!(
                        "`{program} {}` did not finish within {}s",
                        args.join(" "),
                        timeout.as_secs()
                    ),
                ));
            }
            Ok(None) => std::thread::sleep(GIT_POLL),
            Err(e) => {
                super::scan::kill_group(&mut child);
                return Err(SkillError::new(
                    ERR_SKILL_GIT,
                    format!("waiting for `{program}` failed: {e}"),
                ));
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    let stderr = err_thread.and_then(|t| t.join().ok()).unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr);
    let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
    Err(SkillError::new(
        ERR_SKILL_GIT,
        format!("`{program} {}` failed: {tail}", args.join(" ")),
    ))
}

struct TempDir {
    path: PathBuf,
    keep: bool,
}

impl TempDir {
    fn new(root: &Path, prefix: &str) -> Result<Self, SkillError> {
        let path = root.join(format!(".{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path)
            .map_err(|e| SkillError::io(&format!("creating {}", path.display()), &e))?;
        Ok(Self { path, keep: false })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn forget(mut self) {
        self.keep = true;
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::path::PathBuf;

    pub fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-skills-{tag}-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_src(base: &Path) -> PathBuf {
        let src = base.join("src").join("next-book");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join(SKILL_FILE),
            "---\nname: next-book\ndescription: Picks the next book for the reader.\n---\n\nBody\n",
        )
        .unwrap();
        src
    }

    #[test]
    fn a_failed_scan_parks_the_skill_for_confirmation() {
        let base = tests_support::temp_dir("install-scan-failed");
        let root = base.join("skills");
        let src = skill_src(&base);
        let outcome = install_from_dir_in_with(&root, &src, &|_| ScanStatus::Failed {
            reason: "the scanner did not finish within 120s".into(),
        })
        .unwrap();
        assert!(outcome.is_pending());
        assert!(!root.join("next-book").exists());
        let token = outcome.needs_confirm.unwrap();
        assert!(token.starts_with(PENDING_PREFIX));
        assert!(root.join(&token).join(SKILL_FILE).is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_skill_that_was_not_scanned_installs_directly() {
        let base = tests_support::temp_dir("install-not-scanned");
        let root = base.join("skills");
        let src = skill_src(&base);
        let outcome = install_from_dir_in_with(&root, &src, &|_| ScanStatus::NotScanned).unwrap();
        assert!(!outcome.is_pending());
        assert!(root.join("next-book").join(SKILL_FILE).is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn a_git_that_hangs_is_killed_at_the_deadline() {
        let base = tests_support::temp_dir("install-git-hang");
        let git = base.join("git");
        crate::core::process::test_exec::script(&git, "sleep 30\n");
        let started = std::time::Instant::now();
        let err = run_git_within(
            &git.display().to_string(),
            &["clone".to_string()],
            None,
            std::time::Duration::from_millis(300),
        )
        .unwrap_err();
        assert_eq!(err.code(), ERR_SKILL_GIT);
        assert!(err.message.contains("did not finish"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn the_clone_limits_the_blob_size() {
        let base = tests_support::temp_dir("install-git-filter");
        let git = base.join("git");
        crate::core::process::test_exec::script(
            &git,
            "echo \"$@\" > \"$0.args\"\necho refused >&2\nexit 1\n",
        );
        let err = install_from_git_with(
            &base.join("skills"),
            &git.display().to_string(),
            "https://example.com/owner/skill.git",
            None,
            None,
        )
        .unwrap_err();
        assert!(err.message.contains("refused"), "{err}");
        let args = std::fs::read_to_string(base.join("git.body.args")).unwrap();
        assert!(args.contains("--filter=blob:limit=32m"), "{args}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
