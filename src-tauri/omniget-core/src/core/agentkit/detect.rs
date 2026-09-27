//! Which coding tools are on this machine: binaries on PATH, config folders in
//! the home, VS Code extensions, app bundles, and the version from `--version`
//! (short timeout). Only presence is checked: no config or credential file is
//! ever opened here.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::targets::{self, TargetAdapter, TargetStatus};
use super::{Env, Os, Scope};

const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundBinary {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeInfo {
    pub scope: Scope,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Config folder for this scope (global dir, or project root), for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedTarget {
    pub id: String,
    pub name: String,
    pub tier: u8,
    pub status: TargetStatus,
    pub beta: bool,
    pub installed: bool,
    /// Suggested default for the "install into" toggles: installed and active.
    pub enabled_by_default: bool,
    pub binaries: Vec<FoundBinary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub home_dirs: Vec<String>,
    pub vscode_extensions: Vec<String>,
    pub apps: Vec<String>,
    /// Project markers of this tool found in the project folder.
    pub project_markers: Vec<String>,
    pub scopes: Vec<ScopeInfo>,
}

/// PATH entries to search, plus the usual user install folders on the real machine.
fn search_dirs(env: &Env) -> Vec<PathBuf> {
    let Some(pv) = &env.path_var else {
        return vec![];
    };
    let mut dirs: Vec<PathBuf> = std::env::split_paths(pv).collect();
    for extra in [
        env.home.join(".local").join("bin"),
        env.home.join(".npm-global").join("bin"),
        env.home.join(".bun").join("bin"),
        env.home.join(".cargo").join("bin"),
        env.home.join(".volta").join("bin"),
        env.home.join(".opencode").join("bin"),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    if env.os == Os::Macos {
        for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let p = PathBuf::from(d);
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }
    dirs
}

fn find_binary(dirs: &[PathBuf], name: &str, os: Os) -> Option<PathBuf> {
    let exts: &[&str] = if os == Os::Windows {
        &["", ".exe", ".cmd", ".bat", ".ps1"]
    } else {
        &[""]
    };
    for d in dirs {
        for ext in exts {
            let p = d.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// First version-looking token of `<bin> <args>`, killed after a short timeout.
pub fn probe_version(bin: &Path, args: &[String]) -> Option<String> {
    use std::io::Read;
    let mut cmd = crate::core::process::std_command(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = crate::core::process::spawn_retrying_busy(|| cmd.spawn()).ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < VERSION_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(40))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if out.trim().is_empty() {
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut out);
        }
    }
    extract_version(&out)
}

fn extract_version(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"\d+\.\d+(?:\.\d+)?(?:[-+.][0-9A-Za-z.-]+)?").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

fn extension_dirs(env: &Env) -> Vec<PathBuf> {
    [
        ".vscode",
        ".vscode-insiders",
        ".vscode-server",
        ".cursor",
        ".windsurf",
        ".vscode-oss",
        ".trae",
        ".kiro",
    ]
    .iter()
    .map(|d| env.home.join(d).join("extensions"))
    .filter(|p| p.is_dir())
    .collect()
}

fn installed_extensions(env: &Env) -> Vec<String> {
    let mut out = Vec::new();
    for d in extension_dirs(env) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            out.push(e.file_name().to_string_lossy().to_ascii_lowercase());
        }
    }
    out
}

/// Detects one tool.
pub fn detect(env: &Env, t: &TargetAdapter, project: Option<&Path>) -> DetectedTarget {
    let dirs = search_dirs(env);
    detect_with(env, t, project, &dirs, &installed_extensions(env))
}

fn detect_with(
    env: &Env,
    t: &TargetAdapter,
    project: Option<&Path>,
    dirs: &[PathBuf],
    exts: &[String],
) -> DetectedTarget {
    let binaries: Vec<FoundBinary> = t
        .detect
        .binaries
        .iter()
        .filter_map(|b| {
            find_binary(dirs, b, env.os).map(|p| FoundBinary {
                name: b.clone(),
                path: p.display().to_string(),
            })
        })
        .collect();
    let home_dirs: Vec<String> = t
        .detect
        .home_dirs
        .iter()
        .filter_map(|h| env.expand(h, None))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .collect();
    let vscode_extensions: Vec<String> = t
        .detect
        .vscode_extensions
        .iter()
        .filter(|id| {
            let prefix = format!("{}-", id.to_ascii_lowercase());
            exts.iter().any(|e| e.starts_with(&prefix))
        })
        .cloned()
        .collect();
    let mut apps: Vec<String> = Vec::new();
    match env.os {
        Os::Macos => {
            for b in &t.detect.app_bundles {
                for d in &env.applications_dirs {
                    let p = d.join(b);
                    if p.exists() {
                        apps.push(p.display().to_string());
                    }
                }
            }
        }
        Os::Windows => {
            for a in &t.detect.app_paths_windows {
                if let Some(p) = env.expand(a, None).filter(|p| p.exists()) {
                    apps.push(p.display().to_string());
                }
            }
        }
        Os::Linux => {
            for a in &t.detect.app_paths_linux {
                if let Some(p) = env.expand(a, None).filter(|p| p.exists()) {
                    apps.push(p.display().to_string());
                }
            }
        }
    }
    let installed = !binaries.is_empty()
        || !vscode_extensions.is_empty()
        || !apps.is_empty()
        || !home_dirs.is_empty();
    let version = if env.probe_versions {
        binaries.first().and_then(|b| {
            let args = if t.detect.version_args.is_empty() {
                vec!["--version".to_string()]
            } else {
                t.detect.version_args.clone()
            };
            probe_version(Path::new(&b.path), &args)
        })
    } else {
        None
    };
    let project_markers: Vec<String> = project
        .map(|p| {
            t.scopes
                .project_markers
                .iter()
                .filter(|m| p.join(m).exists())
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let scopes = [Scope::Project, Scope::Global, Scope::Local, Scope::Managed]
        .into_iter()
        .map(|s| {
            let (available, reason, root) = if !t.supports_scope(s) {
                (
                    false,
                    Some(format!("{} has no {} scope", t.name, s.as_str())),
                    None,
                )
            } else if s.is_project_bound() && project.is_none() {
                (false, Some("pick a project folder".to_string()), None)
            } else if s.is_project_bound() {
                (true, None, project.map(|p| p.display().to_string()))
            } else if s == Scope::Managed {
                (true, Some("needs administrator rights".to_string()), None)
            } else {
                (
                    true,
                    None,
                    t.scopes
                        .global_dir
                        .as_deref()
                        .and_then(|g| env.expand(g, None))
                        .map(|p| p.display().to_string()),
                )
            };
            ScopeInfo {
                scope: s,
                available,
                reason,
                root,
            }
        })
        .collect();
    DetectedTarget {
        id: t.id.clone(),
        name: t.name.clone(),
        tier: t.tier,
        status: t.status,
        beta: t.beta,
        installed,
        enabled_by_default: installed && t.status != TargetStatus::Deprecated,
        binaries,
        version,
        home_dirs,
        vscode_extensions,
        apps,
        project_markers,
        scopes,
    }
}

/// Detects every known tool (version probes run in parallel).
pub fn detect_all(env: &Env, project: Option<&Path>) -> Vec<DetectedTarget> {
    let all = targets::load_targets(env);
    let dirs = search_dirs(env);
    let exts = installed_extensions(env);
    std::thread::scope(|s| {
        let handles: Vec<_> = all
            .iter()
            .map(|t| s.spawn(|| detect_with(env, t, project, &dirs, &exts)))
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_token() {
        assert_eq!(
            extract_version("2.1.261 (Claude Code)").as_deref(),
            Some("2.1.261")
        );
        assert_eq!(
            extract_version("codex-cli 0.44.0-alpha.3\n").as_deref(),
            Some("0.44.0-alpha.3")
        );
        assert_eq!(extract_version("no version"), None);
    }
}
