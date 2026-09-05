//! Inicialização (estudo 10, Kudu; 25, Sophia): o que abre junto com o
//! sistema, com liga/desliga. macOS: LaunchAgents do usuário e Itens de
//! Login; Windows: chaves Run/RunOnce, pasta Inicializar e o estado em
//! StartupApproved; Linux: `~/.config/autostart` (com override dos itens de
//! `/etc/xdg/autostart`) e serviços `systemd --user`.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupItem {
    pub id: String,
    pub name: String,
    pub command: String,
    /// "launch-agent" | "login-item" | "run" | "run-once" | "startup-folder" | "autostart" | "systemd-user"
    pub source: String,
    /// "user" | "system"
    pub scope: String,
    pub path: String,
    pub enabled: bool,
    pub can_toggle: bool,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

async fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let o = crate::core::process::command(program)
        .args(args)
        .output()
        .await?;
    if !o.status.success() {
        return Err(anyhow!(
            "{} falhou: {}",
            program,
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

// ── macOS ──────────────────────────────────────────────────────────────

async fn plist_json(path: &Path) -> Option<serde_json::Value> {
    let o = crate::core::process::command("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .await
        .ok()?;
    if !o.status.success() {
        return None;
    }
    serde_json::from_slice(&o.stdout).ok()
}

async fn mac_items() -> Vec<StartupItem> {
    let mut out = Vec::new();
    let uid = run("id", &["-u"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "501".into());
    let disabled = run("launchctl", &["print-disabled", &format!("gui/{}", uid)])
        .await
        .unwrap_or_default();
    for (dir, scope) in [
        (home().join("Library/LaunchAgents"), "user"),
        (PathBuf::from("/Library/LaunchAgents"), "system"),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "plist").unwrap_or(false))
            .collect();
        entries.sort();
        for p in entries {
            let json = plist_json(&p).await.unwrap_or(serde_json::Value::Null);
            let label = json
                .get("Label")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    p.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            let command = json
                .get("ProgramArguments")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .or_else(|| {
                    json.get("Program")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            let disabled_key = json
                .get("Disabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let disabled_override = disabled.lines().any(|l| {
                l.contains(&format!("\"{}\" => disabled", label))
                    || l.contains(&format!("\"{}\" => true", label))
            });
            out.push(StartupItem {
                id: format!("la:{}", p.display()),
                name: label,
                command,
                source: "launch-agent".into(),
                scope: scope.into(),
                path: p.to_string_lossy().to_string(),
                enabled: !(disabled_key || disabled_override),
                can_toggle: scope == "user",
            });
        }
    }
    // Itens de Login (System Settings → Geral → Itens de Login)
    if let Ok(list) = run(
        "osascript",
        &[
            "-e",
            "tell application \"System Events\" to get {name, path, hidden} of every login item",
        ],
    )
    .await
    {
        // Saída: {{n1, n2}, {p1, p2}, {false, true}} → parse simples
        let parts: Vec<Vec<String>> = list
            .trim()
            .trim_start_matches('{')
            .trim_end_matches('}')
            .split("}, {")
            .map(|g| {
                g.trim_matches(|c| c == '{' || c == '}')
                    .split(", ")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .collect();
        if parts.len() >= 2 {
            for (i, name) in parts[0].iter().enumerate() {
                let path = parts[1].get(i).cloned().unwrap_or_default();
                out.push(StartupItem {
                    id: format!("li:{}", name),
                    name: name.clone(),
                    command: path.clone(),
                    source: "login-item".into(),
                    scope: "user".into(),
                    path,
                    enabled: true,
                    can_toggle: true,
                });
            }
        }
    }
    // Itens de login desligados por nós ficam guardados para religar.
    for (name, path) in removed_login_items() {
        if !out
            .iter()
            .any(|i| i.source == "login-item" && i.name == name)
        {
            out.push(StartupItem {
                id: format!("li:{}", name),
                name,
                command: path.clone(),
                source: "login-item".into(),
                scope: "user".into(),
                path,
                enabled: false,
                can_toggle: true,
            });
        }
    }
    out
}

fn removed_file() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("startup-removed.json"))
}

fn removed_login_items() -> Vec<(String, String)> {
    removed_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_removed(list: &[(String, String)]) {
    if let Some(p) = removed_file() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = std::fs::write(p, serde_json::to_string_pretty(list).unwrap_or_default());
    }
}

async fn mac_set(item: &StartupItem, enabled: bool) -> anyhow::Result<()> {
    match item.source.as_str() {
        "launch-agent" => {
            let uid = run("id", &["-u"])
                .await
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| "501".into());
            let target = format!("gui/{}", uid);
            if enabled {
                let _ = run(
                    "launchctl",
                    &["enable", &format!("{}/{}", target, item.name)],
                )
                .await;
                run("launchctl", &["bootstrap", &target, &item.path])
                    .await
                    .or_else(|_| Ok::<_, anyhow::Error>(String::new()))?;
            } else {
                let _ = run(
                    "launchctl",
                    &["bootout", &format!("{}/{}", target, item.name)],
                )
                .await;
                run(
                    "launchctl",
                    &["disable", &format!("{}/{}", target, item.name)],
                )
                .await?;
            }
            Ok(())
        }
        "login-item" => {
            let mut removed = removed_login_items();
            if enabled {
                let path = removed
                    .iter()
                    .find(|(n, _)| *n == item.name)
                    .map(|(_, p)| p.clone())
                    .unwrap_or_else(|| item.path.clone());
                if path.is_empty() {
                    return Err(anyhow!("caminho do item de login desconhecido"));
                }
                run("osascript", &["-e", &format!("tell application \"System Events\" to make login item at end with properties {{path:\"{}\", hidden:false}}", path.replace('"', "\\\""))]).await?;
                removed.retain(|(n, _)| *n != item.name);
            } else {
                run(
                    "osascript",
                    &[
                        "-e",
                        &format!(
                            "tell application \"System Events\" to delete login item \"{}\"",
                            item.name.replace('"', "\\\"")
                        ),
                    ],
                )
                .await?;
                removed.retain(|(n, _)| *n != item.name);
                removed.push((item.name.clone(), item.path.clone()));
            }
            save_removed(&removed);
            Ok(())
        }
        _ => Err(anyhow!("nao da para alterar este item")),
    }
}

// ── Windows ────────────────────────────────────────────────────────────

const WIN_RUN: &[(&str, &str, &str)] = &[
    (
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        "run",
        "user",
    ),
    (
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\RunOnce",
        "run-once",
        "user",
    ),
    (
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
        "run",
        "system",
    ),
    (
        r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
        "run",
        "system",
    ),
    (
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce",
        "run-once",
        "system",
    ),
];

fn approved_key(scope: &str, folder: bool) -> String {
    let root = if scope == "user" { "HKCU" } else { "HKLM" };
    format!(
        r"{}\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\{}",
        root,
        if folder { "StartupFolder" } else { "Run" }
    )
}

async fn win_approved(scope: &str, folder: bool) -> std::collections::HashMap<String, bool> {
    let mut map = std::collections::HashMap::new();
    for key in super::win_registry::reg_query(&approved_key(scope, folder), false).await {
        for v in key.values {
            // REG_BINARY: primeiro byte 02/06 = ligado, 03/07 = desligado
            let enabled = v
                .data
                .get(0..2)
                .map(|b| b == "02" || b == "06")
                .unwrap_or(true);
            map.insert(v.name.to_ascii_lowercase(), enabled);
        }
    }
    map
}

async fn win_items() -> Vec<StartupItem> {
    let mut out = Vec::new();
    for (key, source, scope) in WIN_RUN {
        let approved = win_approved(scope, false).await;
        for k in super::win_registry::reg_query(key, false).await {
            for v in k.values {
                if v.name.is_empty() {
                    continue;
                }
                let enabled = *approved.get(&v.name.to_ascii_lowercase()).unwrap_or(&true);
                out.push(StartupItem {
                    id: format!("reg:{}|{}", key, v.name),
                    name: v.name.clone(),
                    command: super::win_registry::expand_env(&v.data),
                    source: source.to_string(),
                    scope: scope.to_string(),
                    path: key.to_string(),
                    enabled,
                    can_toggle: *source == "run",
                });
            }
        }
    }
    for (dir, scope) in [
        (
            std::env::var("APPDATA")
                .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\Startup")),
            "user",
        ),
        (
            std::env::var("ProgramData")
                .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\StartUp")),
            "system",
        ),
    ] {
        let Ok(dir) = dir else { continue };
        let approved = win_approved(scope, true).await;
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            let enabled = *approved.get(&name.to_ascii_lowercase()).unwrap_or(&true);
            out.push(StartupItem {
                id: format!("folder:{}", e.path().display()),
                name: name.trim_end_matches(".lnk").to_string(),
                command: e.path().to_string_lossy().to_string(),
                source: "startup-folder".into(),
                scope: scope.into(),
                path: e.path().to_string_lossy().to_string(),
                enabled,
                can_toggle: scope == "user",
            });
        }
    }
    out
}

async fn win_set(item: &StartupItem, enabled: bool) -> anyhow::Result<()> {
    let folder = item.source == "startup-folder";
    let key = approved_key(&item.scope, folder);
    let name = if folder {
        Path::new(&item.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        item.name.clone()
    };
    let data = if enabled {
        "020000000000000000000000"
    } else {
        "030000000000000000000000"
    };
    super::win_registry::reg_add(&key, &name, "REG_BINARY", data).await
}

// ── Linux ──────────────────────────────────────────────────────────────

fn desktop_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        l.strip_prefix(&format!("{}=", key))
            .map(|v| v.trim().to_string())
    })
}

