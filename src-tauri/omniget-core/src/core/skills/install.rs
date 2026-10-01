//! Installing, listing and removing skills.
//!
//! Every skill lives at `<app_data>/llm/skills/<name>/`, where `<name>` is the
//! validated `name` of its frontmatter — never the name of the folder it came
//! from, so two zips that both unpack to `skill/` cannot collide.
//!
//! Three sources, all local work:
//! - a folder the user picked;
//! - a zip the user picked (entries that escape the destination are refused,
//!   with [`super::ERR_SKILL_ZIP`]);
//! - a git repository, cloned shallow with the system `git` (never a library,
//!   never a credential of ours). The UI must confirm with the user before
//!   calling [`install_from_git`]: it is the only path here that touches the
//!   network, and it runs a binary.
//!
//! Installs are staged: everything is written to `<root>/.staging-*` first and
//! only then renamed over the destination, so a failure halfway never leaves a
//! half-installed skill behind.
//!
//! The staging step is also where the security scan happens ([`super::scan`]):
//! the scanner needs files on disk, and the point of scanning is to decide
//! whether these files may become an installed skill at all. A scan that comes
//! back over SkillSpector's own threshold does not fail the install — it parks
//! it in `<root>/.pending-*`, hands the caller a token, and waits for the user
//! to [`confirm_install`] or [`discard_install`]. A parked skill is in no list
//! and in no prompt: the dot prefix keeps it out of [`list_in`].

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{self, SkillManifest, SKILL_FILE};
use super::scan::ScanStatus;
use super::{
    SkillError, SkillSource, ERR_SKILL_GIT, ERR_SKILL_NOT_FOUND, ERR_SKILL_PARSE, ERR_SKILL_PATH,
    ERR_SKILL_PENDING, ERR_SKILL_TOO_BIG, ERR_SKILL_ZIP,
};

/// Sidecar written next to `SKILL.md` with the origin of the install. The dot
/// prefix keeps it out of the way of the skill's own files.
pub const SIDECAR: &str = ".omniget-skill.json";
/// Most files one skill may carry.
pub const MAX_FILES: usize = 2_000;
/// Most bytes one skill may carry, uncompressed.
pub const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
/// How deep we look for the `SKILL.md` inside an archive or a clone.
pub const MAX_SEARCH_DEPTH: usize = 3;
/// Prefix of a quarantined install, waiting for the user's second click.
pub const PENDING_PREFIX: &str = ".pending-";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sidecar {
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default)]
    pub installed_at: String,
    /// What the scanner said about this copy. `NotScanned` for a sidecar
    /// written before we scanned, which is exactly what it means.
    #[serde(default)]
    pub scan: ScanStatus,
}

/// What one install came to. The skill is installed unless `needs_confirm`
/// carries a token, in which case it is parked and `manifest.path` points at
/// the quarantine folder, not at an installed skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub manifest: SkillManifest,
    pub scan: ScanStatus,
    /// Token for [`confirm_install`] / [`discard_install`]. `None` when the
    /// skill went straight in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_confirm: Option<String>,
}

impl InstallOutcome {
    /// The manifest, for a caller that does not care about the verdict. Panics
    /// on nothing: a parked install still has a manifest.
    pub fn into_manifest(self) -> SkillManifest {
        self.manifest
    }

    /// True when the skill is parked and not installed.
    pub fn is_pending(&self) -> bool {
        self.needs_confirm.is_some()
    }
}

/// Signature of the scan step, so a test can install without a scanner and
/// without the machine's `PATH` deciding the outcome.
pub type ScanFn<'a> = dyn Fn(&Path) -> ScanStatus + 'a;

/// The scan we use in production: SkillSpector when it is installed, and
/// [`ScanStatus::NotScanned`] when it is not.
fn default_scan(dir: &Path) -> ScanStatus {
    super::scan::scan_dir(dir)
}

/// `<app_data>/llm/skills`. Honours `OMNIGET_DATA_DIR` like the rest of the app.
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

/// Install a folder into the default root.
pub fn install_from_dir(src: &Path) -> Result<InstallOutcome, SkillError> {
    install_from_dir_in(&skills_dir()?, src)
}

