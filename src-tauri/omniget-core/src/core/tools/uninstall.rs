//! Desinstalador (estudo 10, Kudu): lista os apps instalados, desinstala e
//! procura as sobras (Application Support, caches, preferências, configs).
//! macOS: `.app` em /Applications e ~/Applications; Windows: chaves Uninstall
//! do registro (executa o UninstallString); Linux: dpkg/rpm (via pkexec),
//! Flatpak, Snap e AppImages em ~/Applications.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    /// "app" | "msi" | "exe" | "deb" | "rpm" | "flatpak" | "snap" | "appimage"
    pub kind: String,
    pub path: String,
    pub bytes: u64,
    pub needs_admin: bool,
    /// Identificador secundário: bundle id (macOS), pacote (Linux), UninstallString (Windows).
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Leftover {
    pub path: String,
    pub bytes: u64,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn size_of(path: &Path) -> u64 {
    super::sysclean::measure(path).0
}

async fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let o = crate::core::process::command(program).args(args).output().await?;
    if !o.status.success() {
        return Err(anyhow!("{} falhou: {}", program, String::from_utf8_lossy(if o.stderr.is_empty() { &o.stdout } else { &o.stderr }).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

// ── macOS ──────────────────────────────────────────────────────────────

async fn mac_list(progress: &super::ProgressFn) -> Vec<App> {
    let mut out = Vec::new();
    let mut paths = Vec::new();
    for dir in [PathBuf::from("/Applications"), home().join("Applications")] {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "app").unwrap_or(false) {
                    paths.push(p);
                }
            }
        }
    }
    paths.sort();
    let total = paths.len() as u64;
    for (i, p) in paths.iter().enumerate() {
        super::report(progress, "uninstall", "progress", i as u64, Some(total), Some(p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()));
        let info = p.join("Contents/Info.plist");
        let json = crate::core::process::command("plutil")
            .args(["-convert", "json", "-o", "-"])
            .arg(&info)
            .output()
            .await
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
            .unwrap_or(serde_json::Value::Null);
        let bundle = json.get("CFBundleIdentifier").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = json
            .get("CFBundleDisplayName")
            .or_else(|| json.get("CFBundleName"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        let version = json.get("CFBundleShortVersionString").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let apple = bundle.starts_with("com.apple.");
        out.push(App {
            id: format!("mac:{}", p.display()),
            name,
            version,
            publisher: bundle.split('.').nth(1).unwrap_or("").to_string(),
            kind: "app".into(),
            path: p.to_string_lossy().to_string(),
            bytes: size_of(p),
            needs_admin: apple || std::fs::metadata(p).map(|m| !is_writable(&m)).unwrap_or(false),
            key: bundle,
        });
    }
    super::report(progress, "uninstall", "done", total, Some(total), None);
    out
}

#[cfg(unix)]
fn is_writable(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc_getuid() };
    m.uid() == uid || (m.mode() & 0o002) != 0
}

#[cfg(unix)]
unsafe fn libc_getuid() -> u32 {
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

#[cfg(not(unix))]
fn is_writable(_m: &std::fs::Metadata) -> bool {
    true
}

fn mac_leftovers(app: &App) -> Vec<Leftover> {
    let h = home();
    let lib = h.join("Library");
    let mut out = Vec::new();
    let stem = Path::new(&app.path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let needles: Vec<String> = [app.key.clone(), stem.clone(), app.name.clone()].into_iter().filter(|s| s.len() >= 3).map(|s| s.to_lowercase()).collect();
    let dirs = [
        "Application Support",
        "Caches",
        "Preferences",
        "Saved Application State",
        "Containers",
        "Group Containers",
        "Logs",
        "WebKit",
        "HTTPStorages",
        "Cookies",
        "LaunchAgents",
        "Application Scripts",
    ];
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(lib.join(d)) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let hit = needles.iter().any(|n| {
                if n.contains('.') {
                    name == *n || name.starts_with(&format!("{}.", n)) || name.starts_with(&format!("{}-", n))
                } else {
                    name == *n || name == format!("{}.plist", n) || name.starts_with(&format!("{}.", n))
                }
            });
            if hit {
                out.push(Leftover { path: e.path().to_string_lossy().to_string(), bytes: size_of(&e.path()) });
            }
        }
    }
    out
}

// ── Windows ────────────────────────────────────────────────────────────

const UNINSTALL_KEYS: &[(&str, bool)] = &[
    (r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", true),
    (r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall", true),
    (r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall", false),
];

async fn win_list(progress: &super::ProgressFn) -> Vec<App> {
    let mut out = Vec::new();
    for (i, (root, admin)) in UNINSTALL_KEYS.iter().enumerate() {
        super::report(progress, "uninstall", "progress", i as u64, Some(3), Some(root.to_string()));
        for key in super::win_registry::reg_query(root, true).await {
            if key.path.eq_ignore_ascii_case(root) {
                continue;
            }
            let get = |n: &str| key.values.iter().find(|v| v.name.eq_ignore_ascii_case(n)).map(|v| v.data.clone()).unwrap_or_default();
            let name = get("DisplayName");
            if name.trim().is_empty() || get("SystemComponent") == "0x1" || !get("ParentKeyName").is_empty() && get("ReleaseType").contains("Update") {
                continue;
            }
            let unins = if get("QuietUninstallString").is_empty() { get("UninstallString") } else { get("QuietUninstallString") };
            if unins.trim().is_empty() {
                continue;
            }
            let kb = u64::from_str_radix(get("EstimatedSize").trim_start_matches("0x"), 16).unwrap_or(0);
            let kind = if unins.to_ascii_lowercase().contains("msiexec") { "msi" } else { "exe" };
            out.push(App {
                id: format!("win:{}", key.path),
                name,
                version: get("DisplayVersion"),
                publisher: get("Publisher"),
                kind: kind.into(),
                path: super::win_registry::expand_env(&get("InstallLocation")),
                bytes: kb * 1024,
                needs_admin: *admin,
                key: unins,
            });
        }
    }
    super::report(progress, "uninstall", "done", 3, Some(3), None);
    out
}

fn win_leftovers(app: &App) -> Vec<Leftover> {
    let mut out = Vec::new();
    if !app.path.trim().is_empty() && Path::new(app.path.trim()).exists() {
        out.push(Leftover { path: app.path.clone(), bytes: size_of(Path::new(&app.path)) });
    }
    let needles: Vec<String> = [app.name.clone(), app.publisher.clone()].into_iter().filter(|s| s.len() >= 3).map(|s| s.to_lowercase()).collect();
    let mut bases = Vec::new();
    for var in ["APPDATA", "LOCALAPPDATA", "ProgramData"] {
        if let Ok(v) = std::env::var(var) {
            bases.push(PathBuf::from(v));
        }
    }
    if let Ok(v) = std::env::var("LOCALAPPDATA") {
        bases.push(PathBuf::from(v).join("Programs"));
    }
    for base in bases {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if needles.contains(&name) {
                let p = e.path();
                if p.to_string_lossy() != app.path {
                    out.push(Leftover { path: p.to_string_lossy().to_string(), bytes: size_of(&p) });
                }
            }
        }
    }
    out
}

// ── Linux ──────────────────────────────────────────────────────────────

async fn linux_list(progress: &super::ProgressFn) -> Vec<App> {
    let mut out = Vec::new();
    super::report(progress, "uninstall", "progress", 0, Some(4), Some("flatpak".into()));
    if let Ok(list) = run("flatpak", &["list", "--app", "--columns=application,name,version,installation,size"]).await {
        for l in list.lines() {
            let cols: Vec<&str> = l.split('\t').collect();
            if cols.len() < 2 {
                continue;
            }
            let id = cols[0].trim();
            let size = cols.get(4).map(|s| parse_size(s)).unwrap_or(0);
            out.push(App {
                id: format!("flatpak:{}", id),
                name: cols[1].trim().to_string(),
                version: cols.get(2).map(|s| s.trim().to_string()).unwrap_or_default(),
                publisher: id.rsplit('.').nth(1).unwrap_or("").to_string(),
                kind: "flatpak".into(),
                path: home().join(".var/app").join(id).to_string_lossy().to_string(),
                bytes: size,
                needs_admin: cols.get(3).map(|s| s.trim() == "system").unwrap_or(false),
                key: id.to_string(),
            });
        }
    }
    super::report(progress, "uninstall", "progress", 1, Some(4), Some("snap".into()));
    if let Ok(list) = run("snap", &["list"]).await {
        for l in list.lines().skip(1) {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if cols.len() < 2 || matches!(cols[0], "core" | "core18" | "core20" | "core22" | "core24" | "snapd" | "bare") || cols[0].starts_with("gnome-") || cols[0].starts_with("gtk-common") {
                continue;
            }
            out.push(App {
                id: format!("snap:{}", cols[0]),
                name: cols[0].to_string(),
                version: cols[1].to_string(),
                publisher: cols.get(4).unwrap_or(&"").to_string(),
                kind: "snap".into(),
                path: format!("/snap/{}", cols[0]),
                bytes: size_of(Path::new(&format!("/snap/{}", cols[0]))),
                needs_admin: true,
                key: cols[0].to_string(),
            });
        }
    }
    super::report(progress, "uninstall", "progress", 2, Some(4), Some("AppImage".into()));
    for dir in [home().join("Applications"), home().join(".local/bin"), home().join("AppImages")] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x.eq_ignore_ascii_case("appimage")).unwrap_or(false) {
                out.push(App {
                    id: format!("appimage:{}", p.display()),
                    name: p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    version: String::new(),
                    publisher: String::new(),
                    kind: "appimage".into(),
                    path: p.to_string_lossy().to_string(),
                    bytes: size_of(&p),
                    needs_admin: false,
                    key: String::new(),
                });
            }
        }
    }
    super::report(progress, "uninstall", "progress", 3, Some(4), Some("pacotes".into()));
    // Pacotes com .desktop (apps de verdade, não bibliotecas)
    let mut desktop_pkgs = std::collections::HashSet::new();
    if let Ok(list) = run("dpkg-query", &["-W", "-f=${Package}\t${Version}\t${Installed-Size}\t${Maintainer}\n"]).await {
        if let Ok(files) = run("bash", &["-lc", "dpkg -S /usr/share/applications/*.desktop 2>/dev/null | cut -d: -f1 | sort -u"]).await {
            desktop_pkgs.extend(files.lines().map(|s| s.trim().to_string()));
        }
        for l in list.lines() {
            let cols: Vec<&str> = l.split('\t').collect();
            if cols.len() < 3 || !desktop_pkgs.contains(cols[0]) {
                continue;
            }
            out.push(App {
                id: format!("deb:{}", cols[0]),
                name: cols[0].to_string(),
                version: cols[1].to_string(),
                publisher: cols.get(3).unwrap_or(&"").split('<').next().unwrap_or("").trim().to_string(),
                kind: "deb".into(),
                path: String::new(),
                bytes: cols[2].trim().parse::<u64>().unwrap_or(0) * 1024,
                needs_admin: true,
                key: cols[0].to_string(),
            });
        }
    } else if let Ok(list) = run("rpm", &["-qa", "--queryformat", "%{NAME}\t%{VERSION}\t%{SIZE}\t%{VENDOR}\n"]).await {
        if let Ok(files) = run("bash", &["-lc", "rpm -qf /usr/share/applications/*.desktop 2>/dev/null | sort -u"]).await {
            for f in files.lines() {
                // nome-versao-release.arch → nome
                if let Some(idx) = f.rfind('-').and_then(|i| f[..i].rfind('-')) {
                    desktop_pkgs.insert(f[..idx].to_string());
                }
            }
        }
        for l in list.lines() {
            let cols: Vec<&str> = l.split('\t').collect();
            if cols.len() < 3 || !desktop_pkgs.contains(cols[0]) {
                continue;
            }
            out.push(App {
                id: format!("rpm:{}", cols[0]),
                name: cols[0].to_string(),
                version: cols[1].to_string(),
                publisher: cols.get(3).unwrap_or(&"").to_string(),
                kind: "rpm".into(),
                path: String::new(),
                bytes: cols[2].trim().parse::<u64>().unwrap_or(0),
                needs_admin: true,
                key: cols[0].to_string(),
            });
        }
    }
    super::report(progress, "uninstall", "done", 4, Some(4), None);
    out
}

fn parse_size(s: &str) -> u64 {
    let s = s.trim().replace(',', ".");
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let v: f64 = num.trim().parse().unwrap_or(0.0);
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "kb" | "kib" => 1024.0,
        "mb" | "mib" => 1024.0 * 1024.0,
        "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (v * mult) as u64
}

fn linux_leftovers(app: &App) -> Vec<Leftover> {
    let h = home();
    let mut out = Vec::new();
    let needles: Vec<String> = [app.key.clone(), app.name.clone()].into_iter().filter(|s| s.len() >= 3).map(|s| s.to_lowercase()).collect();
    for base in [h.join(".config"), h.join(".local/share"), h.join(".cache"), h.join(".var/app")] {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if needles.contains(&name) {
                out.push(Leftover { path: e.path().to_string_lossy().to_string(), bytes: size_of(&e.path()) });
            }
        }
    }
    out
}

