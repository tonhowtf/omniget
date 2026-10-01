use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::client::McpClient;
use super::error::McpError;
use super::types::{default_lookup, valid_id, McpServerConfig, SecretLookup, SECRET_NS};
use crate::core::secrets::SecretStore;

pub const CONFIG_FILE: &str = "mcp-servers.json";
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const REAP_INTERVAL: Duration = Duration::from_secs(30);

pub struct McpRegistry {
    path: Option<PathBuf>,
    configs: RwLock<Vec<McpServerConfig>>,
    clients: Mutex<HashMap<String, Arc<McpClient>>>,
    reaper: StdMutex<Option<JoinHandle<()>>>,
    idle_timeout: Duration,
    reap_interval: Duration,
    secrets: Option<Arc<dyn SecretStore>>,
}

impl McpRegistry {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let configs = read_file(&path);
        Self {
            path: Some(path),
            configs: RwLock::new(configs),
            clients: Mutex::new(HashMap::new()),
            reaper: StdMutex::new(None),
            idle_timeout: IDLE_TIMEOUT,
            reap_interval: REAP_INTERVAL,
            secrets: None,
        }
    }

    pub fn default_store() -> Option<Self> {
        default_path().map(Self::at)
    }

    pub fn lookup(&self) -> SecretLookup {
        match self.secrets.clone() {
            Some(store) => Arc::new(move |id: &str| store.get(SECRET_NS, id)),
            None => default_lookup(),
        }
    }

    fn secret_set(&self, id: &str, value: &str) -> Result<(), String> {
        match &self.secrets {
            Some(store) => store.set(SECRET_NS, id, value),
            None => crate::core::secrets::set(SECRET_NS, id, value),
        }
    }

    fn secret_delete(&self, id: &str) {
        let _ = match &self.secrets {
            Some(store) => store.delete(SECRET_NS, id),
            None => crate::core::secrets::delete(SECRET_NS, id),
        };
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn list(&self) -> Vec<McpServerConfig> {
        self.configs.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<McpServerConfig> {
        self.list().into_iter().find(|c| c.id == id)
    }

    pub async fn upsert(&self, mut cfg: McpServerConfig) -> Result<McpServerConfig, McpError> {
        if !valid_id(&cfg.id) {
            return Err(McpError::spawn(format!(
                "invalid server id {:?}: use lowercase letters, digits, - and _",
                cfg.id
            )));
        }
        for (id, value) in cfg.extract_secrets() {
            self.secret_set(&id, &value).map_err(|e| {
                McpError::spawn(format!("could not store the secret {:?}: {}", id, e))
            })?;
        }
        cfg.validate()?;
        let (changed, previous) = {
            let mut guard = self
                .configs
                .write()
                .map_err(|_| McpError::spawn("the server list is poisoned"))?;
            match guard.iter().position(|c| c.id == cfg.id) {
                Some(i) => {
                    let differs = guard[i] != cfg;
                    let previous = std::mem::replace(&mut guard[i], cfg.clone());
                    (differs, Some(previous))
                }
                None => {
                    guard.push(cfg.clone());
                    (false, None)
                }
            }
        };
        if changed {
            self.disconnect(&cfg.id).await;
        }
        if let Err(error) = self.save() {
            self.restore(&cfg.id, previous);
            return Err(error);
        }
        Ok(cfg)
    }

    fn restore(&self, id: &str, previous: Option<McpServerConfig>) {
        let Ok(mut guard) = self.configs.write() else {
            return;
        };
        match (guard.iter().position(|c| c.id == id), previous) {
            (Some(i), Some(previous)) => guard[i] = previous,
            (Some(i), None) => {
                guard.remove(i);
            }
            (None, Some(previous)) => guard.push(previous),
            (None, None) => {}
        }
    }

    pub async fn remove(&self, id: &str) -> Result<bool, McpError> {
        if let Some(cfg) = self.get(id) {
            for secret in cfg.secret_ids() {
                if secret.starts_with(&format!("{}-", id)) {
                    self.secret_delete(&secret);
                }
            }
        }
        let removed = {
            let mut guard = self
                .configs
                .write()
                .map_err(|_| McpError::spawn("the server list is poisoned"))?;
            let before = guard.len();
            guard.retain(|c| c.id != id);
            guard.len() != before
        };
        self.disconnect(id).await;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub async fn client_for(self: &Arc<Self>, id: &str) -> Result<Arc<McpClient>, McpError> {
        if let Some(existing) = self.clients.lock().await.get(id).cloned() {
            return Ok(existing);
        }
        let cfg = self
            .get(id)
            .ok_or_else(|| McpError::spawn(format!("no MCP server configured as {:?}", id)))?;
        let client = Arc::new(McpClient::connect_with(&cfg, &self.lookup()).await?);
        let mut map = self.clients.lock().await;
        if let Some(existing) = map.get(id).cloned() {
            drop(map);
            client.close().await;
            return Ok(existing);
        }
        map.insert(id.to_string(), client.clone());
        drop(map);
        self.ensure_reaper();
        Ok(client)
    }

    pub async fn connected(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.clients.lock().await.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub async fn disconnect(&self, id: &str) -> bool {
        let client = self.clients.lock().await.remove(id);
        match client {
            Some(c) => {
                c.close().await;
                true
            }
            None => false,
        }
    }

    pub async fn reap_idle(&self) -> Vec<String> {
        let stale: Vec<(String, Arc<McpClient>)> = {
            let map = self.clients.lock().await;
            map.iter()
                .filter(|(_, c)| c.idle() >= self.idle_timeout)
                .map(|(k, c)| (k.clone(), c.clone()))
                .collect()
        };
        let mut closed = Vec::new();
        for (id, client) in stale {
            self.clients.lock().await.remove(&id);
            client.close().await;
            closed.push(id);
        }
        closed.sort();
        closed
    }

    fn ensure_reaper(self: &Arc<Self>) {
        let Ok(mut guard) = self.reaper.lock() else {
            return;
        };
        if guard.as_ref().map(|h| !h.is_finished()).unwrap_or(false) {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        let interval = self.reap_interval;
        *guard = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(registry) = weak.upgrade() else {
                    return;
                };
                registry.reap_idle().await;
                if registry.clients.lock().await.is_empty() {
                    return;
                }
            }
        }));
    }

    fn save(&self) -> Result<(), McpError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let list = self.list();
        let text = serde_json::to_string_pretty(&list)
            .map_err(|e| McpError::spawn(format!("could not encode the server list: {}", e)))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| {
                McpError::spawn(format!("could not create {}: {}", dir.display(), e))
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{}\n", text))
            .map_err(|e| McpError::spawn(format!("could not write {}: {}", tmp.display(), e)))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| McpError::spawn(format!("could not replace {}: {}", path.display(), e)))
    }
}

impl Drop for McpRegistry {
    fn drop(&mut self) {
        if let Ok(mut g) = self.reaper.lock() {
            if let Some(h) = g.take() {
                h.abort();
            }
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::core::llm::roster_store::llm_dir().map(|d| d.join(CONFIG_FILE))
}

fn read_file(path: &Path) -> Vec<McpServerConfig> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<McpServerConfig>>(&t).ok())
        .unwrap_or_default()
}
