//! ⚠️ SOMENTE WINDOWS. Limpar registro (estudo 10, Kudu): encontra entradas
//! órfãs, isto é, chaves e valores que apontam para arquivos que não existem
//! mais. Cada remoção é precedida de um `reg export` da chave para
//! `<app_data>/tools/registry-backups/`, para o usuário poder importar de
//! volta. Fora do Windows tudo devolve vazio.
//!
//! Também abriga os utilitários de `reg query` que Inicialização e
//! Desinstalador reutilizam. Tudo passa pelo `reg.exe`, sem crate de registro.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RegValue {
    pub name: String,
    pub kind: String,
    pub data: String,
}

#[derive(Debug, Clone, Default)]
pub struct RegKey {
    pub path: String,
    pub values: Vec<RegValue>,
    pub subkeys: Vec<String>,
}

/// Interpreta a saída de `reg query` (uma ou mais chaves).
pub fn parse_reg_output(text: &str) -> Vec<RegKey> {
    let mut keys: Vec<RegKey> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if !raw.starts_with(' ') && (line.starts_with("HKEY_") || line.starts_with("HK")) {
            // Subchave da chave anterior ou chave nova?
            if let Some(last) = keys.last_mut() {
                if line.starts_with(&last.path) && line.len() > last.path.len() && line[last.path.len()..].starts_with('\\') {
                    last.subkeys.push(line.to_string());
                    // A própria subchave também pode listar valores a seguir
                    // quando o /s foi usado; abre como chave.
                }
            }
            keys.push(RegKey { path: line.to_string(), ..Default::default() });
            continue;
        }
        let Some(key) = keys.last_mut() else { continue };
        let t = line.trim_start();
        // "<nome>    REG_TIPO    <dados>"
        if let Some(idx) = t.find("    REG_") {
            let name = t[..idx].trim().to_string();
            let rest = t[idx + 4..].trim_start();
            let (kind, data) = match rest.find("    ") {
                Some(i) => (rest[..i].trim().to_string(), rest[i + 4..].trim().to_string()),
                None => (rest.trim().to_string(), String::new()),
            };
            key.values.push(RegValue { name: if name == "(Default)" || name == "(Padrão)" { String::new() } else { name }, kind, data });
        }
    }
    keys
}

pub async fn reg_query(key: &str, recursive: bool) -> Vec<RegKey> {
    if !cfg!(target_os = "windows") {
        return vec![];
    }
    let mut cmd = crate::core::process::command("reg");
    cmd.arg("query").arg(key);
    if recursive {
        cmd.arg("/s");
    }
    match cmd.output().await {
        Ok(o) if o.status.success() => parse_reg_output(&String::from_utf8_lossy(&o.stdout)),
        _ => vec![],
    }
}