/// Install a folder into `root`. The folder is copied, not moved or linked.
pub fn install_from_dir_in(root: &Path, src: &Path) -> Result<InstallOutcome, SkillError> {
    install_from_dir_in_with(root, src, &default_scan)
}

/// [`install_from_dir_in`] with the scan step spelled out.
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

/// Install from a zip into the default root.
pub fn install_from_zip(archive: &Path) -> Result<InstallOutcome, SkillError> {
    install_from_zip_in(&skills_dir()?, archive)
}

/// Install from a zip into `root`. An entry whose path escapes the destination
/// (`../`, absolute, drive-relative) fails the whole install with
/// [`super::ERR_SKILL_ZIP`]; nothing is kept.
pub fn install_from_zip_in(root: &Path, archive: &Path) -> Result<InstallOutcome, SkillError> {
    install_from_zip_in_with(root, archive, &default_scan)
}

/// [`install_from_zip_in`] with the scan step spelled out.
pub fn install_from_zip_in_with(
    root: &Path,
    archive: &Path,
    scan: &ScanFn<'_>,
) -> Result<InstallOutcome, SkillError> {
    ensure_root(root)?;
    let staging = TempDir::new(root, "staging-zip")?;
    extract_zip(archive, staging.path())?;
    let dir = find_skill_dir(staging.path())?;
    let source = SkillSource::Zip {
        from: archive.display().to_string(),
    };
    install_copy(root, &dir, source, scan)
}

/// Clone `url` shallow with the system `git` and install `subdir` (or the
/// repository root) into the default root. Confirm with the user first.
pub fn install_from_git(
    url: &str,
    rev: Option<&str>,
    subdir: Option<&str>,
) -> Result<InstallOutcome, SkillError> {
    install_from_git_in(&skills_dir()?, url, rev, subdir)
}

/// [`install_from_git`] against an explicit root.
pub fn install_from_git_in(
    root: &Path,
    url: &str,
    rev: Option<&str>,
    subdir: Option<&str>,
) -> Result<InstallOutcome, SkillError> {
    install_from_git_with(root, "git", url, rev, subdir)
}

/// Same, with the `git` program spelled out. Tests use it to prove the missing
/// `git` path; production always passes `"git"`.
pub fn install_from_git_with(
    root: &Path,
    program: &str,
    url: &str,
    rev: Option<&str>,
    subdir: Option<&str>,
) -> Result<InstallOutcome, SkillError> {
    install_from_git_with_scan(root, program, url, rev, subdir, &default_scan)
}

