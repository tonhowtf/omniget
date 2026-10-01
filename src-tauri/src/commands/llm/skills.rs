//! `llm_*` commands: Agent Skills. Owned by f3-skills-ui.
//!
//! Thin wrappers over `omniget_core::core::skills` (f3-skills-core): the
//! commands do no work of their own beyond turning a `SkillError` into the
//! `"CODE: message"` string the UI maps to an i18n key (`skillErrorKey` in
//! `src/lib/stores/llm-skills-store.svelte.ts`) and adding the `html_url` of a
//! catalog entry, which is a method on the Rust side and not serialised.
//!
//! Budget: every command is one call fired by a mount or by a click. Nothing
//! here holds state, spawns a thread or touches the network — except
//! `llm_skills_install_git` and `llm_skills_install_catalog`, which shell out
//! to `git` for one shallow clone and only ever run from a confirmed click.

use std::path::PathBuf;

use omniget_core::core::skills::{self, catalog, install, scan, InstallOutcome, SkillError};
use serde_json::{json, Value};

fn fail(err: SkillError) -> String {
    err.to_string()
}

/// One install, as the store reads it: `{ manifest, scan, needs_confirm? }`.
///
/// `needs_confirm` is the whole point of the shape. When the scan came back
/// over SkillSpector's threshold the skill is **not** installed: it sits in
/// quarantine and `needs_confirm` carries the token the UI hands back to
/// `llm_skills_confirm_install` or `llm_skills_discard_install` after showing
/// the score. The manifest is there either way so the dialog can name the
/// skill it is asking about.
fn ok(outcome: InstallOutcome) -> Result<Value, String> {
    if !outcome.is_pending() {
        changed(Some(&outcome.manifest.name));
    }
    serde_json::to_value(outcome).map_err(|e| e.to_string())
}

/// After an install, update, removal or repair: record the new hash (bindings
/// follow it) and re-register the skill tools, so the next turn sees exactly
/// what is on disk and no tool outlives its skill. A failure is logged: the
/// files themselves are already in place, and the Skills page shows drift.
fn changed(name: Option<&str>) {
    let projection = omniget_core::core::assist::bots::skills::projection();
    if let Err(e) = omniget_core::core::assist::bots::skills::skills_changed(&projection, name) {
        tracing::warn!("[skills] re-projecting after a change: {e}");
    }
}

