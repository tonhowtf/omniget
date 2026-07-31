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

    pub fn load_one(&mut self, id: &str, host: Arc<dyn PluginHost>) -> Result<(), PluginLoadError> {
        let plugin_dir = self.plugin_dir(id).map_err(|e| PluginLoadError {
            message: e.to_string(),
            kind: "invalid_id".to_string(),
            plugin_abi: None,
            expected_abi: None,
        })?;
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

        let plugin_dir = self.plugin_dir(plugin_id)?;
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

    /// Diretorio de um plugin, com o id validado.
    ///
    /// Todo caminho derivado de `plugin_id` passa por aqui. O id chega de fora
    /// — registro remoto -> `installed.json` -> frontend -> comando Tauri — e
    /// nenhum ponto dessa cadeia o sanitizava, entao um id com `..` escapava do
    /// diretorio de plugins e o `remove_dir_all` do `unregister` apagava
    /// caminho arbitrario.
    pub fn plugin_dir(&self, plugin_id: &str) -> anyhow::Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        Ok(self.plugins_dir.join(plugin_id))
    }
}

/// Aceita apenas `[A-Za-z0-9._-]`, recusando separador de caminho, byte nulo,
/// componente relativo e id vazio.
///
/// Allowlist e nao blocklist de proposito: o conjunto de ids reais e pequeno e
/// conhecido (`courses`, `study`, `telegram`, `convert`, `misc`), e uma
/// blocklist erra por omissao a cada codificacao nova.
pub fn validate_plugin_id(plugin_id: &str) -> anyhow::Result<()> {
    if plugin_id.is_empty() {
        anyhow::bail!("plugin id must not be empty");
    }
    if plugin_id == "." || plugin_id == ".." || plugin_id.starts_with('.') {
        anyhow::bail!("plugin id must not be a relative path component: {plugin_id:?}");
    }
    if let Some(bad) = plugin_id
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '_' | '-'))
    {
        anyhow::bail!("plugin id contains an illegal character {bad:?}: {plugin_id:?}");
    }
    if plugin_id.contains("..") {
        anyhow::bail!("plugin id must not contain '..': {plugin_id:?}");
    }
    Ok(())
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
        // libloading's Display is just "LoadLibraryExW failed"; the OS error
        // code that says WHY (126 missing dependency, 5 access denied, 193 bad
        // image) lives in the source chain and is what users need to report.
        let mut detail = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(s) = source {
            detail.push_str(": ");
            detail.push_str(&s.to_string());
            source = s.source();
        }
        PluginLoadError::simple(
            "library_load",
            format!("Failed to load {}: {detail}", lib_path.display()),
        )
    })?;

    let abi_fn: libloading::Symbol<extern "C" fn() -> u32> =
        unsafe { lib.get(b"omniget_plugin_abi_version") }.map_err(|_| {
            PluginLoadError {
                message: format!(
                    "This plugin was built for an older version of OmniGet (no ABI handshake; this version requires ABI v{}). Update the plugin: reinstall it from the Marketplace to get a compatible build.",
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
                "This plugin was built for an older version of OmniGet (plugin ABI v{}, this version requires v{}). Update the plugin: reinstall it from the Marketplace to get a compatible build.",
                plugin_abi, ABI_VERSION
            ),
            kind: "abi_mismatch".to_string(),
            plugin_abi: Some(plugin_abi),
            expected_abi: Some(ABI_VERSION),
        });
    }

    // ABI_VERSION alone cannot detect a plugin compiled by a different rustc.
    // Rust has no stable ABI — trait-object vtables, fat-pointer conventions
    // and std/serde type layouts can change between compiler releases — and
    // every non-C-ABI type crossing this boundary (Box<dyn OmnigetPlugin>,
    // Arc<dyn PluginHost>, String, serde_json::Value, boxed futures) is UB on
    // a mismatch. This is exactly what crashed v0.7.1 (host: rustc 1.97.0)
    // with registry plugins built by rustc 1.95/1.96: they passed the ABI v2
    // handshake, loaded, then jumped through corrupted pointers on the first
    // plugin command (SIGSEGV on a tokio worker). Since ABI v3 the
    // export_plugin! macro also exports a build-info symbol; require it and
    // require the plugin's rustc fingerprint to match ours exactly.
    let build_info_fn: libloading::Symbol<extern "C" fn() -> *const std::os::raw::c_char> =
        unsafe { lib.get(b"omniget_plugin_build_info") }.map_err(|_| PluginLoadError {
            message: format!(
                "This plugin was built for an older version of OmniGet (missing toolchain handshake; this version requires ABI v{}). Update the plugin: reinstall it from the Marketplace to get a compatible build.",
                ABI_VERSION
            ),
            kind: "abi_mismatch".to_string(),
            plugin_abi: Some(plugin_abi),
            expected_abi: Some(ABI_VERSION),
        })?;

    // Reading a thin `*const c_char` from an extern "C" fn is FFI-safe on any
    // rustc, unlike the Rust-ABI surface it guards.
    let plugin_build_info = {
        let raw = build_info_fn();
        if raw.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(raw) }
                .to_string_lossy()
                .into_owned()
        }
    };
    let host_info = omniget_plugin_sdk::build_info_str();
    let host_rustc = omniget_plugin_sdk::rustc_component(host_info);
    let plugin_rustc = omniget_plugin_sdk::rustc_component(&plugin_build_info);
    if plugin_rustc.is_none() || plugin_rustc != host_rustc {
        return Err(PluginLoadError {
            message: format!(
                "This plugin was built with a different Rust toolchain ('{}') than this version of OmniGet ('{}') and cannot be loaded safely. Update the plugin: reinstall it from the Marketplace to get a compatible build.",
                plugin_rustc.unwrap_or("unknown"),
                host_rustc.unwrap_or("unknown"),
            ),
            kind: "abi_mismatch".to_string(),
            plugin_abi: Some(plugin_abi),
            expected_abi: Some(ABI_VERSION),
        });
    }
    tracing::debug!(
        "[plugins] toolchain handshake ok: plugin '{}' / host '{}'",
        plugin_build_info,
        host_info
    );

    let init_fn: libloading::Symbol<extern "C" fn() -> *mut dyn OmnigetPlugin> =
        unsafe { lib.get(b"omniget_plugin_init") }.map_err(|_| {
            PluginLoadError::simple("missing_init_symbol", "Missing omniget_plugin_init symbol")
        })?;

    // A plugin's init entrypoint runs arbitrary foreign code; convert panics
    // into load errors instead of aborting the whole app.
    let plugin_ptr = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| init_fn())) {
        Ok(ptr) => ptr,
        Err(_) => {
            // The plugin may already have spawned threads before panicking;
            // leak the library instead of dropping it (dlclose would unmap
            // code those threads are still executing → SIGSEGV). Mirrors
            // the deliberate leak in `PluginManager::unregister`.
            std::mem::forget(lib);
            return Err(PluginLoadError::simple(
                "initialize",
                "Plugin panicked in omniget_plugin_init",
            ));
        }
    };
    if plugin_ptr.is_null() {
        std::mem::forget(lib);
        return Err(PluginLoadError::simple(
            "initialize",
            "omniget_plugin_init returned a null plugin",
        ));
    }
    let mut plugin = unsafe { Box::from_raw(plugin_ptr) };

    let init_result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| plugin.initialize(host)));
    match init_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            // `initialize` may have spawned threads before failing. Dropping
            // the `Library` here would dlclose and unmap the plugin's code
            // while those threads still run → SIGSEGV. Deliberately leak it
            // instead, mirroring `PluginManager::unregister`. The plugin box
            // is leaked too: its Drop impl lives in the (possibly half
            // initialized) plugin and isn't safe to run after a failed init.
            std::mem::forget(plugin);
            std::mem::forget(lib);
            return Err(PluginLoadError::simple(
                "initialize",
                format!("Plugin init failed: {e}"),
            ));
        }
        Err(_) => {
            // Same reasoning as above, but the plugin state is additionally
            // unknown after a panic — never run its Drop.
            std::mem::forget(plugin);
            std::mem::forget(lib);
            return Err(PluginLoadError::simple(
                "initialize",
                "Plugin panicked during initialize",
            ));
        }
    }

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