/// [`install_from_git_with`] with the scan step spelled out.
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
    // `--` keeps a URL that starts with a dash from being read as an option,
    // and the check above already refused one.
    let mut args: Vec<String> = vec![
        "-c".into(),
        "protocol.ext.allow=never".into(),
        "clone".into(),
        "--depth".into(),
        "1".into(),
        "--single-branch".into(),
        "--no-tags".into(),
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

/// Every skill installed in the default root, sorted by name. Folders that do
/// not parse are skipped; use [`list_with_errors_in`] to show them.
pub fn list() -> Vec<SkillManifest> {
    skills_dir().map(|root| list_in(&root)).unwrap_or_default()
}

/// Every skill installed under `root`, sorted by name.
pub fn list_in(root: &Path) -> Vec<SkillManifest> {
    list_with_errors_in(root).0
}

/// Installed skills plus the folders that failed to parse, so the UI can offer
/// to remove them instead of pretending they are not there.
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

/// Remove an installed skill from the default root.
pub fn remove(name: &str) -> Result<(), SkillError> {
    remove_in(&skills_dir()?, name)
}

/// Remove an installed skill from `root`. The name is validated first, so a
/// caller can never delete `..` or an absolute path.
pub fn remove_in(root: &Path, name: &str) -> Result<(), SkillError> {
    let name = manifest::validate_name(name)?;
    let dir = root.join(&name);
    if !dir.is_dir() {
        return Err(SkillError::new(
            ERR_SKILL_NOT_FOUND,
            format!("no skill named `{name}` is installed"),
        ));
    }
    std::fs::remove_dir_all(&dir)
        .map_err(|e| SkillError::io(&format!("removing {}", dir.display()), &e))
}

/// Path of one installed skill, with the name validated.
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

// ---------------------------------------------------------------- internals

/// Parse, copy into staging, scan the staged copy, then either rename over
/// `<root>/<name>` or park it. Reinstalling an existing name replaces it — but
/// only once the install is committed, so a parked risky install never removes
/// the copy the user already trusts.
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

    // Scan before the sidecar is written, so the scanner sees the skill as the
    // agent will see it and not a folder with a file of ours in it.
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

    // Only a scan that ran and came back over the threshold parks the install.
    // No scanner, a crashed scanner or an unreadable report all install:
    // an advisory tool being absent must not become a wall.
    if verdict.is_high_risk() {
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

/// Turns a `token` back into the quarantine folder it names, refusing anything
/// that is not one of ours. A token is a folder name and nothing else: no
/// separator, no `..`, and it has to carry [`PENDING_PREFIX`].
fn pending_path(root: &Path, token: &str) -> Result<PathBuf, SkillError> {
    let bad = |why: &str| SkillError::new(ERR_SKILL_PENDING, format!("`{token}` {why}"));
    if !token.starts_with(PENDING_PREFIX) {
        return Err(bad("does not name a pending install"));
    }
    if token.contains('/') || token.contains('\\') || token.contains("..") || token.contains('\0') {
        return Err(bad("is not a plain folder name"));
    }
    let dir = root.join(token);
    if !dir.is_dir() {
        return Err(bad("is not waiting to be confirmed"));
    }
    Ok(dir)
}

/// Install a skill the user confirmed after seeing its scan. In the default
/// root.
pub fn confirm_install(token: &str) -> Result<SkillManifest, SkillError> {
    confirm_install_in(&skills_dir()?, token)
}

/// [`confirm_install`] against an explicit root. The parked copy is moved into
/// place unchanged: nothing is re-scanned and nothing is re-fetched, so what
/// the user approved is exactly what lands.
pub fn confirm_install_in(root: &Path, token: &str) -> Result<SkillManifest, SkillError> {
    let parked = pending_path(root, token)?;
    let parsed = manifest::parse(&parked)?;
    let dest = root.join(&parsed.name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| SkillError::io(&format!("replacing {}", dest.display()), &e))?;
    }
    std::fs::rename(&parked, &dest).map_err(|e| {
        SkillError::io(
            &format!("moving the confirmed skill to {}", dest.display()),
            &e,
        )
    })?;
    manifest::parse(&dest)
}

/// Throw a parked install away. In the default root.
pub fn discard_install(token: &str) -> Result<(), SkillError> {
    discard_install_in(&skills_dir()?, token)
}

/// [`discard_install`] against an explicit root.
pub fn discard_install_in(root: &Path, token: &str) -> Result<(), SkillError> {
    let parked = pending_path(root, token)?;
    std::fs::remove_dir_all(&parked)
        .map_err(|e| SkillError::io(&format!("discarding {}", parked.display()), &e))
}

/// Installs parked under `root`, newest folder first is not guaranteed — they
/// are sorted by token. The UI uses this to pick up a decision the user left
/// open when the app closed, instead of leaving folders nobody will ever name.
pub fn pending_in(root: &Path) -> Vec<(String, SkillManifest)> {
    let mut out: Vec<(String, SkillManifest)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let token = entry.file_name().to_string_lossy().to_string();
        if !token.starts_with(PENDING_PREFIX) {
            continue;
        }
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Ok(parsed) = manifest::parse(&entry.path()) {
            out.push((token, parsed));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
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

/// Copy `src` into `dst`, skipping `.git`, our own sidecar and every symlink.
/// Symlinks are dropped rather than followed: a skill has no business pointing
/// at `/etc/passwd`, and following one would also let a cycle run forever.
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

/// The shallowest directory under `root` that holds a `SKILL.md`.
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

fn extract_zip(archive: &Path, dst: &Path) -> Result<(), SkillError> {
    let file = std::fs::File::open(archive)
        .map_err(|e| SkillError::io(&format!("opening {}", archive.display()), &e))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| SkillError::new(ERR_SKILL_ZIP, format!("unreadable zip: {e}")))?;
    let mut budget = Budget::default();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| SkillError::new(ERR_SKILL_ZIP, format!("entry {i}: {e}")))?;
        let raw = entry.name().to_string();
        let rel = safe_zip_path(&raw)?;
        let out = dst.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)
                .map_err(|e| SkillError::io(&format!("creating {}", out.display()), &e))?;
            continue;
        }
        // The declared size is only a hint an archive can lie about: refuse
        // early on it, but charge the budget with what the copy really wrote.
        if budget.bytes.saturating_add(entry.size()) > MAX_TOTAL_BYTES {
            return Err(too_many_bytes());
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SkillError::io(&format!("creating {}", parent.display()), &e))?;
        }
        let mut sink = std::fs::File::create(&out)
            .map_err(|e| SkillError::io(&format!("writing {}", out.display()), &e))?;
        copy_within(&mut entry, &mut sink, &mut budget, &out)?;
    }
    Ok(())
}

