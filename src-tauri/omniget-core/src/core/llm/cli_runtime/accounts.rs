use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ERR_CLI_ACCOUNT, ERR_CLI_IO, ERR_CLI_NOT_FOUND};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    Claude,
    Codex,
}

impl CliKind {
    pub const ALL: [CliKind; 2] = [CliKind::Claude, CliKind::Codex];

    pub fn bin(self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
        }
    }

    pub fn config_env(self) -> &'static str {
        match self {
            CliKind::Claude => "CLAUDE_CONFIG_DIR",
            CliKind::Codex => "CODEX_HOME",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Option<CliKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(CliKind::Claude),
            "codex" | "codex-cli" => Some(CliKind::Codex),
            _ => None,
        }
    }
}

impl std::fmt::Display for CliKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    #[default]
    ReadOnly,
    Write,
    Full,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxMode::ReadOnly => "read-only",
            SandboxMode::Write => "write",
            SandboxMode::Full => "full",
        }
    }

    pub fn parse(s: &str) -> SandboxMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "write" | "workspace-write" | "workspace_write" | "accept-edits" | "acceptedits" => {
                SandboxMode::Write
            }
            "full" | "full-access" | "full_access" => SandboxMode::Full,
            _ => SandboxMode::ReadOnly,
        }
    }

    pub fn writes(self) -> bool {
        self != SandboxMode::ReadOnly
    }
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliDetected {
    pub cli: CliKind,
    pub path: PathBuf,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliAccount {
    pub id: String,
    pub cli: CliKind,
    pub config_dir: PathBuf,
    pub label: String,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub sandbox: SandboxMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountError {
    pub code: &'static str,
    pub message: String,
}

impl AccountError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AccountError {}

pub type Result<T> = std::result::Result<T, AccountError>;

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub const DENY_SHARE: &[&str] = &[
    ".credentials.json",
    "credentials.json",
    "auth.json",
    "statsig",
    "ide",
];

pub const SHARE_LINK: &[&str] = &[
    "settings.json",
    "keybindings.json",
    "CLAUDE.md",
    "skills",
    "commands",
    "agents",
    "plugins",
];

pub fn is_denied(name: &str) -> bool {
    if DENY_SHARE.contains(&name) {
        return true;
    }
    name.starts_with("daemon")
        || name.ends_with(".lock")
        || name.ends_with(".sock")
        || name.ends_with(".pid")
}

pub fn sanitize_claude_json(input: &str) -> std::result::Result<String, serde_json::Error> {
    const DROP: &[&str] = &[
        "oauthAccount",
        "userID",
        "organizationUuid",
        "primaryApiKey",
        "customApiKeyResponses",
    ];
    let mut value: Value = serde_json::from_str(input)?;
    if let Some(map) = value.as_object_mut() {
        for key in DROP {
            map.remove(*key);
        }
    }
    serde_json::to_string_pretty(&value)
}

pub fn account_env(account: &CliAccount) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if !account.config_dir.as_os_str().is_empty() {
        env.insert(
            account.cli.config_env().to_string(),
            account.config_dir.display().to_string(),
        );
    }
    if account.cli == CliKind::Claude && account.sandbox == SandboxMode::ReadOnly {
        env.insert("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB".into(), "1".into());
    }
    env
}

pub const SCRUB_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OMNIGET_TOKEN",
    "OMNIGET_BRIDGE_TOKEN",
    "OMNIGET_MCP_TOKEN",
    "OMNIGET_JOB_TOKEN",
    "OMNIGET_ENV",
    "OMNIGET_JOB_ID",
    "OMNIGET_AGENT_ID",
    "OMNIGET_BRIDGE_URL",
];

pub const CREDENTIAL_SCRUB: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
];

pub fn create_profile_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| AccountError::new(ERR_CLI_IO, format!("cannot create {}: {e}", dir.display())))
}

pub fn share_profile(source: &Path, dest: &Path, names: &[&str]) -> Result<Vec<String>> {
    create_profile_dir(dest)?;
    let mut linked = Vec::new();
    for name in names {
        if is_denied(name) {
            continue;
        }
        let from = source.join(name);
        if !from.exists() {
            continue;
        }
        let to = dest.join(name);
        if to.exists() || to.symlink_metadata().is_ok() {
            continue;
        }
        if symlink(&from, &to).is_ok() {
            linked.push((*name).to_string());
        }
    }
    Ok(linked)
}

