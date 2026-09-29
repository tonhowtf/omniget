//! CLI accounts: pointers to an isolated config directory, never to a
//! credential. Owned by f4-cli-runtime.
//!
//! One account is one directory. The CLI itself logs into it (the app opens a
//! terminal running `claude` with that `CLAUDE_CONFIG_DIR`); the app only ever
//! creates the directory, links the parts that are *not* identity, and passes
//! the environment variable down to the child process.
//!
//! Plan §9.2 and `estudos/74` §A.3: never read `~/.claude/.credentials.json`,
//! never read the macOS keychain item `Claude Code-credentials`, never refresh
//! a token, never call `/api/oauth/usage`. [`DENY_SHARE`] is the list of names
//! that are never linked or copied out of an existing profile.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ERR_CLI_ACCOUNT, ERR_CLI_IO, ERR_CLI_NOT_FOUND};

/// Which CLI an account drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    Claude,
    Codex,
}

impl CliKind {
    pub const ALL: [CliKind; 2] = [CliKind::Claude, CliKind::Codex];

    /// Binary name, without the platform extension (`find_tool` adds `.exe`).
    pub fn bin(self) -> &'static str {
        match self {
            CliKind::Claude => "claude",
            CliKind::Codex => "codex",
        }
    }

    /// The environment variable that moves the whole config dir of this CLI.
    /// Both are documented and supported upstream (`estudos/74` §A.2).
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

/// How much the CLI is allowed to touch during a turn OmniGet drives.
///
/// One knob for both CLIs, because the user thinks in "can it write?", not in
/// vendor flags. It maps to:
///
/// - Codex: `--sandbox read-only` / `--sandbox workspace-write`
///   (`codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs`; `read-only` is also the
///   CLI's own `#[default]` in `codex-rs/protocol/src/config_types.rs`).
///   `danger-full-access` exists upstream and is deliberately not offered here.
/// - Claude Code: `--permission-mode plan` / `--permission-mode acceptEdits`
///   (`claude --help`, 2.1.276), on top of the `--permission-prompts none` the
///   runtime always passes.
///
/// [`SandboxMode::ReadOnly`] is the `Default` and the value a new account gets:
/// a turn never writes unless the user changed this on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    #[default]
    ReadOnly,
    /// Codex `workspace-write`, Claude `acceptEdits`: the agent may edit files
    /// in its working directory.
    Write,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxMode::ReadOnly => "read-only",
            SandboxMode::Write => "write",
        }
    }

    /// Parses what the UI sends back. Anything unknown falls back to the safe
    /// mode instead of failing the call: a typo must never grant a write.
    pub fn parse(s: &str) -> SandboxMode {
        match s.trim().to_ascii_lowercase().as_str() {
            "write" | "workspace-write" | "workspace_write" | "accept-edits" | "acceptedits" => {
                SandboxMode::Write
            }
            _ => SandboxMode::ReadOnly,
        }
    }

    /// True when a turn in this mode can modify files. Used by the UI to decide
    /// whether switching needs a confirmation.
    pub fn writes(self) -> bool {
        self == SandboxMode::Write
    }
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A CLI found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliDetected {
    pub cli: CliKind,
    pub path: PathBuf,
    /// `None` when the binary is there but `--version` did not parse.
    pub version: Option<String>,
}

/// One account: a label and a directory. Nothing else is stored, ever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliAccount {
    pub id: String,
    pub cli: CliKind,
    pub config_dir: PathBuf,
    pub label: String,
    /// Set by the user in the Accounts tab; a disabled account is skipped by
    /// the router and never spawned.
    #[serde(default)]
    pub disabled: bool,
    /// What a turn on this account may touch. Missing in an older
    /// `accounts.json` means [`SandboxMode::ReadOnly`], so an upgrade never
    /// silently hands an existing account the right to write.
    #[serde(default)]
    pub sandbox: SandboxMode,
}

/// Stable error with a code the UI maps, same shape as `RosterError`.
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

/// An account id is a slug, like an agent id: `[a-z0-9_-]{1,64}`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Names that are never linked, copied or read out of an existing profile.
///
/// The first three are the identity itself (`estudos/74` §A.1: claude-swap
/// reads exactly these and that is the part we refuse to copy). The rest are
/// per-process state that must not be shared between two live sessions.
pub const DENY_SHARE: &[&str] = &[
    ".credentials.json",
    "credentials.json",
    "auth.json",
    "statsig",
    "ide",
];

/// Names that ARE shared, by symlink, from an existing profile: everything the
/// user configured that is not identity (`estudos/74` §A.3 item 2).
pub const SHARE_LINK: &[&str] = &[
    "settings.json",
    "keybindings.json",
    "CLAUDE.md",
    "skills",
    "commands",
    "agents",
    "plugins",
];

