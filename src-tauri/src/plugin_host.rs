use std::path::PathBuf;

use omniget_plugin_sdk::{PluginHost, ProxyConfig};
use tauri::{AppHandle, Emitter, Runtime};

pub struct PluginHostImpl<R: Runtime> {
    app: AppHandle<R>,
    plugins_dir: PathBuf,
}

impl<R: Runtime> PluginHostImpl<R> {
    pub fn new(app: AppHandle<R>, plugins_dir: PathBuf) -> Self {
        Self { app, plugins_dir }
    }
}

impl<R: Runtime + 'static> PluginHost for PluginHostImpl<R> {
    fn emit_event(&self, name: &str, payload: serde_json::Value) -> anyhow::Result<()> {
        self.app
            .emit(name, payload)
            .map_err(|e| anyhow::anyhow!("Failed to emit event '{}': {}", name, e))
    }

    fn show_toast(&self, toast_type: &str, message: &str) -> anyhow::Result<()> {
        self.app
            .emit(
                "plugin-toast",
                serde_json::json!({
                    "type": toast_type,
                    "message": message,
                }),
            )
            .map_err(|e| anyhow::anyhow!("Failed to show toast: {}", e))
    }

    fn plugin_data_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir.join(plugin_id).join("data")
    }

    fn plugin_frontend_dir(&self, plugin_id: &str) -> PathBuf {
        self.plugins_dir.join(plugin_id).join("frontend")
    }

    fn get_settings(&self, plugin_id: &str) -> serde_json::Value {
        let settings_path = self
            .plugins_dir
            .join(plugin_id)
            .join("data")
            .join("settings.json");
        // Mesmo par lê-default-e-reescreve do registro de cookies: sem a
        // quarentena, um settings.json de plugin corrompido é apagado pelo
        // primeiro `save_settings`.
        crate::core::state_file::load_or_quarantine(
            &settings_path,
            &format!("settings do plugin {}", plugin_id),
        )
        .unwrap_or(serde_json::Value::Object(Default::default()))
    }

    fn save_settings(&self, plugin_id: &str, settings: serde_json::Value) -> anyhow::Result<()> {
        let data_dir = self.plugins_dir.join(plugin_id).join("data");
        std::fs::create_dir_all(&data_dir)?;
        let settings_path = data_dir.join("settings.json");
        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
        Ok(())
    }

    fn proxy_config(&self) -> Option<ProxyConfig> {
        let proxy = crate::core::http_client::get_proxy_snapshot();
        if !proxy.enabled || proxy.host.is_empty() {
            return None;
        }
        Some(ProxyConfig {
            proxy_type: proxy.proxy_type,
            host: proxy.host,
            port: proxy.port,
            username: if proxy.username.is_empty() {
                None
            } else {
                Some(proxy.username)
            },
            password: if proxy.password.is_empty() {
                None
            } else {
                Some(proxy.password)
            },
        })
    }

    fn tool_path(&self, _tool: &str) -> Option<PathBuf> {
        let data_dir = crate::core::paths::app_data_dir()?;
        let managed_dir = data_dir.join("bin");

        #[cfg(target_os = "windows")]
        let bin_name = format!("{}.exe", _tool);
        #[cfg(not(target_os = "windows"))]
        let bin_name = _tool.to_string();

        let managed_path = managed_dir.join(&bin_name);
        if managed_path.exists() {
            return Some(managed_path);
        }

        which::which(&bin_name).ok()
    }

    fn default_output_dir(&self) -> PathBuf {
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn external_data_cache(&self, plugin_id: &str, namespace: &str) -> anyhow::Result<PathBuf> {
        if plugin_id.is_empty() {
            anyhow::bail!("external_data_cache: plugin_id must not be empty");
        }
        if namespace.is_empty() {
            anyhow::bail!("external_data_cache: namespace must not be empty");
        }
        if plugin_id.contains(['/', '\\', ':', '\0']) || namespace.contains(['/', '\\', ':', '\0'])
        {
            anyhow::bail!(
                "external_data_cache: plugin_id/namespace must not contain path separators or null bytes"
            );
        }

        // portable installs keep every file next to the app, so the cache
        // lives under the app data dir instead of the OS cache dir
        let base = if std::env::var("OMNIGET_PORTABLE").is_ok() {
            omniget_core::core::paths::app_data_dir()
                .ok_or_else(|| anyhow::anyhow!("external_data_cache: app data dir unavailable"))?
                .join("cache")
        } else {
            dirs::cache_dir()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "external_data_cache: OS cache dir unavailable on this platform"
                    )
                })?
                .join("wtf.tonho.omniget")
        };
        let dir = base.join("external-cache").join(plugin_id).join(namespace);
        std::fs::create_dir_all(&dir).map_err(|e| {
            anyhow::anyhow!(
                "external_data_cache: failed to create {}: {}",
                dir.display(),
                e
            )
        })?;
        Ok(dir)
    }

    fn get_cookie_file(&self, domain: &str, account: Option<&str>) -> Option<PathBuf> {
        crate::cookies::account_path_for_consumer(domain, account)
    }

    fn cookie_status(&self, domain: &str) -> omniget_plugin_sdk::CookieStatus {
        let registry = crate::cookies::load_registry();
        let root = crate::cookies::root_domain_of(domain);
        let bucket = match registry.buckets.get(&root) {
            Some(b) => b,
            None => return omniget_plugin_sdk::CookieStatus::Missing,
        };
        let primary = match bucket.accounts.iter().find(|a| a.slug == "_default") {
            Some(a) => a,
            None => return omniget_plugin_sdk::CookieStatus::Missing,
        };
        let path = match crate::cookies::account_path_for_consumer(&root, None) {
            Some(p) => p,
            None => return omniget_plugin_sdk::CookieStatus::Missing,
        };
        omniget_plugin_sdk::CookieStatus::Available {
            path,
            last_modified_secs: primary.captured_at_ms / 1000,
            cookie_count: primary.cookie_count,
        }
    }

    fn emit_download_log(&self, download_id: u64, line: &str) {
        omniget_core::core::log_hook::emit_log(download_id, line);
    }
}
