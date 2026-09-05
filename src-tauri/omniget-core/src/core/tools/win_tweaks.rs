//! ⚠️ SOMENTE WINDOWS. Ajustes de privacidade e endurecimento derivados do
//! Sophia Script (MIT, estudo 25) e do hardentools (GPL-3, estudo 31), como
//! regras declarativas de registro. Fora do Windows a lista existe (para a UI
//! explicar o que faria) mas nada é aplicado. Cada regra guarda o valor
//! padrão do Windows para reverter.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RegValue {
    Dword(u32),
    Str(String),
    /// Apagar o valor (o Windows volta ao padrão).
    Delete,
}

#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub id: String,
    /// "privacy" | "harden" | "ui" | "context"
    pub group: String,
    pub source: String,
    pub key: String,
    pub name: String,
    pub on: RegValue,
    pub off: RegValue,
    pub requires_admin: bool,
    pub restart: bool,
    /// `None` fora do Windows ou se não deu para ler.
    pub applied: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
fn r(
    id: &str,
    group: &str,
    source: &str,
    key: &str,
    name: &str,
    on: RegValue,
    off: RegValue,
    admin: bool,
    restart: bool,
) -> Rule {
    Rule {
        id: id.into(),
        group: group.into(),
        source: source.into(),
        key: key.into(),
        name: name.into(),
        on,
        off,
        requires_admin: admin,
        restart,
        applied: None,
    }
}

