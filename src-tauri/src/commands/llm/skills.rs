use std::path::PathBuf;

use omniget_core::core::skills::{self, catalog, install, scan, InstallOutcome, SkillError};
use serde_json::{json, Value};

fn fail(err: SkillError) -> String {
    err.to_string()
}

fn ok(outcome: InstallOutcome) -> Result<Value, String> {
    if !outcome.is_pending() {
        changed(Some(&outcome.manifest.name));
    }
    serde_json::to_value(outcome).map_err(|e| e.to_string())
}

fn changed(name: Option<&str>) {
    let projection = omniget_core::core::assist::bots::skills::projection();
    if let Err(e) = omniget_core::core::assist::bots::skills::skills_changed(&projection, name) {
        tracing::warn!("[skills] re-projecting after a change: {e}");
    }
}

#[tauri::command]
pub async fn llm_skills_list() -> Result<Value, String> {
    serde_json::to_value(skills::list()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_skills_install_dir(path: String) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        ok(skills::install_from_dir(&PathBuf::from(path)).map_err(fail)?)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn llm_skills_install_zip(path: String) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        ok(skills::install_from_zip(&PathBuf::from(path)).map_err(fail)?)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn llm_skills_install_git(
    url: String,
    rev: Option<String>,
    subdir: Option<String>,
) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || {
        ok(skills::install_from_git(&url, rev.as_deref(), subdir.as_deref()).map_err(fail)?)
    })
    .await
    .map_err(|e| e.to_string())?
}

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

#[tauri::command]
pub async fn llm_skills_install_catalog(name: String) -> Result<Value, String> {
    tokio::task::spawn_blocking(move || ok(catalog::install(&name).map_err(fail)?))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn llm_skills_confirm_install(token: String) -> Result<Value, String> {
    let manifest = skills::confirm_install(&token).map_err(fail)?;
    changed(Some(&manifest.name));
    serde_json::to_value(manifest).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_skills_discard_install(token: String) -> Result<Value, String> {
    skills::discard_install(&token).map_err(fail)?;
    Ok(Value::Null)
}

#[tauri::command]
pub async fn llm_skills_pending() -> Result<Value, String> {
    let root = install::skills_dir().map_err(fail)?;
    let entries: Vec<Value> = install::pending_in(&root)
        .into_iter()
        .map(|(token, manifest)| json!({ "token": token, "manifest": manifest }))
        .collect();
    Ok(Value::Array(entries))
}

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

#[tauri::command]
pub async fn llm_skills_reproject() -> Result<Value, String> {
    let projection = omniget_core::core::assist::bots::skills::projection();
    serde_json::to_value(projection.reproject()?).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_skills_accept(name: String) -> Result<Value, String> {
    let projection = omniget_core::core::assist::bots::skills::projection();
    let summary = omniget_core::core::assist::bots::skills::accept_current(&projection, &name)?;
    serde_json::to_value(summary).map_err(|e| e.to_string())
}

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
                "license": catalog::LICENSE,
            })
        })
        .collect();
    Ok(Value::Array(entries))
}