async fn linux_items() -> Vec<StartupItem> {
    let mut out = Vec::new();
    let user_dir = dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join("autostart");
    let mut seen = std::collections::HashSet::new();
    for (dir, scope) in [
        (user_dir.clone(), "user"),
        (PathBuf::from("/etc/xdg/autostart"), "system"),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "desktop").unwrap_or(false))
            .collect();
        entries.sort();
        for p in entries {
            let file = p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if !seen.insert(file.clone()) {
                continue; // o do usuário sobrepõe o do sistema
            }
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let hidden = desktop_field(&text, "Hidden")
                .map(|v| v == "true")
                .unwrap_or(false);
            let gnome_off = desktop_field(&text, "X-GNOME-Autostart-enabled")
                .map(|v| v == "false")
                .unwrap_or(false);
            out.push(StartupItem {
                id: format!("autostart:{}", file),
                name: desktop_field(&text, "Name")
                    .unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
                command: desktop_field(&text, "Exec").unwrap_or_default(),
                source: "autostart".into(),
                scope: scope.into(),
                path: p.to_string_lossy().to_string(),
                enabled: !(hidden || gnome_off),
                can_toggle: true,
            });
        }
    }
    if let Ok(list) = run(
        "systemctl",
        &[
            "--user",
            "list-unit-files",
            "--type=service",
            "--no-legend",
            "--no-pager",
        ],
    )
    .await
    {
        for l in list.lines() {
            let mut cols = l.split_whitespace();
            let (Some(unit), Some(state)) = (cols.next(), cols.next()) else {
                continue;
            };
            if !matches!(state, "enabled" | "disabled") {
                continue;
            }
            out.push(StartupItem {
                id: format!("systemd:{}", unit),
                name: unit.trim_end_matches(".service").to_string(),
                command: unit.to_string(),
                source: "systemd-user".into(),
                scope: "user".into(),
                path: unit.to_string(),
                enabled: state == "enabled",
                can_toggle: true,
            });
        }
    }
    out
}

