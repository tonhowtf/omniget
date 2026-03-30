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
        std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
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
}