/// True when this entry must never leave the source profile: the deny list, or
/// any daemon/lock/socket file.
pub fn is_denied(name: &str) -> bool {
    if DENY_SHARE.contains(&name) {
        return true;
    }
    name.starts_with("daemon")
        || name.ends_with(".lock")
        || name.ends_with(".sock")
        || name.ends_with(".pid")
}

/// `.claude.json` minus the identity. Pure, so the test can prove that no
/// account field survives the copy.
///
/// `oauthAccount` is what pins the org (`estudos/74` §A.1: injecting only the
/// token gives a silent 403), so a fresh profile must not inherit it.
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

/// Environment for one account: the config var plus the scrubbing the upstream
/// switcher does (`estudos/74` §A.1). Returned as a map so the caller can test
/// it without spawning anything; keys with an empty value mean "remove".
pub fn account_env(account: &CliAccount) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    // An empty `config_dir` is the CLI's own default profile: the login the
    // user already has in their terminal. Nothing is redirected.
    if !account.config_dir.as_os_str().is_empty() {
        env.insert(
            account.cli.config_env().to_string(),
            account.config_dir.display().to_string(),
        );
    }
    if account.cli == CliKind::Claude && account.sandbox == SandboxMode::ReadOnly {
        // Stops the CLI from leaking our env into the tools it spawns. Measured
        // on Claude Code 2.1.277: with this set the CLI drops `--permission-mode`
        // back to `default`, so with `--permission-prompts none` every write is
        // denied. That is what a read-only account wants and exactly what a
        // write account must not get. The secrets are already removed from the
        // CLI's own environment by `SCRUB_ENV`.
        env.insert("CLAUDE_CODE_SUBPROCESS_ENV_SCRUB".into(), "1".into());
    }
    env
}

/// Variables removed before launching, so an API key in the OmniGet process
/// never decides which account the CLI bills.
pub const SCRUB_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    // OmniGet's own bridge credentials (2.1): a job's child gets only its
    // job token, set explicitly, never a key the app was started with.
    "OMNIGET_TOKEN",
    "OMNIGET_BRIDGE_TOKEN",
    "OMNIGET_MCP_TOKEN",
    "OMNIGET_JOB_TOKEN",
];

/// Creates the directory for a new account. The directory is left **empty** on
/// purpose: the CLI writes its own credential there when the user runs
/// `/login`, and the app never looks inside.
pub fn create_profile_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .map_err(|e| AccountError::new(ERR_CLI_IO, format!("cannot create {}: {e}", dir.display())))
}

/// Links the non-identity parts of `source` into `dest`. Opt-in: the caller
/// only does this when the user asked to share settings and skills.
///
/// Returns the names actually linked. Anything in [`is_denied`] is skipped
/// even when the caller names it.
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

/// Parses `claude --version` / `codex --version` output. Both print the number
/// first (`2.1.276 (Claude Code)`, `codex-cli 0.58.0`), so take the first token
/// that looks like a version.
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

/// Which CLIs are installed. Runs `--version` once per CLI; nothing else.
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

/// `<app_data>/llm/accounts.json`.
pub fn default_path() -> Option<PathBuf> {
    super::super::roster_store::llm_dir().map(|d| d.join("accounts.json"))
}

/// Where a new account's config dir goes: `<app_data>/llm/profiles/<id>`.
pub fn default_profile_dir(id: &str) -> Option<PathBuf> {
    super::super::roster_store::llm_dir().map(|d| d.join("profiles").join(id))
}

/// Cached, atomically written account list. Same shape as `RosterStore`.
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

    /// Creates the account and its (empty) config dir. Fails on a duplicate id
    /// so two accounts can never share one directory by accident.
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
        if let Some(other) = all.iter().find(|a| a.config_dir == account.config_dir) {
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

    /// Changes what a turn on this account may touch. The UI confirms before
    /// calling this with [`SandboxMode::Write`].
    pub fn set_sandbox(&self, id: &str, sandbox: SandboxMode) -> Result<CliAccount> {
        let mut account = self
            .get(id)
            .ok_or_else(|| AccountError::new(ERR_CLI_NOT_FOUND, format!("no account {id}")))?;
        account.sandbox = sandbox;
        self.update(account)
    }

    /// Forgets the account. The config directory is **not** deleted: it holds
    /// the user's login and only the user decides to throw it away.
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

    /// An `accounts.json` written before this field existed must not turn into
    /// a write-enabled account on upgrade.
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
        // Built the way the account is, so the separator is the platform's.
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
        // Nothing that looks like a secret ever goes in.
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
