//! Versão mais nova de cada ferramenta: registro do npm, PyPI ou releases do
//! GitHub (e `brew info` quando o dono provado é o brew, que atrasa em
//! relação ao npm). Cache de 1 h em memória e em disco; timeout de 4 s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::exec::{compare_versions, parse_version};
use super::table::{CliTool, LatestSource};

pub const TTL: Duration = Duration::from_secs(3600);
pub const TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionState {
    Unknown,
    Current,
    BehindLatest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Latest {
    pub id: String,
    pub source: String,
    pub key: String,
    pub latest: Option<String>,
    pub installed: Option<String>,
    pub state: VersionState,
    /// Segundos desde a época, de quando o valor foi buscado.
    pub checked_at: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    version: Option<String>,
    at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    use std::sync::OnceLock;
    static C: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(load_disk().unwrap_or_default()))
}

fn disk_path() -> Option<PathBuf> {
    Some(
        crate::core::paths::app_data_dir()?
            .join("clitools")
            .join("latest-cache.json"),
    )
}

fn load_disk() -> Option<HashMap<String, Entry>> {
    let text = std::fs::read_to_string(disk_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_disk(map: &HashMap<String, Entry>) {
    if let Some(p) = disk_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(map) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

pub fn state_of(installed: Option<&str>, latest: Option<&str>) -> VersionState {
    match (installed, latest) {
        (Some(i), Some(l)) => {
            if compare_versions(i, l) == std::cmp::Ordering::Less {
                VersionState::BehindLatest
            } else {
                VersionState::Current
            }
        }
        _ => VersionState::Unknown,
    }
}

fn client() -> Result<reqwest::Client, String> {
    crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .timeout(TIMEOUT)
        .user_agent("OmniGet-clitools")
        .build()
        .map_err(|e| e.to_string())
}

/// `@scope/name` → `@scope%2fname` (o registro aceita o `/` escapado).
pub fn npm_url(pkg: &str) -> String {
    format!(
        "https://registry.npmjs.org/{}/latest",
        pkg.replace('/', "%2f")
    )
}

async fn fetch_json(url: &str) -> Result<serde_json::Value, String> {
    let resp = client()?
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

async fn fetch(source: &LatestSource) -> Result<Option<String>, String> {
    match source {
        LatestSource::Npm(pkg) => {
            let v = fetch_json(&npm_url(pkg)).await?;
            Ok(v["version"].as_str().map(str::to_string))
        }
        LatestSource::Pypi(pkg) => {
            let v = fetch_json(&format!("https://pypi.org/pypi/{pkg}/json")).await?;
            Ok(v["info"]["version"].as_str().map(str::to_string))
        }
        LatestSource::Github(repo) => {
            let v = fetch_json(&format!(
                "https://api.github.com/repos/{repo}/releases/latest"
            ))
            .await?;
            Ok(v["tag_name"].as_str().and_then(parse_version))
        }
        LatestSource::None => Ok(None),
    }
}

fn source_key(source: &LatestSource) -> (String, String) {
    match source {
        LatestSource::Npm(k) => ("npm".into(), k.clone()),
        LatestSource::Pypi(k) => ("pypi".into(), k.clone()),
        LatestSource::Github(k) => ("github".into(), k.clone()),
        LatestSource::None => ("none".into(), String::new()),
    }
}

/// Mais nova da fonte, respeitando o cache (a não ser com `force`).
pub async fn latest_version(
    source: &LatestSource,
    force: bool,
) -> (Option<String>, u64, Option<String>) {
    let (kind, key) = source_key(source);
    if kind == "none" {
        return (None, now(), None);
    }
    let ck = format!("{kind}:{key}");
    if !force {
        if let Ok(map) = cache().lock() {
            if let Some(e) = map.get(&ck) {
                if now().saturating_sub(e.at) < TTL.as_secs() {
                    return (e.version.clone(), e.at, None);
                }
            }
        }
    }
    match fetch(source).await {
        Ok(version) => {
            let at = now();
            if let Ok(mut map) = cache().lock() {
                map.insert(
                    ck,
                    Entry {
                        version: version.clone(),
                        at,
                    },
                );
                save_disk(&map);
            }
            (version, at, None)
        }
        Err(e) => {
            // Sem rede: devolve o último valor conhecido, mesmo velho.
            let stale = cache().lock().ok().and_then(|m| m.get(&ck).cloned());
            match stale {
                Some(s) => (s.version, s.at, Some(e)),
                None => (None, now(), Some(e)),
            }
        }
    }
}

/// Versão estável que o brew oferece (`brew info --json=v2`).
pub async fn brew_latest(name: &str, cask: bool) -> Option<String> {
    let brew = super::exec::which("brew")?;
    let mut args = vec!["info".to_string(), "--json=v2".to_string()];
    if cask {
        args.push("--cask".into());
    }
    args.push(name.to_string());
    let out = super::exec::run_capture(&brew, &args, &Default::default(), Duration::from_secs(15))
        .await?;
    if !out.ok() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&out.stdout).ok()?;
    if cask {
        v["casks"][0]["version"]
            .as_str()
            .map(|s| s.split(',').next().unwrap_or(s).to_string())
    } else {
        v["formulae"][0]["versions"]["stable"]
            .as_str()
            .map(str::to_string)
    }
}

/// Estado de versão de uma ferramenta. `installed`/`brew` vêm da detecção.
pub async fn for_tool(
    tool: &CliTool,
    installed: Option<String>,
    brew_owner: Option<(String, bool)>,
    force: bool,
) -> Latest {
    if let Some((name, cask)) = brew_owner {
        // Dono provado é o brew: o que importa é o que o brew consegue entregar.
        let latest = brew_latest(&name, cask).await;
        return Latest {
            id: tool.id.clone(),
            source: "brew".into(),
            key: name,
            state: state_of(installed.as_deref(), latest.as_deref()),
            latest,
            installed,
            checked_at: now(),
            error: None,
        };
    }
    let source = tool.latest_source();
    let (kind, key) = source_key(&source);
    let (latest, at, error) = latest_version(&source, force).await;
    Latest {
        id: tool.id.clone(),
        source: kind,
        key,
        state: state_of(installed.as_deref(), latest.as_deref()),
        latest,
        installed,
        checked_at: at,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estado_por_comparacao() {
        assert_eq!(
            state_of(Some("1.0.0"), Some("1.0.1")),
            VersionState::BehindLatest
        );
        assert_eq!(
            state_of(Some("1.0.1"), Some("1.0.1")),
            VersionState::Current
        );
        assert_eq!(
            state_of(Some("2.0.0"), Some("1.9.0")),
            VersionState::Current
        );
        assert_eq!(state_of(None, Some("1.0.0")), VersionState::Unknown);
        assert_eq!(state_of(Some("1.0.0"), None), VersionState::Unknown);
    }

    #[test]
    fn url_do_npm_escapa_o_escopo() {
        assert_eq!(
            npm_url("@openai/codex"),
            "https://registry.npmjs.org/@openai%2fcodex/latest"
        );
    }
}
