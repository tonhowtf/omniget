//! The configured MCP servers and the live clients. Owned by f3-mcp-core.
//!
//! Config lives in `<app_data>/llm/mcp-servers.json` and holds no secret, and
//! that is enforced rather than hoped for: [`McpRegistry::upsert`] calls
//! `McpServerConfig::extract_secrets`, which moves any literal value of a
//! secret-looking header or environment variable into the secret store and
//! leaves a `secret:<id>` reference in the file. The reference is resolved —
//! for HTTP headers **and** for the environment of a stdio server — at connect
//! time (see `types::resolve_headers` / `types::resolve_env`).
//!
//! Budget (prompt §6): a process is born on the first `client_for` — that is,
//! on the first `tools()`/`call()` of an agent that was granted the server —
//! and dies after five idle minutes. The reaper is a `tokio::time` loop that
//! exists **only while a client is alive**: it is started by the first
//! connection and it returns as soon as the last client is gone, so a closed
//! `/llm` section leaves nothing ticking.

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
/// Prompt §6: "morre após 5 min ocioso".
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const REAP_INTERVAL: Duration = Duration::from_secs(30);

pub struct McpRegistry {
    path: Option<PathBuf>,
    configs: RwLock<Vec<McpServerConfig>>,
    clients: Mutex<HashMap<String, Arc<McpClient>>>,
    reaper: StdMutex<Option<JoinHandle<()>>>,
    idle_timeout: Duration,
    reap_interval: Duration,
    /// Where extracted secrets are written and read. `None` = the process-wide
    /// store (`core::secrets`), which is what the app uses; a test hands in its
    /// own file store instead of touching a keychain or a global env var.
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

    /// `<app_data>/llm/mcp-servers.json`, or `None` when there is no data dir.
    pub fn default_store() -> Option<Self> {
        default_path().map(Self::at)
    }

    /// No file behind it: for tests and for a headless run.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            configs: RwLock::new(Vec::new()),
            clients: Mutex::new(HashMap::new()),
            reaper: StdMutex::new(None),
            idle_timeout: IDLE_TIMEOUT,
            reap_interval: REAP_INTERVAL,
            secrets: None,
        }
    }

    /// Uses `store` for the extracted secrets instead of the process-wide one.
    pub fn with_secret_store(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secrets = Some(store);
        self
    }

    /// How `secret:<id>` is read when a client connects.
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

    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle_timeout = idle;
        self
    }

    pub fn with_reap_interval(mut self, interval: Duration) -> Self {
        self.reap_interval = interval;
        self
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

    /// Adds or replaces a server. A replaced server's live client is closed:
    /// the next call starts a process with the new command or headers.
    /// Any literal secret in the config is moved into the secret store first,
    /// so what reaches the disk is only a `secret:<id>` reference.
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
        // Belt and braces: never write a config that still holds a literal.
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
            // The file still holds the old list, so the memory has to as well:
            // otherwise the next list or connect would use a server the disk
            // never accepted, with references the caller may be rolling back.
            self.restore(&cfg.id, previous);
            return Err(error);
        }
        Ok(cfg)
    }

    /// Puts one entry back the way it was before a failed [`Self::upsert`].
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

    /// Removes a server, stops its process and forgets its secrets. `false`
    /// when there was none.
    pub async fn remove(&self, id: &str) -> Result<bool, McpError> {
        if let Some(cfg) = self.get(id) {
            for secret in cfg.secret_ids() {
                // Only ours: a reference the user pointed at a shared secret
                // (not named after this server) is left alone.
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

    /// The live client for `id`, starting it if needed. This is the only place
    /// a process is born.
    pub async fn client_for(self: &Arc<Self>, id: &str) -> Result<Arc<McpClient>, McpError> {
        if let Some(existing) = self.clients.lock().await.get(id).cloned() {
            return Ok(existing);
        }
        let cfg = self
            .get(id)
            .ok_or_else(|| McpError::spawn(format!("no MCP server configured as {:?}", id)))?;
        // Connect outside the map lock: a slow `npx` must not block the others.
        let client = Arc::new(McpClient::connect_with(&cfg, &self.lookup()).await?);
        let mut map = self.clients.lock().await;
        // Someone may have connected the same server while we were waiting.
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

    /// Ids with a process/session alive right now.
    pub async fn connected(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.clients.lock().await.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Closes one client, if it is up.
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

    /// Closes every client that has been idle past the timeout. Returns the
    /// ids it closed.
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

    /// Everything down. Called when the section closes and at shutdown.
    pub async fn close_all(&self) {
        let clients: Vec<Arc<McpClient>> = {
            let mut map = self.clients.lock().await;
            map.drain().map(|(_, c)| c).collect()
        };
        for c in clients {
            c.close().await;
        }
        if let Ok(mut g) = self.reaper.lock() {
            if let Some(h) = g.take() {
                h.abort();
            }
        }
    }

    /// Starts the reaper if it is not running. Holds only a `Weak`, so a
    /// dropped registry cannot be kept alive by its own timer.
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
                    return; // nothing alive: stop ticking
                }
            }
        }));
    }

    /// `true` while the idle reaper is running. The budget claim "0 processes
    /// and 0 timers with the section closed" is checked through this.
    pub fn reaper_running(&self) -> bool {
        self.reaper
            .lock()
            .map(|g| g.as_ref().map(|h| !h.is_finished()).unwrap_or(false))
            .unwrap_or(false)
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
        // Live children are killed by `kill_on_drop` on the stdio transport.
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::core::llm::roster_store::llm_dir().map(|d| d.join(CONFIG_FILE))
}

/// A config file that is missing or corrupt is an empty list, never a failure
/// to start: a hand-edited file must not take the section down.
fn read_file(path: &Path) -> Vec<McpServerConfig> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<McpServerConfig>>(&t).ok())
        .unwrap_or_default()
}
