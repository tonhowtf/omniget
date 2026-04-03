use std::sync::Arc;

use omniget_plugin_sdk::{PluginManifest, RegistryEntry};
use serde::{Deserialize, Serialize};

use crate::plugin_loader::PluginManager;

#[derive(Debug, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub loaded: bool,
    pub icon: Option<String>,
    pub nav: Vec<PluginNavInfo>,
}

#[derive(Debug, Serialize)]
pub struct PluginNavInfo {
    pub route: String,
    pub label: std::collections::HashMap<String, String>,
    pub icon_svg: Option<String>,
    pub group: String,
    pub order: u32,
}

#[tauri::command]
pub fn list_plugins(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
) -> Result<Vec<PluginInfo>, String> {
    let manager = state.blocking_lock();
    let installed = manager.installed_plugins();

    let infos: Vec<PluginInfo> = installed
        .iter()
        .map(|entry| {
            let loaded = manager.get(&entry.id);
            match loaded {
                Some(lp) => PluginInfo {
                    id: entry.id.clone(),
                    name: lp.manifest.name.clone(),
                    version: lp.manifest.version.clone(),
                    description: lp.manifest.description.clone(),
                    author: lp.manifest.author.clone(),
                    enabled: entry.enabled,
                    loaded: true,
                    icon: lp.manifest.icon.clone(),
                    nav: lp
                        .manifest
                        .nav
                        .iter()
                        .map(|n| PluginNavInfo {
                            route: n.route.clone(),
                            label: n.label.clone(),
                            icon_svg: n.icon_svg.clone(),
                            group: match n.group {
                                omniget_plugin_sdk::NavGroup::Primary => "primary".into(),
                                omniget_plugin_sdk::NavGroup::Secondary => "secondary".into(),
                            },
                            order: n.order,
                        })
                        .collect(),
                },
                None => {
                    let manifest_path = manager.plugins_dir().join(&entry.id).join("plugin.json");
                    let manifest: Option<omniget_plugin_sdk::PluginManifest> = std::fs::read_to_string(&manifest_path)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok());
                    match manifest {
                        Some(m) => PluginInfo {
                            id: entry.id.clone(),
                            name: m.name,
                            version: m.version,
                            description: m.description,
                            author: m.author,
                            enabled: entry.enabled,
                            loaded: false,
                            icon: m.icon,
                            nav: m.nav.iter().map(|n| PluginNavInfo {
                                route: n.route.clone(),
                                label: n.label.clone(),
                                icon_svg: n.icon_svg.clone(),
                                group: match n.group {
                                    omniget_plugin_sdk::NavGroup::Primary => "primary".into(),
                                    omniget_plugin_sdk::NavGroup::Secondary => "secondary".into(),
                                },
                                order: n.order,
                            }).collect(),
                        },
                        None => PluginInfo {
                            id: entry.id.clone(),
                            name: entry.id.clone(),
                            version: entry.version.clone(),
                            description: String::new(),
                            author: String::new(),
                            enabled: entry.enabled,
                            loaded: false,
                            icon: None,
                            nav: Vec::new(),
                        },
                    }
                },
            }
        })
        .collect();

    Ok(infos)
}