pub fn rules() -> Vec<Rule> {
    use RegValue::*;
    vec![
        // ── Privacidade (Sophia: Privacy & Telemetry) ──
        r(
            "advertising-id",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "tailored-experiences",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Privacy",
            "TailoredExperiencesWithDiagnosticDataEnabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "settings-suggested-content",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-338393Enabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "apps-silent-install",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SilentInstalledAppsEnabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "welcome-experience",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-310093Enabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "start-suggestions",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "SubscribedContent-338388Enabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "lockscreen-tips",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
            "RotatingLockScreenOverlayEnabled",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "bing-search",
            "privacy",
            "sophia",
            r"HKCU\Software\Policies\Microsoft\Windows\Explorer",
            "DisableSearchBoxSuggestions",
            Dword(1),
            Delete,
            false,
            true,
        ),
        r(
            "diagnostic-data-minimal",
            "privacy",
            "sophia",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
            "AllowTelemetry",
            Dword(1),
            Delete,
            true,
            true,
        ),
        r(
            "feedback-never",
            "privacy",
            "sophia",
            r"HKCU\Software\Microsoft\Siuf\Rules",
            "NumberOfSIUFInPeriod",
            Dword(0),
            Delete,
            false,
            false,
        ),
        r(
            "activity-history",
            "privacy",
            "sophia",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
            "PublishUserActivities",
            Dword(0),
            Delete,
            true,
            false,
        ),
        r(
            "location-tracking",
            "privacy",
            "sophia",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
            "Value",
            Str("Deny".into()),
            Str("Allow".into()),
            true,
            false,
        ),
        r(
            "copilot-off",
            "privacy",
            "sophia",
            r"HKCU\Software\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            Dword(1),
            Delete,
            false,
            true,
        ),
        r(
            "recall-off",
            "privacy",
            "hardentools",
            r"HKCU\Software\Policies\Microsoft\Windows\WindowsAI",
            "DisableAIDataAnalysis",
            Dword(1),
            Delete,
            false,
            true,
        ),
        // ── Interface (Sophia: UI & Personalization) ──
        r(
            "show-file-extensions",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "HideFileExt",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "show-hidden-files",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Hidden",
            Dword(1),
            Dword(2),
            false,
            false,
        ),
        r(
            "explorer-this-pc",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "LaunchTo",
            Dword(1),
            Dword(2),
            false,
            false,
        ),
        r(
            "taskbar-left",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarAl",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "taskbar-no-search",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Search",
            "SearchboxTaskbarMode",
            Dword(0),
            Dword(2),
            false,
            false,
        ),
        r(
            "taskbar-no-widgets",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarDa",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "taskbar-end-task",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            Dword(1),
            Dword(0),
            false,
            false,
        ),
        r(
            "classic-context-menu",
            "context",
            "sophia",
            r"HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32",
            "",
            Str(String::new()),
            Delete,
            false,
            true,
        ),
        r(
            "onedrive-ad-explorer",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "ShowSyncProviderNotifications",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "seconds-in-clock",
            "ui",
            "sophia",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "ShowSecondsInSystemClock",
            Dword(1),
            Dword(0),
            false,
            false,
        ),
        // ── Endurecimento (hardentools) ──
        r(
            "autorun-off",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer",
            "NoDriveTypeAutoRun",
            Dword(0xFF),
            Delete,
            false,
            false,
        ),
        r(
            "autoplay-off",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorers\AutoplayHandlers",
            "DisableAutoplay",
            Dword(1),
            Dword(0),
            false,
            false,
        ),
        r(
            "wsh-off",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Windows Script Host\Settings",
            "Enabled",
            Dword(0),
            Delete,
            false,
            false,
        ),
        r(
            "office-macros-word",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Office\16.0\Word\Security",
            "VBAWarnings",
            Dword(4),
            Dword(2),
            false,
            false,
        ),
        r(
            "office-macros-excel",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Office\16.0\Excel\Security",
            "VBAWarnings",
            Dword(4),
            Dword(2),
            false,
            false,
        ),
        r(
            "office-macros-powerpoint",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Office\16.0\PowerPoint\Security",
            "VBAWarnings",
            Dword(4),
            Dword(2),
            false,
            false,
        ),
        r(
            "office-dde-word",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Office\16.0\Word\Options",
            "DontUpdateLinks",
            Dword(1),
            Delete,
            false,
            false,
        ),
        r(
            "office-dde-excel",
            "harden",
            "hardentools",
            r"HKCU\Software\Microsoft\Office\16.0\Excel\Options",
            "DontUpdateLinks",
            Dword(1),
            Delete,
            false,
            false,
        ),
        r(
            "acrobat-js-off",
            "harden",
            "hardentools",
            r"HKCU\Software\Adobe\Acrobat Reader\DC\JSPrefs",
            "bEnableJS",
            Dword(0),
            Dword(1),
            false,
            false,
        ),
        r(
            "defender-pua",
            "harden",
            "hardentools",
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender",
            "PUAProtection",
            Dword(1),
            Delete,
            true,
            false,
        ),
        r(
            "uac-max",
            "harden",
            "hardentools",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System",
            "ConsentPromptBehaviorAdmin",
            Dword(2),
            Dword(5),
            true,
            false,
        ),
        r(
            "lsa-protection",
            "harden",
            "hardentools",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Lsa",
            "RunAsPPL",
            Dword(1),
            Delete,
            true,
            true,
        ),
        r(
            "safe-dll-loading",
            "harden",
            "hardentools",
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager",
            "SafeDllSearchMode",
            Dword(1),
            Delete,
            true,
            true,
        ),
    ]
}

#[derive(Debug, Clone, Serialize)]
pub struct TweaksStatus {
    pub supported: bool,
    pub is_admin: bool,
    pub rules: Vec<Rule>,
}

#[cfg(target_os = "windows")]
async fn reg_query(key: &str, name: &str) -> Option<RegValue> {
    let mut cmd = crate::core::process::command("reg");
    cmd.args(["query", key]);
    if name.is_empty() {
        cmd.arg("/ve");
    } else {
        cmd.args(["/v", name]);
    }
    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return Some(RegValue::Delete);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && (parts[1].starts_with("REG_")) {
            let ty = parts[1];
            let val = parts[2..].join(" ");
            return Some(match ty {
                "REG_DWORD" => RegValue::Dword(
                    u32::from_str_radix(val.trim_start_matches("0x"), 16).unwrap_or(0),
                ),
                _ => RegValue::Str(val),
            });
        }
    }
    Some(RegValue::Delete)
}

