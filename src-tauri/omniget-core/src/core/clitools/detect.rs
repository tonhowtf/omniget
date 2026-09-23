//! Detecção: caminho resolvido + real, versão (com timeout), app de GUI e
//! prova de dono. O resultado da última varredura fica em memória; nada roda
//! sozinho, só quando a tela pede (`detect_all(force)`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::stream::{self, StreamExt};
use serde::Serialize;

use super::exec::{self, parse_version, real_path, run_capture};
use super::owner::{self, Owner, OwnerMethod, Probes};
use super::table::{self, CliTool};

pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub id: String,
    pub installed: bool,
    /// Qual dos binários da tabela foi achado.
    pub binary: Option<String>,
    pub resolved_path: Option<PathBuf>,
    pub real_path: Option<PathBuf>,
    pub version: Option<String>,
    /// Texto cru do `--version` quando não deu para extrair a versão.
    pub version_raw: Option<String>,
    pub app_path: Option<PathBuf>,
    pub app_version: Option<String>,
    pub owner: Owner,
    pub checked_at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn last() -> &'static Mutex<Option<Arc<Vec<Detection>>>> {
    use std::sync::OnceLock;
    static L: OnceLock<Mutex<Option<Arc<Vec<Detection>>>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(None))
}

/// Última varredura (sem rodar nada).
pub fn cached() -> Option<Arc<Vec<Detection>>> {
    last().lock().ok().and_then(|g| g.clone())
}

pub fn cached_one(id: &str) -> Option<Detection> {
    cached().and_then(|v| v.iter().find(|d| d.id == id).cloned())
}

fn store(list: Vec<Detection>) -> Arc<Vec<Detection>> {
    let arc = Arc::new(list);
    if let Ok(mut g) = last().lock() {
        *g = Some(arc.clone());
    }
    arc
}

fn replace_one(det: Detection) {
    if let Ok(mut g) = last().lock() {
        if let Some(cur) = g.as_ref() {
            let mut v: Vec<Detection> = cur.iter().cloned().collect();
            match v.iter_mut().find(|d| d.id == det.id) {
                Some(slot) => *slot = det,
                None => v.push(det),
            }
            *g = Some(Arc::new(v));
        }
    }
}

