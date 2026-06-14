use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use omniget_plugin_sdk::{InstalledPlugin, OmnigetPlugin, PluginHost, PluginManifest, ABI_VERSION};
use serde::Serialize;
use tracing;

pub struct LoadedPlugin {
    _lib: Option<libloading::Library>,
    pub plugin: Box<dyn OmnigetPlugin>,
    pub manifest: PluginManifest,
}

unsafe impl Send for LoadedPlugin {}
unsafe impl Sync for LoadedPlugin {}

#[derive(Debug, Clone, Serialize)]
pub struct PluginLoadError {
    pub message: String,
    pub kind: String,
    pub plugin_abi: Option<u32>,
    pub expected_abi: Option<u32>,
}

impl PluginLoadError {
    fn simple(kind: &str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: kind.to_string(),
            plugin_abi: None,
            expected_abi: None,
        }
    }
}

pub struct PluginManager {
    plugins_dir: PathBuf,
    loaded: HashMap<String, LoadedPlugin>,
    installed: Vec<InstalledPlugin>,
    load_errors: HashMap<String, PluginLoadError>,
    user_removed: HashSet<String>,
}

impl PluginManager {
    pub fn new(plugins_dir: PathBuf) -> Self {
        let installed = load_installed_list(&plugins_dir);
        let user_removed = load_removed_list(&plugins_dir);
        Self {
            plugins_dir,
            loaded: HashMap::new(),
            installed,
            load_errors: HashMap::new(),
            user_removed,
        }
    }

    pub fn load_all(&mut self, host: Arc<dyn PluginHost>) {
        tracing::info!(
            "[plugins] load_all: {} installed, {} enabled",
            self.installed.len(),
            self.installed.iter().filter(|p| p.enabled).count()
        );
        let enabled: Vec<_> = self
            .installed
            .iter()
            .filter(|p| p.enabled)
            .cloned()
            .collect();

        for entry in &enabled {
            let plugin_dir = self.plugins_dir.join(&entry.id);
            match load_single_plugin(&plugin_dir, host.clone()) {
                Ok(loaded) => {
                    tracing::info!(
                        "Loaded plugin: {} v{}",
                        loaded.manifest.id,
                        loaded.manifest.version
                    );
                    self.loaded.insert(entry.id.clone(), loaded);
                    self.load_errors.remove(&entry.id);
                }
                Err(e) => {
                    tracing::warn!("Plugin {} not loaded ({}): {}", entry.id, e.kind, e.message);
                    self.load_errors.insert(entry.id.clone(), e);
                }
            }
        }
    }

    pub fn is_loaded(&self, id: &str) -> bool {
        self.loaded.contains_key(id)
    }