#[cfg(target_os = "windows")]
async fn reg_set(key: &str, name: &str, value: &RegValue) -> anyhow::Result<()> {
    let mut cmd = crate::core::process::command("reg");
    match value {
        RegValue::Delete => {
            cmd.args(["delete", key]);
            if name.is_empty() {
                cmd.arg("/ve");
            } else {
                cmd.args(["/v", name]);
            }
            cmd.arg("/f");
        }
        RegValue::Dword(d) => {
            cmd.args(["add", key]);
            if name.is_empty() {
                cmd.arg("/ve");
            } else {
                cmd.args(["/v", name]);
            }
            cmd.args(["/t", "REG_DWORD", "/d", &d.to_string(), "/f"]);
        }
        RegValue::Str(s) => {
            cmd.args(["add", key]);
            if name.is_empty() {
                cmd.arg("/ve");
            } else {
                cmd.args(["/v", name]);
            }
            cmd.args(["/t", "REG_SZ", "/d", s, "/f"]);
        }
    }
    let out = cmd.output().await?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // apagar o que não existe não é erro
        if matches!(value, RegValue::Delete)
            && (msg.contains("unable to find")
                || msg.contains("não foi possível")
                || msg.is_empty())
        {
            return Ok(());
        }
        return Err(anyhow!("reg falhou: {}", msg));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
async fn is_admin() -> bool {
    crate::core::process::command("net")
        .args(["session"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[allow(unused_mut)]
pub async fn status() -> TweaksStatus {
    let mut rules = rules();
    #[cfg(target_os = "windows")]
    {
        for rule in rules.iter_mut() {
            rule.applied = reg_query(&rule.key, &rule.name).await.map(|v| v == rule.on);
        }
        TweaksStatus {
            supported: true,
            is_admin: is_admin().await,
            rules,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        TweaksStatus {
            supported: false,
            is_admin: false,
            rules,
        }
    }
}

/// Aplica (`enable`) ou reverte uma regra. Guarda o valor anterior em
/// `<app_data>/tools/win_tweaks_backup.json` para o botão "restaurar".
#[allow(unused_mut, unused_variables)]
pub async fn apply(id: &str, enable: bool) -> anyhow::Result<Rule> {
    let mut rule = rules()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow!("regra desconhecida: {}", id))?;
    #[cfg(not(target_os = "windows"))]
    {
        let _ = enable;
        let _ = &mut rule;
        Err(anyhow!("esses ajustes so existem no Windows"))
    }
    #[cfg(target_os = "windows")]
    {
        if rule.requires_admin && !is_admin().await {
            return Err(anyhow!(
                "essa regra precisa do OmniGet aberto como administrador"
            ));
        }
        if let Some(prev) = reg_query(&rule.key, &rule.name).await {
            backup_store(&rule.id, &prev);
        }
        let target = if enable {
            rule.on.clone()
        } else {
            restore_value(&rule)
        };
        reg_set(&rule.key, &rule.name, &target).await?;
        rule.applied = Some(enable);
        Ok(rule)
    }
}

#[cfg(target_os = "windows")]
fn backup_path() -> Option<std::path::PathBuf> {
    super::tools_dir().map(|d| d.join("win_tweaks_backup.json"))
}

#[cfg(target_os = "windows")]
fn backup_store(id: &str, value: &RegValue) {
    let Some(p) = backup_path() else { return };
    let mut map: std::collections::BTreeMap<String, RegValue> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    map.entry(id.to_string()).or_insert_with(|| value.clone());
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&map).unwrap_or_default());
}

#[cfg(target_os = "windows")]
fn restore_value(rule: &Rule) -> RegValue {
    let stored: Option<RegValue> = backup_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<std::collections::BTreeMap<String, RegValue>>(&t).ok())
        .and_then(|m| m.get(&rule.id).cloned());
    stored.unwrap_or_else(|| rule.off.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn rules_have_unique_ids() {
        let r = super::rules();
        let mut ids: Vec<&str> = r.iter().map(|x| x.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), r.len());
    }
}