pub async fn reg_delete(key: &str, value: Option<&str>) -> anyhow::Result<()> {
    let mut cmd = crate::core::process::command("reg");
    cmd.arg("delete").arg(key);
    match value {
        Some("") => {
            cmd.arg("/ve");
        }
        Some(v) => {
            cmd.arg("/v").arg(v);
        }
        None => {}
    }
    cmd.arg("/f");
    let o = cmd.output().await?;
    if !o.status.success() {
        return Err(anyhow!("reg delete falhou: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(())
}

pub async fn reg_add(key: &str, value: &str, kind: &str, data: &str) -> anyhow::Result<()> {
    let mut cmd = crate::core::process::command("reg");
    cmd.arg("add").arg(key);
    if value.is_empty() {
        cmd.arg("/ve");
    } else {
        cmd.arg("/v").arg(value);
    }
    cmd.arg("/t").arg(kind).arg("/d").arg(data).arg("/f");
    let o = cmd.output().await?;
    if !o.status.success() {
        return Err(anyhow!("reg add falhou: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(())
}

pub fn backups_dir() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("registry-backups"))
}

/// `reg export` da chave para um `.reg` com carimbo de hora. Devolve o caminho.
pub async fn reg_backup(key: &str) -> anyhow::Result<PathBuf> {
    let dir = backups_dir().ok_or_else(|| anyhow!("sem pasta de dados"))?;
    std::fs::create_dir_all(&dir)?;
    let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let file = dir.join(format!("{}-{}.reg", chrono::Local::now().format("%Y%m%d-%H%M%S"), safe.chars().take(80).collect::<String>()));
    let o = crate::core::process::command("reg").arg("export").arg(key).arg(&file).arg("/y").output().await?;
    if !o.status.success() {
        return Err(anyhow!("reg export falhou: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(file)
}

/// Expande `%VAR%` e devolve o executável de uma linha de comando do registro
/// (`"C:\x\a.exe" /arg`, `C:\x\a.exe /arg`, `rundll32 ...`).
pub fn command_exe(cmd: &str) -> Option<PathBuf> {
    let s = expand_env(cmd.trim());
    if s.is_empty() {
        return None;
    }
    let candidate = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next().unwrap_or("").to_string()
    } else {
        // sem aspas: até o primeiro ".exe" (com espaços no caminho) ou o primeiro token
        let lower = s.to_ascii_lowercase();
        if let Some(i) = lower.find(".exe") {
            s[..i + 4].to_string()
        } else {
            s.split_whitespace().next().unwrap_or("").to_string()
        }
    };
    let candidate = candidate.trim().trim_matches(',').to_string();
    if candidate.is_empty() {
        return None;
    }
    Some(PathBuf::from(candidate))
}

pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let var = &after[..end];
            match std::env::var(var) {
                Ok(v) => out.push_str(&v),
                Err(_) => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[end + 1..];
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Um executável "existe" se o caminho existe, ou se é só um nome que o
/// Windows resolve pelo PATH/App Paths (não dá para afirmar que falta).
fn exe_missing(p: &Path) -> bool {
    if p.exists() {
        return false;
    }
    let s = p.to_string_lossy();
    // Caminho absoluto (C:\...) ou UNC que não existe → órfão. Nome solto
    // (notepad.exe, rundll32) → não sabemos, não marcar.
    s.len() > 2 && (s.as_bytes()[1] == b':' || s.starts_with("\\\\"))
}

// ── Varredura de órfãos ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Orphan {
    pub id: String,
    /// "uninstall" | "app-paths" | "mui-cache" | "shared-dlls" | "run"
    pub category: String,
    pub key: String,
    /// `None` = apagar a chave inteira; `Some("")` = valor padrão.
    pub value: Option<String>,
    pub name: String,
    pub data: String,
    pub reason: String,
}

const UNINSTALL_KEYS: &[&str] = &[
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
];

const APP_PATH_KEYS: &[&str] = &[
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths",
];

const RUN_KEYS: &[&str] = &[
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
];

fn value_of<'a>(key: &'a RegKey, name: &str) -> Option<&'a str> {
    key.values.iter().find(|v| v.name.eq_ignore_ascii_case(name)).map(|v| v.data.as_str())
}

pub async fn scan(progress: &super::ProgressFn) -> Vec<Orphan> {
    let mut out = Vec::new();
    if !cfg!(target_os = "windows") {
        return out;
    }
    let mut n = 0u64;
    let mut push = |o: Orphan| {
        n += 1;
        out.push(o);
    };

    // 1) Programas desinstalados que deixaram a entrada
    super::report(progress, "winreg", "progress", 0, Some(5), Some("Uninstall".into()));
    for root in UNINSTALL_KEYS {
        for key in reg_query(root, true).await {
            if key.path.eq_ignore_ascii_case(root) {
                continue;
            }
            let Some(name) = value_of(&key, "DisplayName") else { continue };
            if value_of(&key, "SystemComponent") == Some("0x1") {
                continue;
            }
            let loc = value_of(&key, "InstallLocation").map(expand_env).unwrap_or_default();
            let unins = value_of(&key, "UninstallString").and_then(command_exe);
            let icon = value_of(&key, "DisplayIcon").and_then(command_exe);
            let loc_missing = !loc.trim().is_empty() && !Path::new(loc.trim().trim_matches('"')).exists();
            let unins_missing = unins.as_ref().map(|p| exe_missing(p) && !p.to_string_lossy().to_ascii_lowercase().contains("msiexec")).unwrap_or(false);
            let icon_missing = icon.as_ref().map(|p| exe_missing(p)).unwrap_or(false);
            if (loc_missing && (unins_missing || icon_missing || unins.is_none())) || (unins_missing && loc.trim().is_empty()) {
                push(Orphan {
                    id: format!("u:{}", key.path),
                    category: "uninstall".into(),
                    key: key.path.clone(),
                    value: None,
                    name: name.to_string(),
                    data: if loc.is_empty() { unins.map(|p| p.to_string_lossy().to_string()).unwrap_or_default() } else { loc },
                    reason: "pasta e desinstalador nao existem mais".into(),
                });
            }
        }
    }

    // 2) App Paths apontando para exe inexistente
    super::report(progress, "winreg", "progress", 1, Some(5), Some("App Paths".into()));
    for root in APP_PATH_KEYS {
        for key in reg_query(root, true).await {
            if key.path.eq_ignore_ascii_case(root) {
                continue;
            }
            let Some(target) = value_of(&key, "") else { continue };
            if let Some(p) = command_exe(target) {
                if exe_missing(&p) {
                    push(Orphan {
                        id: format!("a:{}", key.path),
                        category: "app-paths".into(),
                        key: key.path.clone(),
                        value: None,
                        name: key.path.rsplit('\\').next().unwrap_or("").to_string(),
                        data: p.to_string_lossy().to_string(),
                        reason: "executavel nao existe".into(),
                    });
                }
            }
        }
    }

    // 3) MuiCache
    super::report(progress, "winreg", "progress", 2, Some(5), Some("MuiCache".into()));
    let mui = r"HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\Shell\MuiCache";
    for key in reg_query(mui, false).await {
        for v in &key.values {
            let file = v.name.split(".FriendlyAppName").next().unwrap_or("").split(".ApplicationCompany").next().unwrap_or("");
            if file.is_empty() {
                continue;
            }
            let p = PathBuf::from(expand_env(file));
            if exe_missing(&p) {
                push(Orphan {
                    id: format!("m:{}", v.name),
                    category: "mui-cache".into(),
                    key: mui.to_string(),
                    value: Some(v.name.clone()),
                    name: v.data.clone(),
                    data: p.to_string_lossy().to_string(),
                    reason: "programa nao existe".into(),
                });
            }
        }
    }

    // 4) SharedDLLs
    super::report(progress, "winreg", "progress", 3, Some(5), Some("SharedDLLs".into()));
    let shared = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\SharedDLLs";
    for key in reg_query(shared, false).await {
        for v in &key.values {
            let p = PathBuf::from(expand_env(&v.name));
            if exe_missing(&p) {
                push(Orphan {
                    id: format!("s:{}", v.name),
                    category: "shared-dlls".into(),
                    key: shared.to_string(),
                    value: Some(v.name.clone()),
                    name: p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    data: p.to_string_lossy().to_string(),
                    reason: "arquivo nao existe".into(),
                });
            }
        }
    }

    // 5) Run com executável sumido
    super::report(progress, "winreg", "progress", 4, Some(5), Some("Run".into()));
    for root in RUN_KEYS {
        for key in reg_query(root, false).await {
            for v in &key.values {
                if let Some(p) = command_exe(&v.data) {
                    if exe_missing(&p) {
                        push(Orphan {
                            id: format!("r:{}\\{}", key.path, v.name),
                            category: "run".into(),
                            key: key.path.clone(),
                            value: Some(v.name.clone()),
                            name: v.name.clone(),
                            data: v.data.clone(),
                            reason: "programa de inicializacao nao existe".into(),
                        });
                    }
                }
            }
        }
    }
    super::report(progress, "winreg", "done", 5, Some(5), None);
    let _ = n;
    out
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FixResult {
    pub removed: usize,
    pub backups: Vec<String>,
    pub failed: Vec<String>,
}

/// Remove os itens indicados (por `id`), exportando cada chave antes.
pub async fn fix(items: &[Orphan]) -> FixResult {
    let mut r = FixResult::default();
    let mut backed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in items {
        if !backed.contains(&item.key) {
            match reg_backup(&item.key).await {
                Ok(p) => {
                    r.backups.push(p.to_string_lossy().to_string());
                    backed.insert(item.key.clone());
                }
                Err(e) => {
                    r.failed.push(format!("{}: backup falhou ({})", item.name, e));
                    continue;
                }
            }
        }
        match reg_delete(&item.key, item.value.as_deref()).await {
            Ok(_) => r.removed += 1,
            Err(e) => r.failed.push(format!("{}: {}", item.name, e)),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let text = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    OneDrive    REG_SZ    \"C:\\Users\\x\\OneDrive.exe\" /background\r\n    Foo Bar    REG_EXPAND_SZ    %LOCALAPPDATA%\\foo.exe\r\n    (Default)    REG_SZ    x\r\n\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\Sub\r\n";
        let keys = parse_reg_output(text);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].values.len(), 3);
        assert_eq!(keys[0].values[0].name, "OneDrive");
        assert_eq!(keys[0].values[0].kind, "REG_SZ");
        assert!(keys[0].values[0].data.starts_with("\"C:\\Users"));
        assert_eq!(keys[0].values[1].name, "Foo Bar");
        assert_eq!(keys[0].values[2].name, "");
        assert_eq!(keys[0].subkeys.len(), 1);
    }

    #[test]
    fn exe() {
        assert_eq!(command_exe("\"C:\\a b\\x.exe\" /q").unwrap(), PathBuf::from("C:\\a b\\x.exe"));
        assert_eq!(command_exe("C:\\a b\\x.exe /q").unwrap(), PathBuf::from("C:\\a b\\x.exe"));
        assert_eq!(command_exe("rundll32 foo").unwrap(), PathBuf::from("rundll32"));
        assert!(command_exe("").is_none());
        assert!(exe_missing(Path::new("C:\\nao\\existe\\x.exe")));
        assert!(!exe_missing(Path::new("notepad.exe")));
    }
}