/// Installed skills, read from disk, sorted by name. Never fails: a folder
/// that does not parse is left out of the list instead of failing the call.
#[tauri::command]
pub async fn llm_skills_list() -> Result<Value, String> {
    serde_json::to_value(skills::list()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_skills_install_dir(path: String) -> Result<Value, String> {
    ok(skills::install_from_dir(&PathBuf::from(path)).map_err(fail)?)
}

#[tauri::command]
pub async fn llm_skills_install_zip(path: String) -> Result<Value, String> {
    ok(skills::install_from_zip(&PathBuf::from(path)).map_err(fail)?)
}

/// Shallow-clones `url` (at `rev`, when given) and installs the skill at
/// `subdir`, or the repository root when `subdir` is `None`. The install
/// dialog asks for confirmation before calling it: a clone runs code the user
/// has not read.
#[tauri::command]
pub async fn llm_skills_install_git(
    url: String,
    rev: Option<String>,
    subdir: Option<String>,
) -> Result<Value, String> {
    ok(skills::install_from_git(&url, rev.as_deref(), subdir.as_deref()).map_err(fail)?)
}

/// `owner/repo`, `owner/repo/path/to/skill` or `owner/repo@skill-name`: the
/// spelling `npx skills add` takes (`vercel-labs/skills`). Same clone, same
/// scanner and the same quarantine as any git install.
pub fn parse_repo_spec(spec: &str) -> Result<(String, Vec<Option<String>>), String> {
    let spec = spec
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_end_matches(".git")
        .trim_matches('/');
    let (path, named) = match spec.split_once('@') {
        Some((p, n)) => (p, Some(n.trim().to_string())),
        None => (spec, None),
    };
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if parts.len() < 2 || !parts.iter().all(|p| ok(p)) || parts.contains(&"..") {
        return Err(
            "ERR_SKILL_SOURCE: expected owner/repo, owner/repo/path or owner/repo@skill".into(),
        );
    }
    let url = format!("https://github.com/{}/{}.git", parts[0], parts[1]);
    // `tree/<branch>/` shows up when the user pastes a browser URL.
    let rest: Vec<&str> = match parts.get(2) {
        Some(&"tree") | Some(&"blob") => parts.iter().skip(4).copied().collect(),
        _ => parts.iter().skip(2).copied().collect(),
    };
    let subdirs = match (named, rest.is_empty()) {
        (Some(n), _) if ok(&n) => vec![
            Some(format!("skills/{n}")),
            Some(n.clone()),
            Some(format!(".claude/skills/{n}")),
        ],
        (Some(_), _) => return Err("ERR_SKILL_SOURCE: bad skill name".into()),
        (None, false) => vec![Some(rest.join("/"))],
        (None, true) => vec![None],
    };
    Ok((url, subdirs))
}

#[tauri::command]
pub async fn llm_skills_install_repo(spec: String) -> Result<Value, String> {
    let (url, subdirs) = parse_repo_spec(&spec)?;
    tokio::task::spawn_blocking(move || {
        let mut last = None;
        for subdir in subdirs {
            match skills::install_from_git(&url, None, subdir.as_deref()) {
                Ok(outcome) => return ok(outcome),
                Err(e) => last = Some(fail(e)),
            }
        }
        Err(last.unwrap_or_else(|| "ERR_SKILL_SOURCE: nothing to install".to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Installs one entry of the OpenRouterTeam showcase, pinned at
/// [`catalog::COMMIT`]. Separate from `llm_skills_install_git` so the UI never
/// has to carry the repository URL or the pinned commit.
#[tauri::command]
pub async fn llm_skills_install_catalog(name: String) -> Result<Value, String> {
    ok(catalog::install(&name).map_err(fail)?)
}

/// Installs a skill the user confirmed after reading its scan. The parked copy
/// is moved into place as it is: nothing is re-cloned, so what lands is exactly
/// what was scanned and shown.
#[tauri::command]
pub async fn llm_skills_confirm_install(token: String) -> Result<Value, String> {
    let manifest = skills::confirm_install(&token).map_err(fail)?;
    changed(Some(&manifest.name));
    serde_json::to_value(manifest).map_err(|e| e.to_string())
}

/// Throws a parked install away.
#[tauri::command]
pub async fn llm_skills_discard_install(token: String) -> Result<Value, String> {
    skills::discard_install(&token).map_err(fail)?;
    Ok(Value::Null)
}

/// Installs still waiting for a decision, so closing the app mid-dialog does
/// not leave folders nobody can name again.
#[tauri::command]
pub async fn llm_skills_pending() -> Result<Value, String> {
    let root = install::skills_dir().map_err(fail)?;
    let entries: Vec<Value> = install::pending_in(&root)
        .into_iter()
        .map(|(token, manifest)| json!({ "token": token, "manifest": manifest }))
        .collect();
    Ok(Value::Array(entries))
}

/// Whether a scan will run at all on this machine, plus the threshold we ported
/// from SkillSpector. The page says "not scanned" instead of implying safety
/// when this is false.
#[tauri::command]
pub async fn llm_skills_scanner() -> Result<Value, String> {
    Ok(json!({
        "available": scan::is_available(),
        "name": scan::SCANNER_BIN,
        "threshold": scan::RISK_THRESHOLD,
    }))
}

#[tauri::command]
pub async fn llm_skills_remove(name: String) -> Result<Value, String> {
    skills::remove(&name).map_err(fail)?;
    changed(Some(&name));
    Ok(Value::Null)
}

/// Every installed skill with its version state: the hash on disk, the hash
/// OmniGet recorded, `drift` when they differ (edited outside OmniGet), the
/// declared dependencies, and which bots are bound to it.
#[tauri::command]
pub async fn llm_skills_status() -> Result<Value, String> {
    use omniget_core::core::assist::{bots::skills as bot_skills, db};
    let root = install::skills_dir().map_err(fail)?;
    let db = db::global()?;
    let bound: Vec<(String, String)> = db.with(|c| {
        let mut st = c.prepare("SELECT skill, bot_id FROM bots_skill_bindings ORDER BY bot_id")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    })?;
    let (found, bad) = install::list_with_errors_in(&root);
    let mut out = Vec::new();
    for m in found {
        let hash = skills::hash::dir_hash(&m.path).ok();
        let record = bot_skills::install_record(&db, &m.name)?;
        let state = match (&hash, &record) {
            (Some(h), Some(r)) if h == &r.hash => "ok",
            (Some(_), None) => "unrecorded",
            _ => "drift",
        };
        out.push(json!({
            "name": m.name,
            "hash": hash,
            "recorded_hash": record.as_ref().map(|r| r.hash.clone()),
            "version": m.metadata.get("version"),
            "source": m.source,
            "state": state,
            "compatibility": m.compatibility,
            "dependencies": skills::dependencies(&m),
            "bots": bound.iter().filter(|(s, _)| s == &m.name).map(|(_, b)| b.clone()).collect::<Vec<_>>(),
        }));
    }
    for (name, err) in bad {
        out.push(json!({ "name": name, "state": "invalid", "error": err.to_string(),
            "bots": bound.iter().filter(|(s, _)| s == &name).map(|(_, b)| b.clone()).collect::<Vec<_>>() }));
    }
    Ok(Value::Array(out))
}

/// Re-registers the skill tools from what is on disk (the "reload" repair).
#[tauri::command]
pub async fn llm_skills_reproject() -> Result<Value, String> {
    let projection = omniget_core::core::assist::bots::skills::projection();
    serde_json::to_value(projection.reproject()?).map_err(|e| e.to_string())
}

/// Repair for drift, option 1: accept the files on disk as the new version.
#[tauri::command]
pub async fn llm_skills_accept(name: String) -> Result<Value, String> {
    let projection = omniget_core::core::assist::bots::skills::projection();
    let summary = omniget_core::core::assist::bots::skills::accept_current(&projection, &name)?;
    serde_json::to_value(summary).map_err(|e| e.to_string())
}

/// Repair for drift or a removed skill, option 2: install again from the
/// origin recorded at install time (git at the same revision, the same folder
/// or zip). Same scanner and quarantine as a first install.
#[tauri::command]
pub async fn llm_skills_reinstall(name: String) -> Result<Value, String> {
    use omniget_core::core::assist::{bots::skills as bot_skills, db};
    let root = install::skills_dir().map_err(fail)?;
    let source = match install::skill_path(&root, &name) {
        Ok(dir) => skills::manifest::parse(&dir).ok().map(|m| m.source),
        Err(_) => None,
    }
    .filter(|s| !matches!(s, skills::SkillSource::Unknown))
    .or_else(|| {
        db::global()
            .ok()
            .and_then(|db| bot_skills::install_record(&db, &name).ok().flatten())
            .map(|r| r.source)
    });
    let outcome = tokio::task::spawn_blocking(move || match source {
        Some(skills::SkillSource::Git { url, rev, subdir }) => {
            skills::install_from_git(&url, rev.as_deref(), subdir.as_deref())
        }
        Some(skills::SkillSource::Dir { from }) => skills::install_from_dir(&PathBuf::from(from)),
        Some(skills::SkillSource::Zip { from }) => skills::install_from_zip(&PathBuf::from(from)),
        _ => Err(SkillError::new(
            skills::ERR_SKILL_NOT_FOUND,
            "this skill has no recorded origin to reinstall from",
        )),
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(fail)?;
    ok(outcome)
}

/// The pinned `OpenRouterTeam/skills` showcase. Static data: no network, no
/// disk. `html_url` is added here because it is a method on `CatalogEntry`.
#[tauri::command]
pub async fn llm_skills_catalog() -> Result<Value, String> {
    let entries: Vec<Value> = catalog::entries()
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "description": e.description,
                "subdir": e.subdir,
                "skill_md_sha256": e.skill_md_sha256,
                "files": e.files,
                "bytes": e.bytes,
                "html_url": e.html_url(),
                "repo_url": catalog::REPO_HTML_URL,
                "commit": catalog::COMMIT,
                // `null` today: the repository declares no licence (plan D-5).
                // The showcase card warns before installing when this is null.
                "license": catalog::LICENSE,
            })
        })
        .collect();
    Ok(Value::Array(entries))
}
