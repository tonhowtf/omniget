//! Busca instantânea de arquivos (estudos 26, 27): Everything (`es.exe`) no
//! Windows, Spotlight (`mdfind`) no macOS, `fd`/`find` no Linux.

#[allow(unused_imports)]
use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SearchBackend {
    pub name: String,
    pub available: bool,
    pub path: Option<String>,
    pub install_hint: String,
}

#[cfg(target_os = "windows")]
async fn es_path() -> Option<PathBuf> {
    if let Some(p) = crate::core::dependencies::find_tool("es").await {
        return Some(p);
    }
    for base in [
        "C:\\Program Files\\Everything",
        "C:\\Program Files (x86)\\Everything",
    ] {
        let p = PathBuf::from(base).join("es.exe");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub async fn backend() -> SearchBackend {
    #[cfg(target_os = "windows")]
    {
        let p = es_path().await;
        return SearchBackend {
            name: "Everything".into(),
            available: p.is_some(),
            path: p.map(|p| p.to_string_lossy().to_string()),
            install_hint: "winget install voidtools.Everything voidtools.Everything.Cli".into(),
        };
    }
    #[cfg(target_os = "macos")]
    {
        SearchBackend {
            name: "Spotlight".into(),
            available: true,
            path: Some("/usr/bin/mdfind".into()),
            install_hint: String::new(),
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let fd = crate::core::dependencies::find_tool("fd")
            .await
            .or(crate::core::dependencies::find_tool("fdfind").await);
        SearchBackend {
            name: if fd.is_some() {
                "fd".into()
            } else {
                "find".into()
            },
            available: true,
            path: fd.map(|p| p.to_string_lossy().to_string()),
            install_hint: "sudo apt install fd-find (opcional, mais rapido)".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub path: String,
    pub size: Option<u64>,
    pub is_dir: bool,
}

pub async fn search(query: &str, folder: &str, limit: usize) -> anyhow::Result<Vec<Hit>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let limit = limit.clamp(1, 2000);
    let output;
    #[cfg(target_os = "windows")]
    {
        let es = es_path()
            .await
            .ok_or_else(|| anyhow!("Everything (es.exe) nao encontrado"))?;
        let mut cmd = crate::core::process::command(&es);
        cmd.args(["-n", &limit.to_string()]);
        if !folder.trim().is_empty() {
            cmd.args(["-path", folder.trim()]);
        }
        cmd.arg(q);
        output = cmd.output().await?;
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = crate::core::process::command("/usr/bin/mdfind");
        if !folder.trim().is_empty() {
            cmd.args(["-onlyin", folder.trim()]);
        }
        cmd.args(["-name", q]);
        output = cmd.output().await?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let root = if folder.trim().is_empty() {
            dirs::home_dir()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|| "/".into())
        } else {
            folder.trim().to_string()
        };
        let fd = crate::core::dependencies::find_tool("fd")
            .await
            .or(crate::core::dependencies::find_tool("fdfind").await);
        output = match fd {
            Some(fd) => {
                crate::core::process::command(&fd)
                    .args(["-i", "--max-results", &limit.to_string(), q, &root])
                    .output()
                    .await?
            }
            None => {
                crate::core::process::command("find")
                    .args([&root, "-iname", &format!("*{}*", q)])
                    .output()
                    .await?
            }
        };
    }
    if !output.status.success() && output.stdout.is_empty() {
        return Err(anyhow!(
            "busca falhou: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hits = text
        .lines()
        .map(|l| l.trim().trim_end_matches('\r'))
        .filter(|l| !l.is_empty())
        .take(limit)
        .map(|l| {
            let meta = std::fs::metadata(l).ok();
            Hit {
                path: l.to_string(),
                size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                is_dir: meta.map(|m| m.is_dir()).unwrap_or(false),
            }
        })
        .collect();
    Ok(hits)
}