    pub fn load_one(
        &mut self,
        id: &str,
        host: Arc<dyn PluginHost>,
    ) -> Result<(), PluginLoadError> {
        let plugin_dir = self.plugins_dir.join(id);
        match load_single_plugin(&plugin_dir, host) {
            Ok(loaded) => {
                tracing::info!(
                    "Loaded plugin at runtime: {} v{}",
                    loaded.manifest.id,
                    loaded.manifest.version
                );
                self.loaded.insert(id.to_string(), loaded);
                self.load_errors.remove(id);
                Ok(())
            }
            Err(e) => {
                tracing::warn!("Plugin {} not loaded ({}): {}", id, e.kind, e.message);
                self.load_errors.insert(id.to_string(), e.clone());
                Err(e)
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&LoadedPlugin> {
        self.loaded.get(id)
    }

    pub fn load_error(&self, id: &str) -> Option<&PluginLoadError> {
        self.load_errors.get(id)
    }

    pub fn loaded_plugins(&self) -> Vec<&LoadedPlugin> {
        self.loaded.values().collect()
    }

    pub fn installed_plugins(&self) -> &[InstalledPlugin] {
        tracing::debug!(
            "[plugins] installed_plugins() called, count={}, ids={:?}",
            self.installed.len(),
            self.installed.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
        &self.installed
    }

    pub fn loaded_manifests(&self) -> Vec<&PluginManifest> {
        self.loaded.values().map(|p| &p.manifest).collect()
    }

    pub async fn handle_command(
        &self,
        plugin_id: &str,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let loaded = self
            .loaded
            .get(plugin_id)
            .ok_or_else(|| format!("Plugin '{}' not loaded", plugin_id))?;

        loaded
            .plugin
            .handle_command(command.to_string(), args)
            .await
    }

    pub fn save_installed(&self) -> anyhow::Result<()> {
        save_installed_list(&self.plugins_dir, &self.installed)
    }

    pub fn register_installed(&mut self, entry: InstalledPlugin) -> anyhow::Result<()> {
        if self.user_removed.remove(&entry.id) {
            let _ = save_removed_list(&self.plugins_dir, &self.user_removed);
        }
        self.installed.retain(|p| p.id != entry.id);
        self.installed.push(entry);
        self.save_installed()
    }

    pub fn is_user_removed(&self, id: &str) -> bool {
        self.user_removed.contains(id)
    }

    pub fn unregister(&mut self, plugin_id: &str) -> anyhow::Result<()> {
        if let Some(mut loaded) = self.loaded.remove(plugin_id) {
            loaded.plugin.shutdown();
            let _leaked_lib = loaded._lib.take();
            std::mem::forget(_leaked_lib);
        }
        self.load_errors.remove(plugin_id);
        self.installed.retain(|p| p.id != plugin_id);
        self.save_installed()?;

        self.user_removed.insert(plugin_id.to_string());
        save_removed_list(&self.plugins_dir, &self.user_removed)?;

        let plugin_dir = self.plugins_dir.join(plugin_id);
        if plugin_dir.exists() {
            fs::remove_dir_all(&plugin_dir)?;
        }
        Ok(())
    }

    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) -> anyhow::Result<()> {
        if let Some(entry) = self.installed.iter_mut().find(|p| p.id == plugin_id) {
            entry.enabled = enabled;
        }
        self.save_installed()
    }

    pub fn plugins_dir(&self) -> &Path {
        &self.plugins_dir
    }
}

fn load_single_plugin(
    plugin_dir: &Path,
    host: Arc<dyn PluginHost>,
) -> Result<LoadedPlugin, PluginLoadError> {
    let manifest_path = plugin_dir.join("plugin.json");
    let manifest_str = fs::read_to_string(&manifest_path).map_err(|e| {
        PluginLoadError::simple("manifest_read", format!("Cannot read plugin.json: {e}"))
    })?;
    let manifest: PluginManifest = serde_json::from_str(&manifest_str).map_err(|e| {
        PluginLoadError::simple("manifest_parse", format!("Invalid plugin.json: {e}"))
    })?;

    let lib_path =
        find_native_lib(plugin_dir, manifest.rust_crate.as_deref()).ok_or_else(|| {
            PluginLoadError::simple(
                "no_native_lib",
                format!("No native library found in {}", plugin_dir.display()),
            )
        })?;

    let lib = unsafe { libloading::Library::new(&lib_path) }.map_err(|e| {
        PluginLoadError::simple(
            "library_load",
            format!("Failed to load {}: {e}", lib_path.display()),
        )
    })?;

    let abi_fn: libloading::Symbol<extern "C" fn() -> u32> =
        unsafe { lib.get(b"omniget_plugin_abi_version") }.map_err(|_| {
            PluginLoadError {
                message: format!(
                    "Missing omniget_plugin_abi_version symbol (plugin built against an older SDK; core expects ABI v{})",
                    ABI_VERSION
                ),
                kind: "missing_abi_symbol".to_string(),
                plugin_abi: None,
                expected_abi: Some(ABI_VERSION),
            }
        })?;

    let plugin_abi = abi_fn();
    if plugin_abi != ABI_VERSION {
        return Err(PluginLoadError {
            message: format!(
                "ABI mismatch: plugin has v{}, core expects v{}",
                plugin_abi, ABI_VERSION
            ),
            kind: "abi_mismatch".to_string(),
            plugin_abi: Some(plugin_abi),
            expected_abi: Some(ABI_VERSION),
        });
    }

    let init_fn: libloading::Symbol<extern "C" fn() -> *mut dyn OmnigetPlugin> =
        unsafe { lib.get(b"omniget_plugin_init") }.map_err(|_| {
            PluginLoadError::simple("missing_init_symbol", "Missing omniget_plugin_init symbol")
        })?;

    let mut plugin = unsafe { Box::from_raw(init_fn()) };
    plugin
        .initialize(host)
        .map_err(|e| PluginLoadError::simple("initialize", format!("Plugin init failed: {e}")))?;

    Ok(LoadedPlugin {
        _lib: Some(lib),
        plugin,
        manifest,
    })
}

fn find_native_lib(dir: &Path, rust_crate: Option<&str>) -> Option<PathBuf> {
    let extensions = if cfg!(target_os = "windows") {
        &["dll"][..]
    } else if cfg!(target_os = "macos") {
        &["dylib"][..]
    } else {
        &["so"][..]
    };

    if let Some(crate_name) = rust_crate {
        for ext in extensions {
            let candidate = dir.join(format!("{}.{}", crate_name, ext));
            if candidate.is_file() {
                return Some(candidate);
            }
            if !cfg!(target_os = "windows") {
                let unix_candidate = dir.join(format!("lib{}.{}", crate_name, ext));
                if unix_candidate.is_file() {
                    return Some(unix_candidate);
                }
            }
        }
    }

    for entry in fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if let Some(ext) = path.extension() {
            if extensions.contains(&ext.to_str().unwrap_or("")) {
                return Some(path);
            }
        }
    }
    None
}

fn load_installed_list(plugins_dir: &Path) -> Vec<InstalledPlugin> {
    let path = plugins_dir.join("installed.json");
    tracing::info!("[plugins] reading installed.json from: {}", path.display());
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("[plugins] cannot read installed.json: {e}");
            return Vec::new();
        }
    };
    let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);
    tracing::debug!(
        "[plugins] installed.json raw ({} bytes): {}",
        content.len(),
        &content[..content.len().min(200)]
    );

    #[derive(serde::Deserialize)]
    struct InstalledFile {
        plugins: Vec<InstalledPlugin>,
    }

    match serde_json::from_str::<InstalledFile>(content) {
        Ok(f) => {
            tracing::info!(
                "[plugins] parsed {} installed plugins: {:?}",
                f.plugins.len(),
                f.plugins.iter().map(|p| &p.id).collect::<Vec<_>>()
            );
            f.plugins
        }
        Err(e) => {
            tracing::error!("[plugins] FAILED to parse installed.json: {e}");
            tracing::error!(
                "[plugins] content was: {}",
                &content[..content.len().min(500)]
            );
            Vec::new()
        }
    }
}