#[cfg(unix)]
fn symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(from, to)
}

#[cfg(windows)]
fn symlink(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::os::windows::fs::symlink_dir(from, to)
    } else {
        std::os::windows::fs::symlink_file(from, to)
    }
}

pub fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .find(|tok| {
            let t = tok.trim_start_matches('v');
            t.split('.').count() >= 2
                && t.chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
        })
        .map(|t| t.trim_start_matches('v').to_string())
}

pub async fn detect() -> Vec<CliDetected> {
    let mut found = Vec::new();
    for cli in CliKind::ALL {
        if let Some(d) = detect_one(cli).await {
            found.push(d);
        }
    }
    found
}

pub async fn detect_one(cli: CliKind) -> Option<CliDetected> {
    let path = crate::core::dependencies::find_tool(cli.bin()).await?;
    let version = version_of(&path).await;
    Some(CliDetected { cli, path, version })
}

async fn version_of(path: &Path) -> Option<String> {
    let out = crate::core::process::command(path)
        .arg("--version")
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text)
}

pub fn default_path() -> Option<PathBuf> {
    super::super::roster_store::llm_dir().map(|d| d.join("accounts.json"))
}

pub fn default_profile_dir(id: &str) -> Option<PathBuf> {
    super::super::roster_store::llm_dir().map(|d| d.join("profiles").join(id))
}

pub fn prefs_path() -> Option<PathBuf> {
    super::super::roster_store::llm_dir().map(|d| d.join("accounts-ui.json"))
}

pub fn normalize_chain(chain: &[String], accounts: &[CliAccount]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(accounts.len());
    for id in chain {
        if accounts.iter().any(|a| &a.id == id) && !out.contains(id) {
            out.push(id.clone());
        }
    }
    for a in accounts {
        if !out.contains(&a.id) {
            out.push(a.id.clone());
        }
    }
    out
}

pub fn move_to_head(chain: &[String], id: &str) -> Vec<String> {
    if !chain.iter().any(|c| c == id) {
        return chain.to_vec();
    }
    let mut out = vec![id.to_string()];
    out.extend(chain.iter().filter(|c| c.as_str() != id).cloned());
    out
}

fn chain_of(prefs: &Value) -> Vec<String> {
    prefs
        .get("chain")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn primary_in(prefs: &Value, accounts: &[CliAccount], cli: CliKind) -> Option<CliAccount> {
    chain_of(prefs).iter().find_map(|id| {
        accounts
            .iter()
            .find(|a| &a.id == id && a.cli == cli && !a.disabled)
            .cloned()
    })
}

pub fn promote_in(prefs: &Value, accounts: &[CliAccount], id: &str) -> Value {
    let chain = move_to_head(&normalize_chain(&chain_of(prefs), accounts), id);
    let mut out = match prefs {
        Value::Object(_) => prefs.clone(),
        _ => Value::Object(Default::default()),
    };
    out["chain"] = Value::from(chain);
    out
}

fn read_prefs(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

pub fn primary(store: &AccountStore, cli: CliKind) -> Option<CliAccount> {
    let prefs = read_prefs(&prefs_path()?);
    primary_in(&prefs, &store.list(), cli)
}

pub fn set_primary(store: &AccountStore, id: &str) -> Result<()> {
    if store.get(id).is_none() {
        return Err(AccountError::new(
            ERR_CLI_NOT_FOUND,
            format!("no account {id}"),
        ));
    }
    let path =
        prefs_path().ok_or_else(|| AccountError::new(ERR_CLI_IO, "no app data dir".to_string()))?;
    let prefs = promote_in(&read_prefs(&path), &store.list(), id);
    let text = serde_json::to_string_pretty(&prefs)
        .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes())
        .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))
}

pub struct AccountStore {
    path: PathBuf,
    cache: RwLock<Option<Arc<Vec<CliAccount>>>>,
}