/// Varre todas as ferramentas (com cache, a não ser com `force`).
pub async fn detect_all(force: bool) -> Arc<Vec<Detection>> {
    if !force {
        if let Some(c) = cached() {
            return c;
        }
    }
    let probes = Arc::new(Probes::default());
    let extra = extra_dirs(&probes).await;
    let tools = table::all();
    let mut found: Vec<Detection> = stream::iter(tools.iter())
        .map(|t| {
            let probes = probes.clone();
            let extra = extra.clone();
            async move { detect_with(t, &probes, &extra).await }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let order: BTreeMap<&str, usize> = tools
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    found.sort_by_key(|d| order.get(d.id.as_str()).copied().unwrap_or(usize::MAX));
    store(found)
}

/// Detecta uma ferramenta de novo (fresh) e atualiza o cache.
pub async fn detect_one(tool: &CliTool) -> Detection {
    let probes = Probes::default();
    let extra = extra_dirs(&probes).await;
    let det = detect_with(tool, &probes, &extra).await;
    replace_one(det.clone());
    det
}

/// `<npm prefix -g>/bin`: no macOS a GUI não herda o PATH do nvm.
async fn extra_dirs(probes: &Probes) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = probes.npm_prefix().await {
        v.push(if cfg!(windows) {
            p.clone()
        } else {
            p.join("bin")
        });
    }
    // Binários que o OmniGet baixou do registro ACP (dono = omniget).
    for launch in super::acp_registry::installed().values() {
        if launch.dir.is_some() {
            if let Some(parent) = Path::new(&launch.command).parent() {
                v.push(parent.to_path_buf());
            }
        }
    }
    v
}

async fn detect_with(tool: &CliTool, probes: &Probes, extra: &[PathBuf]) -> Detection {
    let dirs = exec::search_dirs(extra);
    let hit = tool
        .binaries
        .iter()
        .find_map(|b| exec::which_in(b, &dirs).map(|p| (b.clone(), p)));
    let (app_path, app_version) = find_app(tool).await;

    let Some((binary, resolved)) = hit else {
        let owner = if app_path.is_some() {
            Owner {
                method: OwnerMethod::App,
                proven: false,
                prefix: app_path.clone(),
                evidence: if tool.auto_updates {
                    "app de GUI; atualiza sozinho".into()
                } else {
                    "app de GUI".into()
                },
                update: None,
                manual: None,
            }
        } else {
            Owner::unknown("não instalado", super::plan::manual_install_line(tool))
        };
        return Detection {
            id: tool.id.clone(),
            installed: app_path.is_some(),
            binary: None,
            resolved_path: None,
            real_path: None,
            version: app_version.clone(),
            version_raw: None,
            app_path,
            app_version,
            owner,
            checked_at: now(),
        };
    };

    let real = real_path(&resolved);
    let (version, version_raw) = read_version(tool, &resolved).await;
    let candidate = owner::classify(tool, &resolved, &real);
    let owner = owner::prove(tool, &resolved, &real, candidate, probes).await;
    Detection {
        id: tool.id.clone(),
        installed: true,
        binary: Some(binary),
        resolved_path: Some(resolved),
        real_path: Some(real),
        version,
        version_raw,
        app_path,
        app_version,
        owner,
        checked_at: now(),
    }
}

async fn read_version(tool: &CliTool, bin: &Path) -> (Option<String>, Option<String>) {
    // Nunca deixa o CLI se atualizar sozinho durante a sondagem.
    let mut env = BTreeMap::new();
    env.insert("NO_COLOR".into(), "1".into());
    env.insert("CI".into(), "1".into());
    env.insert("DISABLE_AUTOUPDATER".into(), "1".into());
    env.insert("AUGMENT_DISABLE_AUTO_UPDATE".into(), "1".into());
    match run_capture(bin, &tool.version_args, &env, VERSION_TIMEOUT).await {
        Some(out) => {
            let text = format!("{}\n{}", out.stdout, out.stderr);
            match parse_version(&text) {
                Some(v) => (Some(v), None),
                None => (None, Some(out.first_line().chars().take(120).collect())),
            }
        }
        None => (None, Some("timeout".into())),
    }
}

/// Apps de GUI no macOS (`/Applications` e `~/Applications`).
async fn find_app(tool: &CliTool) -> (Option<PathBuf>, Option<String>) {
    if !cfg!(target_os = "macos") {
        return (None, None);
    }
    let roots = [
        PathBuf::from("/Applications"),
        table::expand_home("~/Applications"),
    ];
    for name in &tool.apps.macos {
        for root in &roots {
            let app = root.join(name);
            if app.is_dir() {
                let version = app_version(&app).await;
                return (Some(app), version);
            }
        }
    }
    (None, None)
}

async fn app_version(app: &Path) -> Option<String> {
    let plist = app.join("Contents").join("Info.plist");
    let bytes = std::fs::read(&plist).ok()?;
    if bytes.starts_with(b"<?xml")
        || bytes.starts_with(b"<!DOCTYPE")
        || bytes.starts_with(b"<plist")
    {
        let text = String::from_utf8_lossy(&bytes);
        return plist_string(&text, "CFBundleShortVersionString");
    }
    // plist binário: o `plutil` do sistema converte.
    let out = run_capture(
        Path::new("/usr/bin/plutil"),
        &[
            "-extract".into(),
            "CFBundleShortVersionString".into(),
            "raw".into(),
            plist.to_string_lossy().into_owned(),
        ],
        &BTreeMap::new(),
        Duration::from_secs(3),
    )
    .await?;
    out.ok()
        .then(|| out.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Valor `<string>` depois de `<key>name</key>` num plist XML.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    let k = format!("<key>{key}</key>");
    let rest = &xml[xml.find(&k)? + k.len()..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    let v = rest[start..start + end].trim();
    (!v.is_empty()).then(|| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_xml_simples() {
        let xml = "<plist><dict><key>CFBundleName</key><string>Zed</string>\n<key>CFBundleShortVersionString</key>\n\t<string>0.210.4</string></dict></plist>";
        assert_eq!(
            plist_string(xml, "CFBundleShortVersionString").as_deref(),
            Some("0.210.4")
        );
        assert_eq!(plist_string(xml, "Nada"), None);
    }
}