// ── API ────────────────────────────────────────────────────────────────

pub async fn list(progress: super::ProgressFn) -> Vec<App> {
    let mut apps = if cfg!(target_os = "macos") {
        mac_list(&progress).await
    } else if cfg!(target_os = "windows") {
        win_list(&progress).await
    } else {
        linux_list(&progress).await
    };
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub fn leftovers(app: &App) -> Vec<Leftover> {
    if cfg!(target_os = "macos") {
        mac_leftovers(app)
    } else if cfg!(target_os = "windows") {
        win_leftovers(app)
    } else {
        linux_leftovers(app)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UninstallResult {
    pub ok: bool,
    pub message: String,
    pub trashed: Vec<String>,
    pub failed: Vec<String>,
}

/// Desinstala o app e, se pedido, manda as sobras para a lixeira.
pub async fn uninstall(app: &App, leftover_paths: &[String]) -> UninstallResult {
    let mut r = UninstallResult::default();
    let res: anyhow::Result<String> = if cfg!(target_os = "macos") {
        match trash::delete(&app.path) {
            Ok(_) => Ok("movido para a Lixeira".into()),
            Err(e) => {
                // Sem permissão: pede ao Finder (dialogo de senha do sistema)
                let script = format!("tell application \"Finder\" to delete POSIX file \"{}\"", app.path.replace('"', "\\\""));
                run("osascript", &["-e", &script]).await.map(|_| "movido para a Lixeira".into()).map_err(|_| anyhow!("{}", e))
            }
        }
    } else if cfg!(target_os = "windows") {
        let cmd = app.key.trim();
        let lower = cmd.to_ascii_lowercase();
        if lower.contains("msiexec") {
            // MsiExec.exe /I{GUID} → /X{GUID} silencioso
            let guid = cmd.split(['{', '}']).nth(1).map(|g| format!("{{{}}}", g)).unwrap_or_default();
            if guid.is_empty() {
                Err(anyhow!("UninstallString sem GUID: {}", cmd))
            } else {
                run("msiexec", &["/x", &guid, "/passive", "/norestart"]).await.map(|_| "desinstalado".into())
            }
        } else {
            run("cmd", &["/C", cmd]).await.map(|_| "desinstalador executado".into())
        }
    } else {
        match app.kind.as_str() {
            "flatpak" => run("flatpak", &["uninstall", "-y", "--delete-data", if app.needs_admin { "--system" } else { "--user" }, &app.key]).await.map(|_| "desinstalado".into()),
            "snap" => run("pkexec", &["snap", "remove", &app.key]).await.map(|_| "desinstalado".into()),
            "deb" => run("pkexec", &["apt-get", "remove", "-y", &app.key]).await.map(|_| "desinstalado".into()),
            "rpm" => run("pkexec", &["dnf", "remove", "-y", &app.key]).await.map(|_| "desinstalado".into()),
            _ => trash::delete(&app.path).map(|_| "movido para a lixeira".to_string()).map_err(|e| anyhow!("{}", e)),
        }
    };
    match res {
        Ok(m) => {
            r.ok = true;
            r.message = m;
        }
        Err(e) => {
            r.message = e.to_string();
            return r;
        }
    }
    for p in leftover_paths {
        match trash::delete(p) {
            Ok(_) => r.trashed.push(p.clone()),
            Err(_) => r.failed.push(p.clone()),
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1.5 MB"), 1_572_864);
        assert_eq!(parse_size("300 kB"), 307_200);
        assert_eq!(parse_size("2,0 GB"), 2_147_483_648);
        assert_eq!(parse_size("bytes"), 0);
    }
}

#[cfg(test)]
mod live {
    /// `cargo test -p omniget-core --lib tools::uninstall::live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn list_here() {
        let apps = super::list(super::super::noop_progress()).await;
        for a in apps.iter().take(15) {
            println!("{:<9} {:<40} {:<10} {:>10} admin={} {}", a.kind, a.name.chars().take(40).collect::<String>(), a.version, a.bytes, a.needs_admin, a.key);
        }
        println!("{} apps", apps.len());
        if let Some(a) = apps.iter().find(|a| a.name.to_lowercase().contains("spotify") || a.name.to_lowercase().contains("discord") || a.name.to_lowercase().contains("code")) {
            for l in super::leftovers(a) {
                println!("  sobra: {} ({})", l.path, l.bytes);
            }
        }
    }
}