#[tauri::command]
pub fn get_plugin_frontend_path(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
) -> Result<String, String> {
    let manager = state.blocking_lock();
    let frontend_dir = manager.plugins_dir().join(&plugin_id).join("frontend");
    if !frontend_dir.exists() {
        return Err(format!("Plugin '{}' has no frontend", plugin_id));
    }
    Ok(frontend_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub fn set_plugin_enabled(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
    enabled: bool,
) -> Result<(), String> {
    let mut manager = state.blocking_lock();
    manager
        .set_enabled(&plugin_id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn uninstall_plugin(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
) -> Result<(), String> {
    let mut manager = state.blocking_lock();
    manager.unregister(&plugin_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_loaded_plugin_manifests(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
) -> Result<Vec<PluginManifest>, String> {
    let manager = state.blocking_lock();
    Ok(manager.loaded_manifests().into_iter().cloned().collect())
}

#[tauri::command]
pub fn get_plugin_i18n(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
    locale: String,
) -> Result<serde_json::Value, String> {
    let manager = state.blocking_lock();
    let i18n_dir = manager.plugins_dir().join(&plugin_id).join("i18n");
    let locale_file = i18n_dir.join(format!("{}.json", locale));
    if !locale_file.exists() {
        let fallback = i18n_dir.join("en.json");
        if !fallback.exists() {
            return Ok(serde_json::json!({}));
        }
        let content = std::fs::read_to_string(&fallback).map_err(|e| e.to_string())?;
        return serde_json::from_str(&content).map_err(|e| e.to_string());
    }
    let content = std::fs::read_to_string(&locale_file).map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugin_command(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
    command: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let manager = state.lock().await;
    manager.handle_command(&plugin_id, &command, args).await
}

const REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/tonhowtf/omniget-plugins/main/plugins.json";

#[derive(Debug, Serialize)]
pub struct MarketplaceEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub repo: String,
    pub homepage: Option<String>,
    pub tags: Vec<String>,
    pub official: bool,
    pub capabilities: Vec<String>,
    pub installed: bool,
    pub installed_version: Option<String>,
}

#[tauri::command]
pub async fn fetch_marketplace_registry(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
) -> Result<Vec<MarketplaceEntry>, String> {
    let client = reqwest::Client::builder()
        .user_agent("OmniGet")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(REGISTRY_URL).send()
        .await
        .map_err(|e| format!("Failed to fetch registry: {}", e))?;

    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read registry: {}", e))?;

    #[derive(Deserialize)]
    struct RegistryFile {
        plugins: Vec<RegistryEntry>,
    }

    let registry: RegistryFile =
        serde_json::from_str(&body).map_err(|e| format!("Invalid registry: {}", e))?;

    let installed = {
        let manager = state.lock().await;
        manager.installed_plugins().to_vec()
    };

    let entries = registry
        .plugins
        .into_iter()
        .map(|entry| {
            let inst = installed.iter().find(|i| i.id == entry.id);
            MarketplaceEntry {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                author: entry.author,
                repo: entry.repo,
                homepage: entry.homepage,
                tags: entry.tags,
                official: entry.official,
                capabilities: entry.capabilities,
                installed: inst.is_some(),
                installed_version: inst.map(|i| i.version.clone()),
            }
        })
        .collect();

    Ok(entries)
}

fn platform_artifact_suffix() -> &'static str {
    if cfg!(target_os = "windows") && cfg!(target_arch = "x86_64") {
        "windows-x86_64"
    } else if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        "linux-x86_64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        "macos-aarch64"
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "x86_64") {
        "macos-x86_64"
    } else {
        "unknown"
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub async fn install_plugin_from_registry(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
    repo: String,
) -> Result<String, String> {
    let suffix = platform_artifact_suffix();
    if suffix == "unknown" {
        return Err("Unsupported platform".to_string());
    }

    let client = reqwest::Client::builder()
        .user_agent("OmniGet")
        .build()
        .map_err(|e| e.to_string())?;

    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let release: GitHubRelease = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch release: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Invalid release response: {}", e))?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains(suffix) && a.name.ends_with(".zip"))
        .ok_or_else(|| format!("No artifact found for {}", suffix))?;

    let zip_bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read download: {}", e))?;

    let manager = state.lock().await;
    let plugin_dir = manager.plugins_dir().join(&plugin_id);
    std::fs::create_dir_all(&plugin_dir).map_err(|e| e.to_string())?;

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("Invalid ZIP: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = plugin_dir.join(file.mangled_name());
        if file.is_dir() {
            std::fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if outpath.exists() {
                let old = outpath.with_extension("old");
                let _ = std::fs::remove_file(&old);
                let _ = std::fs::rename(&outpath, &old);
            }
            let mut outfile = std::fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }

    let version = release.tag_name.trim_start_matches('v').to_string();
    drop(manager);

    let mut manager = state.lock().await;
    manager
        .register_installed(omniget_plugin_sdk::InstalledPlugin {
            id: plugin_id.clone(),
            version: version.clone(),
            installed_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            enabled: true,
            repo: Some(repo),
            source_release: Some(release.tag_name),
        })
        .map_err(|e| e.to_string())?;

    Ok(version)
}

#[derive(Debug, Serialize)]
pub struct PluginUpdateInfo {
    pub id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub repo: String,
    pub has_update: bool,
}

#[tauri::command]
pub async fn check_plugin_updates(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
) -> Result<Vec<PluginUpdateInfo>, String> {
    let manager = state.lock().await;
    let installed = manager.installed_plugins().to_vec();
    drop(manager);

    let client = reqwest::Client::builder()
        .user_agent("OmniGet")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let mut updates = Vec::new();

    for plugin in &installed {
        let repo = match &plugin.repo {
            Some(r) => r.clone(),
            None => continue,
        };

        let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
        let release: GitHubRelease = match client.get(&api_url).send().await {
            Ok(resp) => match resp.json().await {
                Ok(r) => r,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        let latest = release.tag_name.trim_start_matches('v').to_string();
        let has_update = latest != plugin.version;

        updates.push(PluginUpdateInfo {
            id: plugin.id.clone(),
            installed_version: plugin.version.clone(),
            latest_version: latest,
            repo,
            has_update,
        });
    }

    Ok(updates)
}

#[tauri::command]
pub async fn update_plugin(
    state: tauri::State<'_, Arc<tokio::sync::Mutex<PluginManager>>>,
    plugin_id: String,
    repo: String,
) -> Result<String, String> {
    install_plugin_from_registry(state, plugin_id, repo).await
}
