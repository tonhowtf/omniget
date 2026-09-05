//! ⚠️ SOMENTE WINDOWS. Atualizar programas em massa (estudo 10, Kudu) com os
//! gerenciadores que já estiverem na máquina: winget, Chocolatey e Scoop.
//! Só lista e atualiza; não instala gerenciador nenhum. Fora do Windows tudo
//! devolve vazio.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outdated {
    /// "winget" | "choco" | "scoop"
    pub manager: String,
    pub id: String,
    pub name: String,
    pub current: String,
    pub available: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdaterStatus {
    pub winget: bool,
    pub choco: bool,
    pub scoop: bool,
    pub items: Vec<Outdated>,
}

async fn have(bin: &str) -> bool {
    crate::core::dependencies::find_tool(bin).await.is_some()
}

async fn output(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let o = crate::core::process::command(program).args(args).output().await?;
    // winget devolve códigos != 0 quando não há nada, então não falha por status.
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

/// Tabela do winget: as colunas são fixadas pela posição no cabeçalho
/// ("Name  Id  Version  Available  Source"). A linha de traços delimita.
pub fn parse_winget(text: &str) -> Vec<Outdated> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(sep) = lines.iter().position(|l| l.trim_start().starts_with("---")) else { return vec![] };
    if sep == 0 {
        return vec![];
    }
    let header = lines[sep - 1];
    let col = |name: &str| header.find(name);
    let (Some(id_i), Some(ver_i), Some(av_i)) = (col("Id"), col("Version"), col("Available")) else { return vec![] };
    let src_i = col("Source").unwrap_or(header.len());
    let cut = |l: &str, a: usize, b: usize| -> String {
        let chars: Vec<char> = l.chars().collect();
        let a = a.min(chars.len());
        let b = b.min(chars.len());
        chars[a..b].iter().collect::<String>().trim().to_string()
    };
    let mut out = Vec::new();
    for l in lines.iter().skip(sep + 1) {
        if l.trim().is_empty() || l.chars().count() < av_i {
            continue;
        }
        let name = cut(l, 0, id_i);
        let id = cut(l, id_i, ver_i);
        let current = cut(l, ver_i, av_i);
        let available = cut(l, av_i, src_i);
        if id.is_empty() || available.is_empty() || name.contains("upgrades available") {
            continue;
        }
        out.push(Outdated { manager: "winget".into(), id, name, current, available });
    }
    out
}

/// `choco outdated -r` → `nome|atual|disponivel|pinned`
pub fn parse_choco(text: &str) -> Vec<Outdated> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.trim().split('|').collect();
            if c.len() < 3 || c[0].is_empty() {
                return None;
            }
            Some(Outdated { manager: "choco".into(), id: c[0].into(), name: c[0].into(), current: c[1].into(), available: c[2].into() })
        })
        .collect()
}

/// `scoop status`: tabela "Name  Installed Version  Latest Version  ..."
pub fn parse_scoop(text: &str) -> Vec<Outdated> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(sep) = lines.iter().position(|l| l.trim_start().starts_with("----")) else { return vec![] };
    let mut out = Vec::new();
    for l in lines.iter().skip(sep + 1) {
        let cols: Vec<&str> = l.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        out.push(Outdated { manager: "scoop".into(), id: cols[0].into(), name: cols[0].into(), current: cols[1].into(), available: cols[2].into() });
    }
    out
}

pub async fn status() -> UpdaterStatus {
    let mut s = UpdaterStatus { winget: false, choco: false, scoop: false, items: vec![] };
    if !cfg!(target_os = "windows") {
        return s;
    }
    s.winget = have("winget").await;
    s.choco = have("choco").await;
    s.scoop = have("scoop").await;
    if s.winget {
        if let Ok(t) = output("winget", &["upgrade", "--include-unknown", "--accept-source-agreements", "--disable-interactivity"]).await {
            s.items.extend(parse_winget(&t));
        }
    }
    if s.choco {
        if let Ok(t) = output("choco", &["outdated", "-r", "--no-color"]).await {
            s.items.extend(parse_choco(&t));
        }
    }
    if s.scoop {
        if let Ok(t) = output("scoop", &["status"]).await {
            s.items.extend(parse_scoop(&t));
        }
    }
    s
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpgradeResult {
    pub upgraded: Vec<String>,
    pub failed: Vec<String>,
}

pub async fn upgrade(items: &[Outdated], progress: &super::ProgressFn) -> UpgradeResult {
    let mut r = UpgradeResult::default();
    let total = items.len() as u64;
    for (i, it) in items.iter().enumerate() {
        super::report(progress, "updater", "progress", i as u64, Some(total), Some(it.name.clone()));
        let res = match it.manager.as_str() {
            "winget" => {
                crate::core::process::command("winget")
                    .args(["upgrade", "--id", &it.id, "--exact", "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"])
                    .output()
                    .await
            }
            "choco" => crate::core::process::command("choco").args(["upgrade", &it.id, "-y", "--no-progress"]).output().await,
            "scoop" => crate::core::process::command("scoop").args(["update", &it.id]).output().await,
            _ => Err(std::io::Error::other("gerenciador desconhecido")),
        };
        match res {
            Ok(o) if o.status.success() => r.upgraded.push(it.name.clone()),
            Ok(o) => r.failed.push(format!("{}: {}", it.name, String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("").trim())),
            Err(e) => r.failed.push(format!("{}: {}", it.name, anyhow!(e))),
        }
    }
    super::report(progress, "updater", "done", total, Some(total), None);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winget_table() {
        let t = "\
Name                     Id                         Version   Available Source
-------------------------------------------------------------------------------
Google Chrome            Google.Chrome              120.0.1   121.0.2   winget
7-Zip 23.01 (x64)        7zip.7zip                  23.01     24.08     winget
2 upgrades available.
";
        let v = parse_winget(t);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, "Google.Chrome");
        assert_eq!(v[0].available, "121.0.2");
        assert_eq!(v[1].name, "7-Zip 23.01 (x64)");
        assert_eq!(v[1].current, "23.01");
    }

    #[test]
    fn choco_lines() {
        let v = parse_choco("git|2.40.0|2.44.0|false\nvlc|3.0.18|3.0.20|false\n");
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].id, "vlc");
        assert_eq!(v[1].available, "3.0.20");
    }

    #[test]
    fn scoop_table() {
        let t = "Scoop is up to date.\n\nName  Installed Version  Latest Version  Missing Dependencies  Info\n----  -----------------  --------------  --------------------  ----\nnodejs 20.1.0 20.5.0\n";
        let v = parse_scoop(t);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "nodejs");
        assert_eq!(v[0].available, "20.5.0");
    }
}
