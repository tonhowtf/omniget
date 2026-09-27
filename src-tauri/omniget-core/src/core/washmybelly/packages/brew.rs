//! Homebrew (macOS).
//!
//! - updates: `brew outdated --json=v2` (`formulae[]` and `casks[]` with
//!   `name`, `installed_versions[]`, `current_version`, `pinned`); pinned
//!   formulae are listed with a note and never planned;
//! - upgrade: `brew upgrade --formula|--cask -- <name>`, one process per
//!   package, no administrator (a cask whose installer asks for a password
//!   fails and says so);
//! - the installed inventory and `brew uninstall --zap` live in `apps/macos.rs`.

use serde_json::Value;

use super::{query_env, tail, App, PackageBackend, RemovePreview, Update};

pub struct Brew;

/// Where Homebrew lives (Apple silicon, Intel).
pub fn path() -> Option<std::path::PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}

fn program() -> String {
    path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "brew".into())
}

/// `brew outdated --json=v2`.
pub fn parse_outdated(json: &str) -> Vec<Update> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, scope) in [("formulae", "formula"), ("casks", "cask")] {
        for f in v[key].as_array().into_iter().flatten() {
            let Some(name) = f["name"].as_str() else {
                continue;
            };
            if !super::valid_name("brew", name) {
                continue;
            }
            let current = f["installed_versions"]
                .as_array()
                .and_then(|a| a.last())
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let pinned = f["pinned"].as_bool().unwrap_or(false);
            out.push(Update {
                id: format!("packages:brew:{scope}:{name}"),
                name: name.to_string(),
                manager: "brew".into(),
                current,
                available: f["current_version"].as_str().unwrap_or("").to_string(),
                size: None,
                admin: false,
                scope: Some(scope.to_string()),
                note: pinned.then(|| format!("{name} is pinned (brew unpin {name} to update it)")),
            });
        }
    }
    out
}

/// argv (after `brew`) to upgrade one formula or cask.
pub fn upgrade_args(name: &str, scope: &str) -> Vec<String> {
    let kind = if scope == "cask" {
        "--cask"
    } else {
        "--formula"
    };
    ["upgrade", kind, "--", name].map(String::from).to_vec()
}

pub fn upgrade_one(name: &str, scope: &str) -> Result<String, String> {
    if !super::valid_name("brew", name) || !matches!(scope, "formula" | "cask") {
        return Err("BAD_NAME".into());
    }
    let args = upgrade_args(name, scope);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = query_env(&program(), &args, &[("HOMEBREW_NO_ENV_HINTS", "1")])?;
    if r.code == Some(0) {
        Ok("updated".into())
    } else {
        Err(format!(
            "brew: {}",
            tail(&format!("{}\n{}", r.stdout, r.stderr), 4)
        ))
    }
}

impl PackageBackend for Brew {
    fn id(&self) -> &'static str {
        "brew"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "macos") && path().is_some()
    }

    fn list(&self) -> Result<Vec<App>, String> {
        Ok(Vec::new())
    }

    fn remove_preview(&self, names: &[String]) -> Result<RemovePreview, String> {
        Ok(RemovePreview {
            removes: names.to_vec(),
            also_removes: Vec::new(),
            frees_bytes: None,
            exact: false,
        })
    }

    fn remove_argv(&self, names: &[String]) -> Vec<String> {
        let mut v: Vec<String> = ["brew", "uninstall", "--"].map(String::from).to_vec();
        v.extend(names.iter().cloned());
        v
    }

    fn orphans(&self) -> Result<Vec<App>, String> {
        Ok(Vec::new())
    }

    fn updates(&self) -> Result<Vec<Update>, String> {
        // No auto-update here: listing must stay a read.
        let r = query_env(
            &program(),
            &["outdated", "--json=v2"],
            &[
                ("HOMEBREW_NO_AUTO_UPDATE", "1"),
                ("HOMEBREW_NO_ENV_HINTS", "1"),
            ],
        )?;
        if r.code != Some(0) && r.stdout.trim().is_empty() {
            return Err(format!("brew outdated: {}", tail(&r.stderr, 3)));
        }
        Ok(parse_outdated(&r.stdout))
    }

    fn upgrade_argv(&self, names: &[String]) -> Vec<String> {
        let mut v = vec!["brew".to_string()];
        v.extend(upgrade_args(
            names.first().map(String::as_str).unwrap_or(""),
            "formula",
        ));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture;
    use super::*;

    #[test]
    fn outdated_json() {
        let v = parse_outdated(&fixture("brew_outdated.json"));
        assert_eq!(v.len(), 4);
        let aom = &v[0];
        assert_eq!(
            (
                aom.name.as_str(),
                aom.current.as_str(),
                aom.available.as_str()
            ),
            ("aom", "3.15.0", "3.15.1")
        );
        assert_eq!(aom.scope.as_deref(), Some("formula"));
        assert_eq!(aom.id, "packages:brew:formula:aom");
        let cask = v.last().unwrap();
        assert_eq!(cask.scope.as_deref(), Some("cask"));
        assert_eq!(cask.id, "packages:brew:cask:azure-cli-preview");
        assert!(v.iter().all(|u| !u.admin && u.note.is_none()));
        let pinned = parse_outdated(
            r#"{"formulae":[{"name":"node","installed_versions":["20"],"current_version":"22","pinned":true}],"casks":[]}"#,
        );
        assert!(pinned[0].note.is_some());
    }

    #[test]
    fn upgrade_argv_by_kind() {
        assert_eq!(
            upgrade_args("firefox", "cask"),
            vec!["upgrade", "--cask", "--", "firefox"]
        );
        assert_eq!(
            upgrade_args("aom", "formula"),
            vec!["upgrade", "--formula", "--", "aom"]
        );
        assert!(upgrade_one("aom", "bottle").is_err());
    }
}