async fn linux_set(item: &StartupItem, enabled: bool) -> anyhow::Result<()> {
    match item.source.as_str() {
        "autostart" => {
            let user_dir = dirs::config_dir()
                .unwrap_or_else(|| home().join(".config"))
                .join("autostart");
            std::fs::create_dir_all(&user_dir)?;
            let file = Path::new(&item.path)
                .file_name()
                .ok_or_else(|| anyhow!("arquivo invalido"))?;
            let target = user_dir.join(file);
            let mut text = std::fs::read_to_string(&item.path).unwrap_or_default();
            let mut lines: Vec<String> = text
                .lines()
                .filter(|l| {
                    !l.starts_with("Hidden=") && !l.starts_with("X-GNOME-Autostart-enabled=")
                })
                .map(String::from)
                .collect();
            if !enabled {
                lines.push("Hidden=true".into());
                lines.push("X-GNOME-Autostart-enabled=false".into());
            }
            text = lines.join("\n") + "\n";
            std::fs::write(&target, text)?;
            Ok(())
        }
        "systemd-user" => {
            run(
                "systemctl",
                &[
                    "--user",
                    if enabled { "enable" } else { "disable" },
                    &item.path,
                ],
            )
            .await?;
            Ok(())
        }
        _ => Err(anyhow!("nao da para alterar este item")),
    }
}

// ── API ────────────────────────────────────────────────────────────────

pub async fn list() -> Vec<StartupItem> {
    let mut items = if cfg!(target_os = "macos") {
        mac_items().await
    } else if cfg!(target_os = "windows") {
        win_items().await
    } else {
        linux_items().await
    };
    items.sort_by_key(|a| a.name.to_lowercase());
    items
}

pub async fn set_enabled(item: &StartupItem, enabled: bool) -> anyhow::Result<()> {
    if !item.can_toggle {
        return Err(anyhow!(
            "este item so pode ser alterado com privilegios de administrador"
        ));
    }
    if cfg!(target_os = "macos") {
        mac_set(item, enabled).await
    } else if cfg!(target_os = "windows") {
        win_set(item, enabled).await
    } else {
        linux_set(item, enabled).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop() {
        let t = "[Desktop Entry]\nName=Foo\nExec=/usr/bin/foo --x\nHidden=true\n";
        assert_eq!(desktop_field(t, "Name").as_deref(), Some("Foo"));
        assert_eq!(
            desktop_field(t, "Exec").as_deref(),
            Some("/usr/bin/foo --x")
        );
        assert_eq!(desktop_field(t, "Hidden").as_deref(), Some("true"));
        assert!(desktop_field(t, "Nope").is_none());
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p omniget-core --lib tools::startup::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn list_here() {
        let items = super::list().await;
        for i in &items {
            println!(
                "{:<14} {:<6} {:<5} {} :: {}",
                i.source,
                i.scope,
                i.enabled,
                i.name,
                i.command.chars().take(80).collect::<String>()
            );
        }
        println!("{} itens", items.len());
    }
}