#[cfg(test)]
mod plugin_id_tests {
    use super::*;

    #[test]
    fn aceita_os_ids_reais() {
        for id in [
            "courses",
            "study",
            "telegram",
            "convert",
            "misc",
            "my-plugin_2",
            "wtf.tonho.x",
        ] {
            assert!(validate_plugin_id(id).is_ok(), "recusou id valido {id:?}");
        }
    }

    #[test]
    fn recusa_travessia_de_caminho() {
        // Criterio de aceite do B48: um id relativo falha explicitamente em vez
        // de virar um caminho fora do diretorio de plugins.
        for id in [
            "../../algo",
            "..",
            ".",
            "../etc",
            "a/../../b",
            ".oculto",
            "sub/dir",
            "sub\\dir",
            "com:dois",
            "nulo\0byte",
            "",
        ] {
            assert!(
                validate_plugin_id(id).is_err(),
                "aceitou id perigoso {id:?}"
            );
        }
    }

    #[test]
    fn plugin_dir_nunca_escapa_do_diretorio_base() {
        let base = std::path::PathBuf::from("/tmp/omniget-plugins");
        let manager = PluginManager::new(base.clone());

        let ok = manager.plugin_dir("courses").expect("id valido");
        assert_eq!(ok, base.join("courses"));
        assert!(ok.starts_with(&base));

        // Sem o guard, `join("../../etc")` normalizaria para fora de `base` e o
        // `remove_dir_all` do `unregister` apagaria caminho arbitrario.
        assert!(manager.plugin_dir("../../etc").is_err());
        assert!(manager.plugin_dir("..").is_err());
    }
}
