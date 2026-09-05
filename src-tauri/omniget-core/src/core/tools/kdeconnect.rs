//! KDE Connect pela CLI oficial (estudo 30): listar dispositivos pareados,
//! mandar arquivo/URL/texto, ping. Sem daemon, a tool explica como instalar.

use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub reachable: bool,
    pub paired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct KdeStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub devices: Vec<Device>,
    pub install_hint: String,
}

async fn locate() -> Option<PathBuf> {
    if let Some(p) = crate::core::dependencies::find_tool("kdeconnect-cli").await {
        return Some(p);
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/kdeconnect-indicator.app/Contents/MacOS/kdeconnect-cli",
            "/Applications/KDE Connect.app/Contents/MacOS/kdeconnect-cli",
            "/opt/homebrew/bin/kdeconnect-cli",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            r"C:\Program Files\KDE Connect\bin\kdeconnect-cli.exe",
            r"C:\Program Files\KDE\bin\kdeconnect-cli.exe",
        ]
    } else {
        &[
            "/usr/bin/kdeconnect-cli",
            "/usr/lib/kdeconnectd/kdeconnect-cli",
            "/app/bin/kdeconnect-cli",
        ]
    };
    candidates.iter().map(PathBuf::from).find(|p| p.exists())
}

/// `kdeconnect-cli -l` imprime "- Nome: id (paired and reachable)".
pub fn parse_devices(text: &str) -> Vec<Device> {
    let re = regex::Regex::new(r"^- (.+?): (\S+) \((.*)\)$").unwrap();
    text.lines()
        .filter_map(|l| re.captures(l.trim()))
        .map(|c| Device {
            name: c[1].to_string(),
            id: c[2].to_string(),
            reachable: c[3].contains("reachable"),
            paired: c[3].contains("paired"),
        })
        .collect()
}

pub async fn status() -> KdeStatus {
    let hint = if cfg!(target_os = "macos") {
        "brew install --cask kde-connect (ou use o Soduto)".to_string()
    } else if cfg!(target_os = "windows") {
        "winget install KDE.KDEConnect".to_string()
    } else {
        "sudo apt install kdeconnect  ·  flatpak install org.kde.kdeconnect".to_string()
    };
    let Some(bin) = locate().await else {
        return KdeStatus {
            installed: false,
            path: None,
            devices: vec![],
            install_hint: hint,
        };
    };
    let devices = crate::core::process::command(&bin)
        .args(["-l"])
        .output()
        .await
        .map(|o| parse_devices(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    KdeStatus {
        installed: true,
        path: Some(bin.to_string_lossy().to_string()),
        devices,
        install_hint: hint,
    }
}

async fn run(args: &[&str]) -> anyhow::Result<String> {
    let bin = locate()
        .await
        .ok_or_else(|| anyhow!("kdeconnect-cli nao encontrado"))?;
    let out = crate::core::process::command(&bin)
        .args(args)
        .output()
        .await?;
    if !out.status.success() {
        return Err(anyhow!(
            "kdeconnect-cli falhou: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn share_file(device: &str, path: &str) -> anyhow::Result<String> {
    run(&["-d", device, "--share", path]).await
}

pub async fn share_url(device: &str, url: &str) -> anyhow::Result<String> {
    run(&["-d", device, "--share", url]).await
}

pub async fn share_text(device: &str, text: &str) -> anyhow::Result<String> {
    run(&["-d", device, "--share-text", text]).await
}

pub async fn ping(device: &str, msg: &str) -> anyhow::Result<String> {
    if msg.trim().is_empty() {
        run(&["-d", device, "--ping"]).await
    } else {
        run(&["-d", device, "--ping-msg", msg]).await
    }
}

pub async fn refresh() -> anyhow::Result<String> {
    run(&["--refresh"]).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_list() {
        let t = "- Pixel 8: abc123 (paired and reachable)\n- Tablet: def (paired)\n1 device found";
        let d = super::parse_devices(t);
        assert_eq!(d.len(), 2);
        assert!(d[0].reachable && d[0].paired);
        assert!(!d[1].reachable);
    }
}