/// `io::copy` with the byte ceiling applied to the bytes that actually come
/// out of the reader. A zip header may declare 10 bytes and inflate to 10 GiB;
/// the reader is cut one byte past what is left so the overflow is seen, not
/// silently truncated.
fn copy_within(
    reader: &mut dyn std::io::Read,
    sink: &mut dyn std::io::Write,
    budget: &mut Budget,
    out: &Path,
) -> Result<(), SkillError> {
    use std::io::Read as _;
    let left = MAX_TOTAL_BYTES.saturating_sub(budget.bytes);
    let mut capped = reader.take(left.saturating_add(1));
    let wrote = std::io::copy(&mut capped, sink)
        .map_err(|e| SkillError::io(&format!("writing {}", out.display()), &e))?;
    budget.take(wrote)
}

/// Turns a zip entry name into a relative path that cannot leave the
/// destination, or refuses it. Checked by hand instead of trusting
/// `enclosed_name` alone, so the rejection has our own stable code.
pub fn safe_zip_path(raw: &str) -> Result<PathBuf, SkillError> {
    let bad = |why: &str| {
        SkillError::new(
            ERR_SKILL_ZIP,
            format!("zip entry `{raw}` was refused: {why}"),
        )
    };
    if raw.is_empty() {
        return Err(bad("empty name"));
    }
    if raw.contains('\0') {
        return Err(bad("contains a NUL byte"));
    }
    // Windows separators and drive letters never appear in a well-formed zip,
    // and both are traversal vectors on Windows.
    if raw.contains('\\') {
        return Err(bad("contains a backslash"));
    }
    if raw.starts_with('/') {
        return Err(bad("is absolute"));
    }
    if raw.len() >= 2 && raw.as_bytes()[1] == b':' {
        return Err(bad("has a drive letter"));
    }
    let mut out = PathBuf::new();
    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return Err(bad("walks up out of the destination")),
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(bad("resolves to nothing"));
    }
    // Belt and braces: the same check the standard library would do.
    if out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(bad("is not a plain relative path"));
    }
    Ok(out)
}

/// Joins a caller-supplied relative path onto `base` without letting it escape.
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

/// Only the transports we are willing to hand to `git`, and nothing that could
/// be read as an option.
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
    // scp-like `user@host:path`, and a plain local path for a folder clone.
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
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        // Never let git stop on a credential prompt inside the app.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GCM_INTERACTIVE", "never")
        .stdin(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| {
        SkillError::new(
            ERR_SKILL_GIT,
            format!("could not run `{program}`: {e}. Install git and try again."),
        )
    })?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
    Err(SkillError::new(
        ERR_SKILL_GIT,
        format!("`{program} {}` failed: {tail}", args.join(" ")),
    ))
}

/// A staging directory that deletes itself unless [`TempDir::forget`] is called.
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

    /// A fresh directory under the system temp dir. No `tempfile` dependency in
    /// this crate, and the tests run in parallel, so the name carries a uuid.
    pub fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-skills-{tag}-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}