fn save_installed_list(plugins_dir: &Path, plugins: &[InstalledPlugin]) -> anyhow::Result<()> {
    fs::create_dir_all(plugins_dir)?;
    let path = plugins_dir.join("installed.json");

    #[derive(serde::Serialize)]
    struct InstalledFile<'a> {
        plugins: &'a [InstalledPlugin],
    }

    let content = serde_json::to_string_pretty(&InstalledFile { plugins })?;
    fs::write(&path, content)?;
    Ok(())
}

fn load_removed_list(plugins_dir: &Path) -> HashSet<String> {
    let path = plugins_dir.join("removed-plugins.json");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    #[derive(serde::Deserialize)]
    struct RemovedFile {
        #[serde(default)]
        removed: Vec<String>,
    }
    match serde_json::from_str::<RemovedFile>(&content) {
        Ok(f) => f.removed.into_iter().collect(),
        Err(e) => {
            tracing::warn!("[plugins] cannot parse removed-plugins.json: {e}");
            HashSet::new()
        }
    }
}

fn save_removed_list(plugins_dir: &Path, removed: &HashSet<String>) -> anyhow::Result<()> {
    fs::create_dir_all(plugins_dir)?;
    let path = plugins_dir.join("removed-plugins.json");
    #[derive(serde::Serialize)]
    struct RemovedFile<'a> {
        removed: Vec<&'a String>,
    }
    let mut list: Vec<&String> = removed.iter().collect();
    list.sort();
    let content = serde_json::to_string_pretty(&RemovedFile { removed: list })?;
    fs::write(&path, content)?;
    Ok(())
}
