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
        let changed = {
            let mut guard = self
                .configs
                .write()
                .map_err(|_| McpError::spawn("the server list is poisoned"))?;
            match guard.iter().position(|c| c.id == cfg.id) {
                Some(i) => {
                    let differs = guard[i] != cfg;
                    guard[i] = cfg.clone();
                    differs
                }
                None => {
                    guard.push(cfg.clone());
                    false
                }
            }
        };
        if changed {
            self.disconnect(&cfg.id).await;
        }
        self.save()?;
        Ok(cfg)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mcp::client::fixture::{have_node, stdio_config};
    use crate::core::mcp::types::Transport;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-mcp-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn http_config(id: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            name: id.into(),
            transport: Transport::Http {
                url: "https://example.com/mcp".into(),
                headers: Default::default(),
            },
            enabled: true,
            timeout_ms: None,
        }
    }

    #[tokio::test]
    async fn servers_survive_a_restart_of_the_registry() {
        let dir = temp_dir("persist");
        let path = dir.join(CONFIG_FILE);
        let reg = McpRegistry::at(&path);
        reg.upsert(http_config("a")).await.unwrap();
        reg.upsert(http_config("b")).await.unwrap();
        assert_eq!(reg.list().len(), 2);

        let again = McpRegistry::at(&path);
        assert_eq!(again.list().len(), 2);
        assert_eq!(again.get("b").unwrap().name, "b");
        assert!(again.get("zzz").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn upsert_replaces_instead_of_duplicating() {
        let reg = McpRegistry::in_memory();
        reg.upsert(http_config("a")).await.unwrap();
        let mut second = http_config("a");
        second.name = "renamed".into();
        reg.upsert(second).await.unwrap();
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.get("a").unwrap().name, "renamed");
    }

    #[tokio::test]
    async fn a_bad_id_is_refused() {
        let reg = McpRegistry::in_memory();
        let err = reg.upsert(http_config("Bad Id")).await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_SPAWN);
        assert!(reg.list().is_empty());
    }

    #[tokio::test]
    async fn remove_reports_whether_there_was_anything_to_remove() {
        let dir = temp_dir("remove");
        let reg = McpRegistry::at(dir.join(CONFIG_FILE));
        reg.upsert(http_config("a")).await.unwrap();
        assert!(reg.remove("a").await.unwrap());
        assert!(!reg.remove("a").await.unwrap());
        assert!(reg.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_corrupt_config_file_loads_as_an_empty_list() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        std::fs::write(&path, "{ not json ]").unwrap();
        assert!(McpRegistry::at(&path).list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The claim at the top of this file, end to end: a config handed in with
    /// a literal header **and** a literal environment variable reaches the disk
    /// with neither, and the server process still receives both values.
    #[tokio::test]
    async fn literal_secrets_never_reach_the_disk_and_still_reach_the_server() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let dir = temp_dir("extract");
        let path = dir.join(CONFIG_FILE);
        let store: Arc<dyn SecretStore> = Arc::new(crate::core::secrets::FileSecretStore::new(
            dir.join("secrets"),
        ));
        let reg = Arc::new(McpRegistry::at(&path).with_secret_store(store.clone()));

        // stdio: GITHUB_TOKEN=y
        let mut stdio = stdio_config("fx", &[], Some(10_000));
        if let Transport::Stdio { env, .. } = &mut stdio.transport {
            env.insert("GITHUB_TOKEN".into(), "y".into());
        }
        reg.upsert(stdio).await.unwrap();

        // http: Authorization: Bearer x
        let server = crate::core::mcp::client::fixture::http(&[]).await;
        let mut http_cfg = server.config("web", Some(10_000));
        if let Transport::Http { headers, .. } = &mut http_cfg.transport {
            headers.insert("Authorization".into(), "Bearer x".into());
        }
        reg.upsert(http_cfg).await.unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("Bearer x"), "{}", on_disk);
        assert!(
            !on_disk.contains("\"y\""),
            "the env literal is on disk: {}",
            on_disk
        );
        assert!(on_disk.contains("secret:fx-env-github_token"));
        assert!(on_disk.contains("secret:web-header-authorization"));
        assert_eq!(
            store
                .get(SECRET_NS, "fx-env-github_token")
                .unwrap()
                .as_deref(),
            Some("y")
        );

        // The child process really got the resolved value.
        let client = reg.client_for("fx").await.unwrap();
        let out = client
            .call("env", serde_json::json!({ "name": "GITHUB_TOKEN" }), None)
            .await
            .unwrap();
        assert_eq!(out["structuredContent"]["value"], "y");

        // And so did the HTTP server, in the header.
        let web = reg.client_for("web").await.unwrap();
        let seen = web
            .call("headers", serde_json::json!({}), None)
            .await
            .unwrap();
        assert_eq!(
            seen["structuredContent"]["headers"]["authorization"],
            "Bearer x"
        );

        // Removing the server takes its secrets with it.
        reg.remove("fx").await.unwrap();
        assert_eq!(store.get(SECRET_NS, "fx-env-github_token").unwrap(), None);
        assert_eq!(
            store
                .get(SECRET_NS, "web-header-authorization")
                .unwrap()
                .as_deref(),
            Some("Bearer x"),
            "the other server keeps its own"
        );

        reg.close_all().await;
        server.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_saved_file_keeps_the_secret_reference_not_the_secret() {
        let dir = temp_dir("secrets");
        let path = dir.join(CONFIG_FILE);
        let reg = McpRegistry::at(&path);
        let mut cfg = http_config("gh");
        cfg.transport = Transport::Http {
            url: "https://example.com/mcp".into(),
            headers: [("Authorization".to_string(), "secret:gh".to_string())]
                .into_iter()
                .collect(),
        };
        reg.upsert(cfg).await.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("secret:gh"));
        assert!(!text.contains("Bearer"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn asking_for_an_unknown_server_does_not_start_anything() {
        let reg = Arc::new(McpRegistry::in_memory());
        let err = reg.client_for("ghost").await.unwrap_err();
        assert_eq!(err.code, super::super::error::ERR_MCP_SPAWN);
        assert!(reg.connected().await.is_empty());
        assert!(!reg.reaper_running());
    }

    #[tokio::test]
    async fn nothing_runs_until_a_client_is_asked_for() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let reg = Arc::new(McpRegistry::in_memory());
        reg.upsert(stdio_config("fx", &[], Some(10_000)))
            .await
            .unwrap();
        // Configured, but nobody asked: no process, no timer.
        assert!(reg.connected().await.is_empty());
        assert!(!reg.reaper_running());

        let a = reg.client_for("fx").await.unwrap();
        let b = reg.client_for("fx").await.unwrap();
        assert!(Arc::ptr_eq(&a, &b), "one process per server, not two");
        assert_eq!(reg.connected().await, vec!["fx".to_string()]);
        assert!(reg.reaper_running());

        reg.close_all().await;
        assert!(reg.connected().await.is_empty());
        assert!(!reg.reaper_running());
    }

    #[tokio::test]
    async fn an_idle_client_is_reaped_and_a_busy_one_is_not() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        // The background reaper is parked (60 s) so this test drives `reap_idle`
        // itself: otherwise a `node` start-up that takes longer than the idle
        // window closes the first server while the second is still spawning.
        let reg = Arc::new(
            McpRegistry::in_memory()
                .with_idle_timeout(Duration::from_millis(150))
                .with_reap_interval(Duration::from_secs(60)),
        );
        reg.upsert(stdio_config("idle", &[], Some(10_000)))
            .await
            .unwrap();
        reg.upsert(stdio_config("busy", &[], Some(10_000)))
            .await
            .unwrap();
        let busy = reg.client_for("busy").await.unwrap();
        reg.client_for("idle").await.unwrap();
        assert_eq!(reg.connected().await.len(), 2);

        // Keep one of them working while the other goes cold.
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(40)).await;
            busy.call("echo", serde_json::json!({ "text": "x" }), None)
                .await
                .unwrap();
        }
        let closed = reg.reap_idle().await;
        assert_eq!(closed, vec!["idle".to_string()]);
        assert_eq!(reg.connected().await, vec!["busy".to_string()]);
        reg.close_all().await;
    }

    /// The reaper is a timer: once it has nothing to watch it must return, or
    /// the "0 timers with the section closed" line of the budget is a lie.
    #[tokio::test]
    async fn the_reaper_stops_itself_once_the_last_client_is_gone() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let reg = Arc::new(
            McpRegistry::in_memory()
                .with_idle_timeout(Duration::from_millis(50))
                .with_reap_interval(Duration::from_millis(50)),
        );
        reg.upsert(stdio_config("fx", &[], Some(10_000)))
            .await
            .unwrap();
        reg.client_for("fx").await.unwrap();
        assert!(reg.reaper_running());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while reg.reaper_running() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!reg.reaper_running(), "the reaper should have returned");
        assert!(reg.connected().await.is_empty());
    }

    #[tokio::test]
    async fn changing_a_server_drops_its_running_client() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let reg = Arc::new(McpRegistry::in_memory());
        reg.upsert(stdio_config("fx", &[], Some(10_000)))
            .await
            .unwrap();
        reg.client_for("fx").await.unwrap();
        assert_eq!(reg.connected().await.len(), 1);
        let mut changed = stdio_config("fx", &["--garbage"], Some(10_000));
        changed.name = "other".into();
        reg.upsert(changed).await.unwrap();
        assert!(
            reg.connected().await.is_empty(),
            "the old process must not survive an edit"
        );
        reg.close_all().await;
    }

    #[tokio::test]
    async fn removing_a_server_stops_its_process() {
        if !have_node() {
            eprintln!("skipping: node is not on PATH");
            return;
        }
        let reg = Arc::new(McpRegistry::in_memory());
        reg.upsert(stdio_config("fx", &[], Some(10_000)))
            .await
            .unwrap();
        reg.client_for("fx").await.unwrap();
        assert!(reg.remove("fx").await.unwrap());
        assert!(reg.connected().await.is_empty());
        assert!(reg.get("fx").is_none());
    }
}
