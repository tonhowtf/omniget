use serde_json::Value;

use super::{query_env, tail, App, PackageBackend, RemovePreview, Update};

pub struct Brew;

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