impl AccountStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            cache: RwLock::new(None),
        }
    }

    pub fn default_store() -> Option<Self> {
        default_path().map(Self::at)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Arc<Vec<CliAccount>> {
        if let Some(cached) = self.cache.read().ok().and_then(|g| g.clone()) {
            return cached;
        }
        let loaded = Arc::new(self.read_from_disk());
        if let Ok(mut g) = self.cache.write() {
            *g = Some(loaded.clone());
        }
        loaded
    }

    pub fn get(&self, id: &str) -> Option<CliAccount> {
        self.list().iter().find(|a| a.id == id).cloned()
    }

    pub fn create(&self, account: CliAccount) -> Result<CliAccount> {
        if !valid_id(&account.id) {
            return Err(AccountError::new(
                ERR_CLI_ACCOUNT,
                format!("invalid account id: {:?}", account.id),
            ));
        }
        let mut all = (*self.list()).clone();
        if all.iter().any(|a| a.id == account.id) {
            return Err(AccountError::new(
                ERR_CLI_ACCOUNT,
                format!("account {} already exists", account.id),
            ));
        }
        let default_profile = account.config_dir.as_os_str().is_empty();
        if let Some(other) = all.iter().find(|a| {
            a.config_dir == account.config_dir && (!default_profile || a.cli == account.cli)
        }) {
            return Err(AccountError::new(
                ERR_CLI_ACCOUNT,
                format!("{} already uses that config dir", other.id),
            ));
        }
        create_profile_dir(&account.config_dir)?;
        all.push(account.clone());
        self.save(all)?;
        Ok(account)
    }

    pub fn update(&self, account: CliAccount) -> Result<CliAccount> {
        let mut all = (*self.list()).clone();
        let slot = all.iter_mut().find(|a| a.id == account.id).ok_or_else(|| {
            AccountError::new(ERR_CLI_NOT_FOUND, format!("no account {}", account.id))
        })?;
        *slot = account.clone();
        self.save(all)?;
        Ok(account)
    }

    pub fn set_disabled(&self, id: &str, disabled: bool) -> Result<CliAccount> {
        let mut account = self
            .get(id)
            .ok_or_else(|| AccountError::new(ERR_CLI_NOT_FOUND, format!("no account {id}")))?;
        account.disabled = disabled;
        self.update(account)
    }

    pub fn set_sandbox(&self, id: &str, sandbox: SandboxMode) -> Result<CliAccount> {
        let mut account = self
            .get(id)
            .ok_or_else(|| AccountError::new(ERR_CLI_NOT_FOUND, format!("no account {id}")))?;
        account.sandbox = sandbox;
        self.update(account)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut all = (*self.list()).clone();
        let before = all.len();
        all.retain(|a| a.id != id);
        if all.len() == before {
            return Err(AccountError::new(
                ERR_CLI_NOT_FOUND,
                format!("no account {id}"),
            ));
        }
        self.save(all)
    }

    fn read_from_disk(&self) -> Vec<CliAccount> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        match serde_json::from_str::<Vec<CliAccount>>(&text) {
            Ok(list) => list,
            Err(error) => {
                tracing::warn!("[llm] accounts.json is not readable ({error}); starting empty");
                Vec::new()
            }
        }
    }

    fn save(&self, accounts: Vec<CliAccount>) -> Result<()> {
        let text = serde_json::to_string_pretty(&accounts)
            .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| AccountError::new(ERR_CLI_IO, e.to_string()))?;
        if let Ok(mut g) = self.cache.write() {
            *g = Some(Arc::new(accounts));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omniget-cli-acct-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn account(id: &str, dir: &Path) -> CliAccount {
        CliAccount {
            id: id.into(),
            cli: CliKind::Claude,
            config_dir: dir.join(id),
            label: id.to_uppercase(),
            disabled: false,
            sandbox: SandboxMode::default(),
        }
    }

    #[test]
    fn the_primary_is_the_first_enabled_account_of_that_cli_in_the_chain() {
        let dir = Path::new("/tmp");
        let mut off = account("off", dir);
        off.disabled = true;
        let mut codex = account("codex", dir);
        codex.cli = CliKind::Codex;
        let accounts = vec![account("a", dir), account("b", dir), off, codex];
        let prefs = serde_json::json!({ "chain": ["codex", "off", "ghost", "b", "a"] });
        let p = primary_in(&prefs, &accounts, CliKind::Claude).unwrap();
        assert_eq!(p.id, "b");
        assert_eq!(
            primary_in(&prefs, &accounts, CliKind::Codex).unwrap().id,
            "codex"
        );
        assert!(primary_in(&Value::Null, &accounts, CliKind::Claude).is_none());
    }

    #[test]
    fn promoting_moves_the_account_first_and_keeps_the_other_settings() {
        let dir = Path::new("/tmp");
        let accounts = vec![account("a", dir), account("b", dir), account("c", dir)];
        let prefs = serde_json::json!({
            "chain": ["c", "ghost", "a"],
            "rotation": { "enabled": true, "threshold": 0.8 }
        });
        let out = promote_in(&prefs, &accounts, "a");
        assert_eq!(out["chain"], serde_json::json!(["a", "c", "b"]));
        assert_eq!(out["rotation"], prefs["rotation"]);
        let fresh = promote_in(&Value::Null, &accounts, "b");
        assert_eq!(fresh["chain"], serde_json::json!(["b", "a", "c"]));
    }

    #[test]
    fn a_new_account_is_read_only_and_says_so_on_the_wire() {
        assert_eq!(SandboxMode::default(), SandboxMode::ReadOnly);
        assert!(!SandboxMode::default().writes());
        assert_eq!(
            account("a", Path::new("/tmp")).sandbox,
            SandboxMode::ReadOnly
        );
        assert_eq!(
            serde_json::to_string(&SandboxMode::ReadOnly).unwrap(),
            "\"read-only\""
        );
        assert_eq!(
            serde_json::to_string(&SandboxMode::Write).unwrap(),
            "\"write\""
        );
    }

    #[test]
    fn an_account_without_the_field_reads_back_read_only() {
        let text = r#"[{"id":"max-1","cli":"claude","config_dir":"/tmp/max-1","label":"Max"}]"#;
        let list: Vec<CliAccount> = serde_json::from_str(text).unwrap();
        assert_eq!(list[0].sandbox, SandboxMode::ReadOnly);
        assert!(!list[0].disabled);
    }

    #[test]
    fn an_unknown_sandbox_string_never_grants_a_write() {
        assert_eq!(SandboxMode::parse("write"), SandboxMode::Write);
        assert_eq!(SandboxMode::parse("workspace-write"), SandboxMode::Write);
        assert_eq!(SandboxMode::parse("read-only"), SandboxMode::ReadOnly);
        for bad in [
            "",
            "danger-full-access",
            "yolo",
            "WRITE!!",
            "bypassPermissions",
        ] {
            assert_eq!(SandboxMode::parse(bad), SandboxMode::ReadOnly, "{bad}");
        }
    }

    #[test]
    fn set_sandbox_persists_and_survives_a_reopen() {
        let root = tmp("sandbox");
        let path = root.join("accounts.json");
        let store = AccountStore::at(&path);
        store.create(account("max-1", &root)).unwrap();
        assert_eq!(store.get("max-1").unwrap().sandbox, SandboxMode::ReadOnly);
        store.set_sandbox("max-1", SandboxMode::Write).unwrap();
        assert_eq!(
            AccountStore::at(&path).get("max-1").unwrap().sandbox,
            SandboxMode::Write
        );
        assert_eq!(
            store
                .set_sandbox("ghost", SandboxMode::Write)
                .unwrap_err()
                .code,
            ERR_CLI_NOT_FOUND
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn each_cli_gets_its_own_terminal_login_account() {
        let root = tmp("terminal");
        let store = AccountStore::at(root.join("accounts.json"));
        let terminal = |id: &str, cli: CliKind| CliAccount {
            id: id.into(),
            cli,
            config_dir: PathBuf::new(),
            label: "Terminal login".into(),
            disabled: false,
            sandbox: SandboxMode::Write,
        };
        store
            .create(terminal("claude-terminal", CliKind::Claude))
            .unwrap();
        store
            .create(terminal("codex-terminal", CliKind::Codex))
            .unwrap();
        assert!(store.create(terminal("claude-2", CliKind::Claude)).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn config_env_is_the_documented_variable_per_cli() {
        assert_eq!(CliKind::Claude.config_env(), "CLAUDE_CONFIG_DIR");
        assert_eq!(CliKind::Codex.config_env(), "CODEX_HOME");
        assert_eq!(CliKind::parse("Claude-Code"), Some(CliKind::Claude));
        assert_eq!(CliKind::parse("codex"), Some(CliKind::Codex));
        assert_eq!(CliKind::parse("gemini"), None);
    }

    #[test]
    fn credentials_and_runtime_state_are_never_shared() {
        for name in [
            ".credentials.json",
            "credentials.json",
            "daemon.json",
            "x.lock",
            "ide.sock",
            "statsig",
        ] {
            assert!(is_denied(name), "{name} must be denied");
        }
        for name in SHARE_LINK {
            assert!(!is_denied(name), "{name} must be shareable");
        }
    }

    #[test]
    fn share_profile_links_settings_and_skips_the_credential() {
        let root = tmp("share");
        let source = root.join("src");
        std::fs::create_dir_all(source.join("skills")).unwrap();
        std::fs::write(source.join("settings.json"), "{}").unwrap();
        std::fs::write(source.join(".credentials.json"), "SECRET").unwrap();
        let dest = root.join("dst");
        let mut names: Vec<&str> = SHARE_LINK.to_vec();
        names.push(".credentials.json");
        let linked = share_profile(&source, &dest, &names).unwrap();
        assert!(linked.contains(&"settings.json".to_string()));
        assert!(linked.contains(&"skills".to_string()));
        assert!(!linked.iter().any(|n| n.contains("credentials")));
        assert!(!dest.join(".credentials.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sanitize_claude_json_drops_the_identity() {
        let input = r#"{"oauthAccount":{"emailAddress":"a@b.c"},"userID":"u","theme":"dark","numStartups":3}"#;
        let out = sanitize_claude_json(input).unwrap();
        assert!(!out.contains("oauthAccount"));
        assert!(!out.contains("a@b.c"));
        assert!(!out.contains("userID"));
        assert!(out.contains("theme"));
        assert!(out.contains("numStartups"));
    }

    #[test]
    fn account_env_carries_only_pointers() {
        let a = account("max-1", Path::new("/tmp/p"));
        let env = account_env(&a);
        let expected = Path::new("/tmp/p").join("max-1").display().to_string();
        assert_eq!(
            env.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some(expected.as_str())
        );
        assert_eq!(
            env.get("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB")
                .map(String::as_str),
            Some("1")
        );
        assert!(!env
            .values()
            .any(|v| v.contains("sk-") || v.to_lowercase().contains("token")));
        assert!(SCRUB_ENV.contains(&"ANTHROPIC_API_KEY"));
        assert!(SCRUB_ENV.contains(&"CLAUDE_CODE_OAUTH_TOKEN"));
    }

    #[test]
    fn parse_version_reads_both_cli_shapes() {
        assert_eq!(
            parse_version("2.1.276 (Claude Code)").as_deref(),
            Some("2.1.276")
        );
        assert_eq!(parse_version("codex-cli 0.58.0").as_deref(), Some("0.58.0"));
        assert_eq!(parse_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn store_rejects_a_duplicate_id_and_a_shared_config_dir() {
        let root = tmp("store");
        let store = AccountStore::at(root.join("accounts.json"));
        assert!(store.list().is_empty());
        store.create(account("max-1", &root)).unwrap();
        let dup = store.create(account("max-1", &root)).unwrap_err();
        assert_eq!(dup.code, ERR_CLI_ACCOUNT);
        let mut same_dir = account("max-2", &root);
        same_dir.config_dir = root.join("max-1");
        let err = store.create(same_dir).unwrap_err();
        assert_eq!(err.code, ERR_CLI_ACCOUNT);
        assert!(root.join("max-1").is_dir(), "config dir is created empty");
        assert!(
            std::fs::read_dir(root.join("max-1"))
                .unwrap()
                .next()
                .is_none(),
            "the app writes nothing into the profile"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_round_trips_and_remove_keeps_the_directory() {
        let root = tmp("rt");
        let path = root.join("accounts.json");
        let store = AccountStore::at(&path);
        store.create(account("max-1", &root)).unwrap();
        store.create(account("max-2", &root)).unwrap();
        assert_eq!(store.list().len(), 2);
        let reopened = AccountStore::at(&path);
        assert_eq!(reopened.list().len(), 2);
        assert_eq!(reopened.get("max-2").unwrap().label, "MAX-2");
        store.set_disabled("max-1", true).unwrap();
        assert!(store.get("max-1").unwrap().disabled);
        store.remove("max-1").unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(root.join("max-1").is_dir(), "login survives the removal");
        assert_eq!(store.remove("max-1").unwrap_err().code, ERR_CLI_NOT_FOUND);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn invalid_ids_are_refused() {
        assert!(valid_id("max-1"));
        assert!(valid_id("work_2"));
        assert!(!valid_id(""));
        assert!(!valid_id("Max"));
        assert!(!valid_id("../escape"));
    }
}
