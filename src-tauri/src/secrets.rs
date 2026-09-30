//! Platform-neutral secret storage, shared by every feature that has to keep a
//! token or a private key on this machine.
//!
//! Extracted from `commands/omnidisc/store.rs`, which now re-exports from here
//! without changing behaviour. A [`Namespace`] is the unit of separation: it
//! carries the keyring service name, the folder the encrypted file store lives
//! in, and the environment variable that forces that folder somewhere else for
//! tests. The namespaces are [`OMNIDISC`], [`PROFILE`], [`AI_KEYS`] and [`MCP`].
//!
//! Where the bytes end up depends on [`use_keyring`]:
//!
//! | Platform | Default | Fallback |
//! | --- | --- | --- |
//! | macOS | encrypted file, until the binary ships signed with a stable identity | keyring with `OMNIGET_KEYRING=1` |
//! | Windows | Credential Manager in release builds | encrypted file |
//! | Linux | Secret Service over zbus (pure Rust; Flatpak needs `--talk-name=org.freedesktop.secrets`) | encrypted file on `PlatformFailure`/`NoStorageAccess` |
//!
//! WHY macOS defaults to the file: the release is not signed by the bundler, so
//! every new version is a new binary identity and the keychain asks for the
//! login password again — the same problem `tauri dev` always had, just slower
//! to notice. `codesign_is_stable()` measures that instead of guessing from
//! `debug_assertions`: an ad-hoc signature (`Signature=adhoc`, `TeamIdentifier=
//! not set`) is not stable, a Developer ID one is.
//!
//! A release build that does not write to the keyring still *reads* it once per
//! account, as a last resort, and copies what it finds into the file store —
//! otherwise an update would lose every secret the previous, keyring-writing
//! version left behind. See [`keyring_is_last_resort`].
//!
//! The file store is AES-256-CBC + HMAC-SHA256 with a random key written next
//! to it at 0600, so secrets never sit in plain text inside a backup or a synced
//! folder. It is a floor, not a keyring: anyone running as the same OS user can
//! read the key.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const SESSIONS_FILE: &str = "sessions.bin";
const KEY_FILE: &str = "session.key";
pub const IV_LEN: usize = 16;
const MAC_LEN: usize = 32;

type Enc = cbc::Encryptor<aes::Aes256>;
type Dec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// Forces every namespace at once into `<dir>/<namespace>`; a namespace's own
/// variable still wins. Tests use it so one temporary directory covers the
/// profile seed and the OmniDisc tokens together.
pub const SECRETS_DIR_ENV: &str = "OMNIGET_SECRETS_DIR";
/// Overrides the keyring decision in both directions.
pub const KEYRING_ENV: &str = "OMNIGET_KEYRING";
/// Kept for one version: the name this override had while it was OmniDisc-only.
pub const KEYRING_ENV_LEGACY: &str = "OMNIGET_OMNIDISC_KEYRING";

/// One store: a keyring service, a folder, and the variable that moves it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub service: &'static str,
    pub dir_name: &'static str,
    pub dir_env: &'static str,
}

/// OmniDisc session tokens, device keys and MLS state keys.
pub const OMNIDISC: Namespace = Namespace {
    service: "wtf.tonho.omniget.omnidisc",
    dir_name: "omnidisc",
    dir_env: "OMNIGET_OMNIDISC_SESSION_DIR",
};

/// The local profile's ed25519 seed.
pub const PROFILE: Namespace = Namespace {
    service: "wtf.tonho.omniget.profile",
    dir_name: "profile",
    dir_env: "OMNIGET_PROFILE_SECRET_DIR",
};

/// Provider API keys: the `ai-keys.json` vault and the app's own AI config.
/// `dir_name` matches `omniget_core::core::secrets::AI_KEYS`, which is the name
/// the core asks for over the [`AppSecretStore`] seam.
pub const AI_KEYS: Namespace = Namespace {
    service: "wtf.tonho.omniget.ai-keys",
    dir_name: "ai_keys",
    dir_env: "OMNIGET_AI_KEYS_SECRET_DIR",
};

/// Credentials of the external MCP servers: the header and environment values
/// `mcp-servers.json` only points at with `secret:<id>`. `dir_name` matches
/// `omniget_core::core::mcp::types::SECRET_NS`, the name the MCP registry asks
/// for over the [`AppSecretStore`] seam. Its own service and folder, so a
/// server token never shares a store with a provider key or the profile seed.
pub const MCP: Namespace = Namespace {
    service: "wtf.tonho.omniget.mcp",
    dir_name: "mcp",
    dir_env: "OMNIGET_MCP_SECRET_DIR",
};

/// The namespaces the core is allowed to name over the `SecretStore` seam.
pub fn namespace_by_name(name: &str) -> Option<Namespace> {
    match name {
        n if n == OMNIDISC.dir_name => Some(OMNIDISC),
        n if n == PROFILE.dir_name => Some(PROFILE),
        n if n == AI_KEYS.dir_name => Some(AI_KEYS),
        n if n == MCP.dir_name => Some(MCP),
        _ => None,
    }
}

/// The implementation `omniget-core` runs on inside the app: keychain where the
/// policy allows it, encrypted file otherwise. Installed once at boot with
/// `omniget_core::core::secrets::install(Arc::new(AppSecretStore))`; without
/// that call the core keeps its own file store, which is a different file, so
/// the install is not optional in the app.
pub struct AppSecretStore;

impl AppSecretStore {
    fn resolve(namespace: &str) -> Result<Namespace, String> {
        namespace_by_name(namespace)
            .ok_or_else(|| format!("secrets: unknown namespace {:?}", namespace))
    }
}

impl omniget_core::core::secrets::SecretStore for AppSecretStore {
    fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String> {
        load_secret(Self::resolve(namespace)?, account)
    }

    fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String> {
        save_secret(Self::resolve(namespace)?, account, value)
    }

    fn delete(&self, namespace: &str, account: &str) -> Result<(), String> {
        delete_secret(Self::resolve(namespace)?, account)
    }
}

// ---------------------------------------------------------------------------
// Keyring policy
// ---------------------------------------------------------------------------

/// The platforms the policy distinguishes. Spelled out instead of read from
/// `cfg!` at the call site so the decision below is one pure function that can
/// be tested for every platform from any platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Macos,
    Windows,
    Linux,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Platform::Macos,
            "windows" => Platform::Windows,
            "linux" => Platform::Linux,
            _ => Platform::Other,
        }
    }
}

/// Whether the `keyring` crate is compiled in at all on this target. It is a
/// per-platform dependency in `Cargo.toml`; on Linux it is absent, so the
/// policy has to answer "no" there no matter what the environment says.
pub const KEYRING_COMPILED: bool = cfg!(any(target_os = "macos", windows, target_os = "linux"));

/// The one place that decides. Inputs are explicit so the table in the module
/// docs is a test, not a comment.
pub fn keyring_policy(
    platform: Platform,
    env_override: Option<bool>,
    codesign_stable: bool,
    debug_build: bool,
    keyring_compiled: bool,
) -> bool {
    if !keyring_compiled {
        return false;
    }
    if let Some(forced) = env_override {
        return forced;
    }
    match platform {
        // Signed with a stable identity or nothing: an ad-hoc signature changes
        // with every build and voids the "Always Allow" grant.
        Platform::Macos => codesign_stable,
        // The Credential Manager never prompts, but a debug build is still a
        // throwaway identity, so keep release-only as it has always been.
        Platform::Windows => !debug_build,
        // Secret Service over zbus (plan §9.4); PlatformFailure/NoStorageAccess
        // fall back to the encrypted file store at call time.
        Platform::Linux => true,
        Platform::Other => false,
    }
}

/// `1`/`true`/`yes`/`on` and `0`/`false`/`no`/`off`, case-insensitive; anything
/// else is not an answer and falls through to the platform default.
pub fn parse_bool_env(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn keyring_env_override() -> Option<bool> {
    for key in [KEYRING_ENV, KEYRING_ENV_LEGACY] {
        if let Ok(raw) = std::env::var(key) {
            if let Some(value) = parse_bool_env(&raw) {
                return Some(value);
            }
        }
    }
    None
}

/// True when this binary carries a code signature that survives an update, so
/// a keychain grant given once keeps working. Ad-hoc signatures do not: they
/// change with every build.
pub fn codesign_output_is_stable(output: &str) -> bool {
    let mut team_is_set = false;
    let mut adhoc = false;
    let mut developer_id = false;
    for line in output.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("TeamIdentifier=") {
            team_is_set = !value.trim().is_empty() && value.trim() != "not set";
        } else if let Some(value) = line.strip_prefix("Signature=") {
            adhoc = value.trim().eq_ignore_ascii_case("adhoc");
        } else if let Some(value) = line.strip_prefix("Authority=") {
            let value = value.trim();
            if value.starts_with("Developer ID Application")
                || value.starts_with("Apple Development")
                || value.starts_with("Apple Distribution")
            {
                developer_id = true;
            }
        }
    }
    !adhoc && team_is_set && developer_id
}

/// Ask `codesign` about the running executable, once per process. WHY cached:
/// this spawns a process, and `load_secret` is on the path of every gateway
/// reconnect.
#[cfg(target_os = "macos")]
pub fn codesign_is_stable() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let out = std::process::Command::new("/usr/bin/codesign")
            .args(["-dv", "--verbose=4"])
            .arg(&exe)
            .output();
        match out {
            // codesign writes its report to stderr; stdout is joined in case a
            // future version moves it.
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
                text.push('\n');
                text.push_str(&String::from_utf8_lossy(&out.stdout));
                codesign_is_stable_from(&text)
            }
            Err(e) => {
                tracing::warn!("[secrets] codesign check failed, assuming unsigned: {}", e);
                false
            }
        }
    })
}

#[cfg(not(target_os = "macos"))]
pub fn codesign_is_stable() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn codesign_is_stable_from(text: &str) -> bool {
    let stable = codesign_output_is_stable(text);
    tracing::info!(
        "[secrets] code signature is {}; keyring default is {}",
        if stable { "stable" } else { "not stable" },
        if stable { "on" } else { "off" }
    );
    stable
}

/// Whether to put secrets in the OS keyring at all on this machine, right now.
pub fn use_keyring() -> bool {
    keyring_policy(
        Platform::current(),
        keyring_env_override(),
        codesign_is_stable(),
        cfg!(debug_assertions),
        KEYRING_COMPILED,
    )
}

// ---------------------------------------------------------------------------
// Cache and paths
// ---------------------------------------------------------------------------

/// Read-through cache so one launch asks the OS once per secret instead of once
/// per call. Every gateway reconnect and every authenticated request would
/// otherwise reach for the token again.
fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The key carries the namespace and the store the value came from, not just
/// the account. The integration tests give each side its own forced directory
/// and switch between them in one process, so an account-only key would hand
/// one side the other's device key.
fn cache_key(ns: Namespace, account: &str) -> String {
    match forced_dir(ns) {
        Some(dir) => format!("{}|{}|{}", ns.service, dir.display(), account),
        None => format!("{}|<default>|{}", ns.service, account),
    }
}

fn cache_put(ns: Namespace, account: &str, value: Option<String>) {
    if let Ok(mut c) = cache().lock() {
        c.insert(cache_key(ns, account), value);
    }
}

fn cache_get(ns: Namespace, account: &str) -> Option<Option<String>> {
    cache()
        .lock()
        .ok()
        .and_then(|c| c.get(&cache_key(ns, account)).cloned())
}

/// Drop everything the cache holds for a namespace. Tests that move the forced
/// directory around in one process call it; production never needs to.
pub fn clear_cache() {
    if let Ok(mut c) = cache().lock() {
        c.clear();
    }
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
}

/// The directory a forced environment points this namespace at, if any. The
/// namespace's own variable is used literally (the OmniDisc integration tests
/// expect exactly that directory); the shared one gets the namespace folder
/// appended so two namespaces under one temporary root stay apart.
pub fn forced_dir(ns: Namespace) -> Option<PathBuf> {
    env_dir(ns.dir_env).or_else(|| env_dir(SECRETS_DIR_ENV).map(|d| d.join(ns.dir_name)))
}

/// Directory the file store writes into. Also where a namespace's other blobs
/// live (MLS state, device ids), so one forced variable moves all of a
/// namespace's data at once instead of half of it.
pub fn base_dir(ns: Namespace) -> Result<PathBuf, String> {
    if let Some(dir) = forced_dir(ns) {
        return Ok(dir);
    }
    let base = crate::core::paths::app_data_dir()
        .ok_or_else(|| "secrets: could not resolve the app data directory".to_string())?;
    Ok(base.join(ns.dir_name))
}

/// Secrets that belong to the same account but are different kinds of material
/// share the store under a suffixed account, so one instance's material stays
/// together and is dropped together on logout.
pub fn secret_account(url: &str, kind: &str) -> String {
    format!("{}#{}", url, kind)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn save_secret(ns: Namespace, account: &str, value: &str) -> Result<(), String> {
    cache_put(ns, account, Some(value.to_string()));
    if let Some(dir) = forced_dir(ns) {
        return FileStore::new(dir).set(account, value);
    }
    match keyring_set(ns.service, account, value) {
        Some(Ok(())) => return Ok(()),
        Some(Err(e)) => tracing::warn!("[secrets] keyring write failed, using file store: {}", e),
        None => {}
    }
    FileStore::for_namespace(ns)?.set(account, value)
}

pub fn load_secret(ns: Namespace, account: &str) -> Result<Option<String>, String> {
    if let Some(hit) = cache_get(ns, account) {
        return Ok(hit);
    }
    if let Some(dir) = forced_dir(ns) {
        let found = FileStore::new(dir).get(account)?;
        cache_put(ns, account, found.clone());
        return Ok(found);
    }
    match keyring_get(ns.service, account) {
        Some(Ok(found)) => {
            if found.is_some() {
                cache_put(ns, account, found.clone());
                return Ok(found);
            }
        }
        Some(Err(e)) => tracing::warn!("[secrets] keyring read failed, using file store: {}", e),
        None => {}
    }
    let store = FileStore::for_namespace(ns)?;
    let found = match store.get(account)? {
        Some(found) => Some(found),
        // Nothing in the file. Before concluding the secret does not exist, ask
        // the keyring an older build of this app may have written it to.
        None => adopt_from_keyring(ns, account, &store)?,
    };
    cache_put(ns, account, found.clone());
    Ok(found)
}

/// The one-time upgrade read described on [`keyring_is_last_resort`]. A secret
/// found there is copied into the file store, so this costs one keyring call
/// per account per machine and every later read is a file read.
fn adopt_from_keyring(
    ns: Namespace,
    account: &str,
    store: &FileStore,
) -> Result<Option<String>, String> {
    if !keyring_is_last_resort() {
        return Ok(None);
    }
    let found = match keyring_get_raw(ns.service, account) {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!("[secrets] last-resort keyring read failed: {}", e);
            return Ok(None);
        }
    };
    let Some(value) = found else {
        return Ok(None);
    };
    match store.set(account, &value) {
        Ok(()) => tracing::info!(
            "[secrets] migrated {}/{} out of the OS keyring into the encrypted file store",
            ns.dir_name,
            account
        ),
        // Worth returning the value anyway: failing to cache it is not a reason
        // to tell the caller the secret is gone.
        Err(e) => tracing::warn!(
            "[secrets] could not copy {} out of the keyring: {}",
            account,
            e
        ),
    }
    Ok(Some(value))
}

pub fn delete_secret(ns: Namespace, account: &str) -> Result<(), String> {
    cache_put(ns, account, None);
    if let Some(dir) = forced_dir(ns) {
        return FileStore::new(dir).remove(account);
    }
    match keyring_delete(ns.service, account) {
        Some(Err(e)) => tracing::warn!("[secrets] keyring delete failed: {}", e),
        Some(Ok(())) => {}
        // Not writing to the keyring does not mean nothing of ours is in it:
        // a build from before the policy change may have put it there, and
        // "forget this instance" has to forget it everywhere.
        None => {
            if keyring_is_last_resort() {
                if let Err(e) = keyring_delete_raw(ns.service, account) {
                    tracing::warn!("[secrets] last-resort keyring delete failed: {}", e);
                }
            }
        }
    }
    FileStore::for_namespace(ns)?.remove(account)
}

/// Every account name the encrypted file store holds for a namespace. The
/// keyring cannot be enumerated, so this only sees what landed in the file —
/// which is what the profile migration needs, plus an explicit list of
/// candidate accounts for the keyring case.
pub fn file_store_accounts(ns: Namespace) -> Vec<String> {
    let store = match forced_dir(ns) {
        Some(dir) => FileStore::new(dir),
        None => match FileStore::for_namespace(ns) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        },
    };
    let mut names: Vec<String> = store
        .load()
        .map(|m| m.into_keys().collect())
        .unwrap_or_default();
    names.sort();
    names
}

// ---------------------------------------------------------------------------
// Keyring backends
// ---------------------------------------------------------------------------

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_entry(service: &str, account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, account).map_err(|e| e.to_string())
}

/// Read the OS keyring whether or not [`use_keyring`] says to write to it. The
/// gated wrapper below is what normal reads go through; this one exists for the
/// last-resort read in [`load_secret`].
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_get_raw(service: &str, account: &str) -> Result<Option<String>, String> {
    #[cfg(test)]
    if let Some(found) = test_keyring::intercept_read(service, account) {
        return Ok(found);
    }
    keyring_entry(service, account).and_then(|e| match e.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.to_string()),
    })
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_delete_raw(service: &str, account: &str) -> Result<(), String> {
    #[cfg(test)]
    if test_keyring::intercept_delete(service, account) {
        return Ok(());
    }
    keyring_entry(service, account).and_then(|e| match e.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.to_string()),
    })
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_set(service: &str, account: &str, value: &str) -> Option<Result<(), String>> {
    if !use_keyring() {
        return None;
    }
    #[cfg(test)]
    if test_keyring::intercept_write(service, account, value) {
        return Some(Ok(()));
    }
    Some(
        keyring_entry(service, account)
            .and_then(|e| e.set_password(value).map_err(|e| e.to_string())),
    )
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_get(service: &str, account: &str) -> Option<Result<Option<String>, String>> {
    if !use_keyring() {
        return None;
    }
    Some(keyring_get_raw(service, account))
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_delete(service: &str, account: &str) -> Option<Result<(), String>> {
    if !use_keyring() {
        return None;
    }
    Some(keyring_delete_raw(service, account))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn keyring_set(_service: &str, _account: &str, _value: &str) -> Option<Result<(), String>> {
    None
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn keyring_get(_service: &str, _account: &str) -> Option<Result<Option<String>, String>> {
    None
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn keyring_delete(_service: &str, _account: &str) -> Option<Result<(), String>> {
    None
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn keyring_get_raw(_service: &str, _account: &str) -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn keyring_delete_raw(_service: &str, _account: &str) -> Result<(), String> {
    Ok(())
}

/// Whether a build that does not *write* to the keyring should still *read* it
/// once before giving up.
///
/// WHY this exists: `use_keyring()` used to be `!debug_assertions`, so every
/// published macOS and Windows build put its secrets in the OS keyring. The
/// signature-based policy turns macOS off, and without this an update would
/// stop seeing the session token, the MLS state key and the `#device-key` that
/// the profile migration exists to preserve — the user would silently get a new
/// identity, orphaned from their MLS groups. So: keyring compiled in, keyring
/// not in use, and a release build (a `tauri dev` rebuild must not start asking
/// for the login password again).
pub fn keyring_is_last_resort() -> bool {
    keyring_last_resort_policy(KEYRING_COMPILED, use_keyring(), is_release_build())
}

/// The rule, as a pure function.
pub fn keyring_last_resort_policy(
    keyring_compiled: bool,
    keyring_in_use: bool,
    release_build: bool,
) -> bool {
    keyring_compiled && !keyring_in_use && release_build
}

fn is_release_build() -> bool {
    #[cfg(test)]
    if test_keyring::pretend_release() {
        return true;
    }
    !cfg!(debug_assertions)
}

/// A fake OS keyring, so a test can set up "the secret is only in the keychain"
/// without touching the developer's real login keychain — which would prompt
/// for a password and block forever. While a fake is installed, no call reaches
/// the real keyring at all.
#[cfg(test)]
pub mod test_keyring {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    #[derive(Default)]
    pub struct Fake {
        entries: HashMap<String, String>,
        /// Stands in for `!cfg!(debug_assertions)`: tests are debug builds, and
        /// the last-resort read is release-only.
        pretend_release: bool,
    }

    impl Fake {
        pub fn release() -> Self {
            Self {
                entries: HashMap::new(),
                pretend_release: true,
            }
        }

        pub fn with(mut self, service: &str, account: &str, value: &str) -> Self {
            self.entries
                .insert(key(service, account), value.to_string());
            self
        }
    }

    fn key(service: &str, account: &str) -> String {
        format!("{service}|{account}")
    }

    fn slot() -> &'static Mutex<Option<Fake>> {
        static SLOT: OnceLock<Mutex<Option<Fake>>> = OnceLock::new();
        SLOT.get_or_init(|| Mutex::new(None))
    }

    fn with_slot<T>(f: impl FnOnce(&mut Option<Fake>) -> T) -> T {
        let mut guard = slot().lock().unwrap_or_else(|p| p.into_inner());
        f(&mut guard)
    }

    /// Install a fake. Callers hold `test_env::EnvGuard`, which is the same
    /// global lock every environment-touching test takes, so two tests never
    /// see each other's fake.
    pub fn install(fake: Fake) {
        with_slot(|slot| *slot = Some(fake));
        super::clear_cache();
    }

    pub fn clear() {
        with_slot(|slot| *slot = None);
        super::clear_cache();
    }

    pub fn pretend_release() -> bool {
        with_slot(|slot| slot.as_ref().is_some_and(|f| f.pretend_release))
    }

    /// `None` means "no fake installed, go to the real keyring".
    pub fn intercept_read(service: &str, account: &str) -> Option<Option<String>> {
        with_slot(|slot| {
            slot.as_ref()
                .map(|f| f.entries.get(&key(service, account)).cloned())
        })
    }

    pub fn intercept_write(service: &str, account: &str, value: &str) -> bool {
        with_slot(|slot| match slot.as_mut() {
            Some(f) => {
                f.entries.insert(key(service, account), value.to_string());
                true
            }
            None => false,
        })
    }

    pub fn intercept_delete(service: &str, account: &str) -> bool {
        with_slot(|slot| match slot.as_mut() {
            Some(f) => {
                f.entries.remove(&key(service, account));
                true
            }
            None => false,
        })
    }

    pub fn holds(service: &str, account: &str) -> bool {
        intercept_read(service, account).flatten().is_some()
    }
}

// ---------------------------------------------------------------------------
// Encrypted file store
// ---------------------------------------------------------------------------

pub struct FileStore {
    dir: PathBuf,
}

impl FileStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn for_namespace(ns: Namespace) -> Result<Self, String> {
        Ok(Self::new(base_dir(ns)?))
    }

    pub fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.load()?.remove(account))
    }

    pub fn set(&self, account: &str, value: &str) -> Result<(), String> {
        let mut map = self.load()?;
        map.insert(account.to_string(), value.to_string());
        self.save(&map)
    }

    pub fn remove(&self, account: &str) -> Result<(), String> {
        let mut map = self.load()?;
        if map.remove(account).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }

    fn load(&self) -> Result<HashMap<String, String>, String> {
        let path = self.dir.join(SESSIONS_FILE);
        let blob = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(format!("secrets: could not read the secret store: {}", e)),
        };
        let key = self.key(false)?;
        let Some(key) = key else {
            return Ok(HashMap::new());
        };
        let plain = decrypt(&key, &blob)?;
        serde_json::from_slice(&plain)
            .map_err(|e| format!("secrets: the secret store is corrupt: {}", e))
    }

    fn save(&self, map: &HashMap<String, String>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("secrets: could not create the secret store dir: {}", e))?;
        let key = self
            .key(true)?
            .ok_or_else(|| "secrets: could not create the store key".to_string())?;
        let plain = serde_json::to_vec(map)
            .map_err(|e| format!("secrets: could not encode the secret store: {}", e))?;
        let blob = encrypt(&key, &plain);
        write_private(&self.dir.join(SESSIONS_FILE), &blob)
    }

    fn key(&self, create: bool) -> Result<Option<[u8; 32]>, String> {
        let path = self.dir.join(KEY_FILE);
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                Ok(Some(key))
            }
            Ok(_) => Err("secrets: the store key file has the wrong size".to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                let mut key = [0u8; 32];
                rand::Rng::fill_bytes(&mut rand::rng(), &mut key);
                write_private(&path, &key)?;
                Ok(Some(key))
            }
            Err(e) => Err(format!("secrets: could not read the store key: {}", e)),
        }
    }
}

/// Write a file only its owner can read. On unix the mode is part of the
/// `open` call: creating the file first and chmod'ing after leaves a window in
/// which the store key, the device key or the MLS state is world-readable.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let io = |e: std::io::Error| format!("secrets: could not write {}: {}", path.display(), e);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if let Some(dir) = path.parent() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).mode(0o600);
        let mut file = match opts.clone().create_new(true).open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Re-tighten before reopening: an older build may have left it
                // readable, and truncating does not change the mode.
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                    .map_err(io)?;
                opts.truncate(true).open(path).map_err(io)?
            }
            Err(e) => return Err(io(e)),
        };
        file.write_all(bytes).map_err(io)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(io)
    }
}

fn derive(key: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key);
    hasher.update(label);
    hasher.finalize().into()
}

pub(crate) fn encrypt(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let enc_key = derive(key, b"enc");
    let mac_key = derive(key, b"mac");
    let mut iv = [0u8; IV_LEN];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut iv);
    let mut buf = vec![0u8; plain.len() + 16];
    buf[..plain.len()].copy_from_slice(plain);
    let ct_len = Enc::new(&enc_key.into(), &iv.into())
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len())
        .map(|ct| ct.len())
        .unwrap_or(0);
    buf.truncate(ct_len);
    let mut out = Vec::with_capacity(IV_LEN + buf.len() + MAC_LEN);
    out.extend_from_slice(&iv);
    out.extend_from_slice(&buf);
    let mut mac = HmacSha256::new_from_slice(&mac_key).unwrap_or_else(|_| {
        HmacSha256::new_from_slice(b"omnidisc").expect("hmac accepts any key length")
    });
    mac.update(&out);
    out.extend_from_slice(&mac.finalize().into_bytes());
    out
}

pub(crate) fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < IV_LEN + MAC_LEN {
        return Err("secrets: the secret store is truncated".to_string());
    }
    let (body, tag) = blob.split_at(blob.len() - MAC_LEN);
    let mac_key = derive(key, b"mac");
    let mut mac = HmacSha256::new_from_slice(&mac_key).unwrap_or_else(|_| {
        HmacSha256::new_from_slice(b"omnidisc").expect("hmac accepts any key length")
    });
    mac.update(body);
    mac.verify_slice(tag)
        .map_err(|_| "secrets: the secret store failed its integrity check".to_string())?;
    let (iv, ct) = body.split_at(IV_LEN);
    let enc_key = derive(key, b"enc");
    let mut iv_arr = [0u8; IV_LEN];
    iv_arr.copy_from_slice(iv);
    let mut buf = ct.to_vec();
    Dec::new(&enc_key.into(), &iv_arr.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map(|pt| pt.to_vec())
        .map_err(|_| "secrets: the secret store could not be decrypted".to_string())
}

/// Environment juggling for tests. `std::env::set_var` is process-global and
/// cargo runs tests on threads, so every test that moves a secret directory
/// takes this lock and gets its variables restored on drop.
#[cfg(test)]
pub mod test_env {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    pub struct EnvGuard {
        _guard: MutexGuard<'static, ()>,
        previous: std::cell::RefCell<Vec<(String, Option<String>)>>,
    }

    impl EnvGuard {
        pub fn acquire() -> Self {
            let guard = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Self {
                _guard: guard,
                previous: std::cell::RefCell::new(Vec::new()),
            }
        }

        pub fn set(&self, key: &str, value: Option<&str>) {
            self.previous
                .borrow_mut()
                .push((key.to_string(), std::env::var(key).ok()));
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            super::clear_cache();
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.previous.borrow().iter().rev() {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            super::clear_cache();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_env::EnvGuard;
    use super::*;

    #[test]
    fn keyring_policy_covers_every_platform() {
        // Linux: nothing can turn it on without the dependency compiled in...
        assert!(!keyring_policy(
            Platform::Linux,
            Some(true),
            true,
            false,
            false
        ));
        // ...and with it, Secret Service is the default and the override still wins.
        assert!(keyring_policy(Platform::Linux, None, false, true, true));
        assert!(!keyring_policy(
            Platform::Linux,
            Some(false),
            false,
            false,
            true
        ));
        // macOS: follows the signature, not the build profile.
        assert!(!keyring_policy(Platform::Macos, None, false, false, true));
        assert!(keyring_policy(Platform::Macos, None, true, true, true));
        // ...and the override wins in both directions.
        assert!(keyring_policy(
            Platform::Macos,
            Some(true),
            false,
            true,
            true
        ));
        assert!(!keyring_policy(
            Platform::Macos,
            Some(false),
            true,
            false,
            true
        ));
        // Windows: release only, as before.
        assert!(keyring_policy(Platform::Windows, None, false, false, true));
        assert!(!keyring_policy(Platform::Windows, None, false, true, true));
        assert!(!keyring_policy(Platform::Other, None, true, false, true));
    }

    /// The read-only step that keeps an update from losing secrets an older
    /// build wrote to the OS keyring.
    #[test]
    fn the_last_resort_read_is_release_only_and_never_when_the_keyring_is_in_use() {
        // The case that matters: release macOS, unsigned, so the policy says
        // "do not write to the keyring" — but a previous version did.
        assert!(keyring_last_resort_policy(true, false, true));
        // A dev build must not start prompting for the login password again.
        assert!(!keyring_last_resort_policy(true, false, false));
        // Already reading the keyring the normal way: nothing to salvage.
        assert!(!keyring_last_resort_policy(true, true, true));
        // No keyring compiled in: nothing to ask.
        assert!(!keyring_last_resort_policy(false, false, true));
    }

    /// The regression the Phase 1 verifier caught: with `use_keyring()` false,
    /// a session token, an MLS state key or a `#device-key` already sitting in
    /// the login keychain has to still be found — once — and then live in the
    /// file store.
    #[test]
    fn a_secret_left_in_the_keyring_is_found_once_and_then_read_from_the_file() {
        let root = temp_dir("last-resort");
        let guard = EnvGuard::acquire();
        guard.set("OMNIGET_DATA_DIR", Some(root.to_str().expect("utf-8 path")));
        guard.set(SECRETS_DIR_ENV, None);
        guard.set(OMNIDISC.dir_env, None);
        guard.set(PROFILE.dir_env, None);
        guard.set(KEYRING_ENV, Some("0"));
        guard.set(KEYRING_ENV_LEGACY, None);

        let account = "https://a.example";
        test_keyring::install(test_keyring::Fake::release().with(
            OMNIDISC.service,
            account,
            "od1.left-in-the-keychain",
        ));

        assert!(!use_keyring(), "the policy has to be off for this test");
        assert!(keyring_is_last_resort());

        // Nothing on disk yet, and the value still comes back.
        assert!(file_store_accounts(OMNIDISC).is_empty());
        assert_eq!(
            load_secret(OMNIDISC, account).unwrap().as_deref(),
            Some("od1.left-in-the-keychain")
        );
        // ...and it was copied, so this is a one-time keyring call.
        assert_eq!(file_store_accounts(OMNIDISC), vec![account.to_string()]);

        // Take the entry out of the keyring entirely: the second read no longer
        // needs it.
        clear_cache();
        test_keyring::install(test_keyring::Fake::release());
        assert_eq!(
            load_secret(OMNIDISC, account).unwrap().as_deref(),
            Some("od1.left-in-the-keychain")
        );

        // Forgetting has to forget both copies.
        test_keyring::install(test_keyring::Fake::release().with(
            OMNIDISC.service,
            account,
            "od1.left-in-the-keychain",
        ));
        delete_secret(OMNIDISC, account).unwrap();
        assert!(!test_keyring::holds(OMNIDISC.service, account));
        clear_cache();
        assert_eq!(load_secret(OMNIDISC, account).unwrap(), None);

        test_keyring::clear();
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `tauri dev` rebuild must not reach for the keychain, which is exactly
    /// the prompt loop the old `!debug_assertions` rule existed to avoid.
    #[test]
    fn a_debug_build_does_not_reach_for_the_keyring_at_all() {
        let root = temp_dir("debug-build");
        let guard = EnvGuard::acquire();
        guard.set("OMNIGET_DATA_DIR", Some(root.to_str().expect("utf-8 path")));
        guard.set(SECRETS_DIR_ENV, None);
        guard.set(OMNIDISC.dir_env, None);
        guard.set(KEYRING_ENV, Some("0"));
        guard.set(KEYRING_ENV_LEGACY, None);

        // `Fake::default()` is the debug case: pretend_release stays false.
        test_keyring::install(test_keyring::Fake::default().with(
            OMNIDISC.service,
            "https://a.example",
            "od1.invisible",
        ));
        assert!(!keyring_is_last_resort());
        assert_eq!(load_secret(OMNIDISC, "https://a.example").unwrap(), None);

        test_keyring::clear();
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn bool_env_only_answers_when_it_is_an_answer() {
        for yes in ["1", "true", "YES", " on "] {
            assert_eq!(parse_bool_env(yes), Some(true), "{yes}");
        }
        for no in ["0", "false", "No", "off"] {
            assert_eq!(parse_bool_env(no), Some(false), "{no}");
        }
        for neither in ["", "maybe", "2"] {
            assert_eq!(parse_bool_env(neither), None, "{neither}");
        }
    }

    #[test]
    fn adhoc_signatures_are_not_stable() {
        let adhoc = "Executable=/tmp/omniget\nIdentifier=omniget\nSignature=adhoc\nTeamIdentifier=not set\n";
        assert!(!codesign_output_is_stable(adhoc));

        let unsigned = "/tmp/omniget: code object is not signed at all\n";
        assert!(!codesign_output_is_stable(unsigned));

        let developer_id = concat!(
            "Executable=/Applications/OmniGet.app/Contents/MacOS/omniget\n",
            "Identifier=wtf.tonho.omniget\n",
            "Signature size=9000\n",
            "Authority=Developer ID Application: Antonio (AB12CD34EF)\n",
            "Authority=Developer ID Certification Authority\n",
            "Authority=Apple Root CA\n",
            "TeamIdentifier=AB12CD34EF\n"
        );
        assert!(codesign_output_is_stable(developer_id));

        // A team id with no Apple authority behind it is not enough.
        assert!(!codesign_output_is_stable(
            "TeamIdentifier=AB12CD34EF\nAuthority=Self-signed\n"
        ));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-secrets-{}-{}-{}",
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

    #[test]
    fn namespaces_do_not_share_a_folder_under_one_root() {
        let root = temp_dir("namespaces");
        let guard = EnvGuard::acquire();
        guard.set(SECRETS_DIR_ENV, Some(root.to_str().expect("utf-8 path")));
        guard.set(OMNIDISC.dir_env, None);
        guard.set(PROFILE.dir_env, None);

        assert_eq!(base_dir(OMNIDISC).unwrap(), root.join("omnidisc"));
        assert_eq!(base_dir(PROFILE).unwrap(), root.join("profile"));

        save_secret(PROFILE, "profile-seed", "seed-value").unwrap();
        save_secret(OMNIDISC, "profile-seed", "token-value").unwrap();
        clear_cache();
        assert_eq!(
            load_secret(PROFILE, "profile-seed").unwrap().as_deref(),
            Some("seed-value")
        );
        assert_eq!(
            load_secret(OMNIDISC, "profile-seed").unwrap().as_deref(),
            Some("token-value")
        );
        assert_eq!(
            file_store_accounts(PROFILE),
            vec!["profile-seed".to_string()]
        );

        delete_secret(PROFILE, "profile-seed").unwrap();
        clear_cache();
        assert_eq!(load_secret(PROFILE, "profile-seed").unwrap(), None);
        assert!(file_store_accounts(PROFILE).is_empty());

        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The seam the core uses for provider keys: the namespace string has to
    /// land in the AI_KEYS folder, and an unknown one has to be refused instead
    /// of silently sharing a store with somebody else.
    #[test]
    fn the_core_seam_resolves_namespaces_by_name() {
        use omniget_core::core::secrets::SecretStore;

        let root = temp_dir("seam");
        let guard = EnvGuard::acquire();
        guard.set(SECRETS_DIR_ENV, Some(root.to_str().expect("utf-8 path")));
        guard.set(AI_KEYS.dir_env, None);

        let store = AppSecretStore;
        assert!(store.get("nope", "x").is_err());
        store
            .set(omniget_core::core::secrets::AI_KEYS, "openai:1", "sk-live")
            .unwrap();
        assert_eq!(base_dir(AI_KEYS).unwrap(), root.join("ai_keys"));
        assert_eq!(
            store
                .get(omniget_core::core::secrets::AI_KEYS, "openai:1")
                .unwrap()
                .as_deref(),
            Some("sk-live")
        );
        clear_cache();
        assert_eq!(file_store_accounts(AI_KEYS), vec!["openai:1".to_string()]);
        store
            .delete(omniget_core::core::secrets::AI_KEYS, "openai:1")
            .unwrap();
        clear_cache();
        assert_eq!(
            store
                .get(omniget_core::core::secrets::AI_KEYS, "openai:1")
                .unwrap(),
            None
        );

        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A synthetic MCP token goes in, comes back and goes away through the
    /// same seam the registry uses, inside a temporary directory and apart
    /// from the provider keys. The forced directory keeps the keyring out.
    #[test]
    fn an_mcp_token_lives_in_its_own_folder_and_can_be_forgotten() {
        use omniget_core::core::mcp::types::SECRET_NS;
        use omniget_core::core::secrets::SecretStore;

        let root = temp_dir("mcp");
        let guard = EnvGuard::acquire();
        guard.set(SECRETS_DIR_ENV, Some(root.to_str().expect("utf-8 path")));
        guard.set(MCP.dir_env, None);
        guard.set(AI_KEYS.dir_env, None);

        let store = AppSecretStore;
        let account = "fixture-header-authorization";
        store
            .set(SECRET_NS, account, "Bearer synthetic-token")
            .unwrap();
        assert_eq!(base_dir(MCP).unwrap(), root.join("mcp"));
        assert!(root.join("mcp").join(SESSIONS_FILE).is_file());
        clear_cache();
        assert_eq!(
            store.get(SECRET_NS, account).unwrap().as_deref(),
            Some("Bearer synthetic-token")
        );
        assert_eq!(file_store_accounts(MCP), vec![account.to_string()]);
        // Not visible under another namespace, not written into its folder.
        assert_eq!(
            store
                .get(omniget_core::core::secrets::AI_KEYS, account)
                .unwrap(),
            None
        );
        assert!(file_store_accounts(AI_KEYS).is_empty());

        store.delete(SECRET_NS, account).unwrap();
        clear_cache();
        assert_eq!(store.get(SECRET_NS, account).unwrap(), None);
        assert!(file_store_accounts(MCP).is_empty());

        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_namespace_variable_beats_the_shared_one() {
        let root = temp_dir("shared");
        let own = temp_dir("own");
        let guard = EnvGuard::acquire();
        guard.set(SECRETS_DIR_ENV, Some(root.to_str().expect("utf-8 path")));
        guard.set(OMNIDISC.dir_env, Some(own.to_str().expect("utf-8 path")));

        // Literal, not joined: the OmniDisc e2e tests point at exactly the
        // directory they created.
        assert_eq!(base_dir(OMNIDISC).unwrap(), own);
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&own);
    }
}

/// Pure resolver audit: no filesystem, keyring, credential or network access.
#[cfg(test)]
mod settings_rebrand_audit {
    use super::*;

    #[test]
    fn the_desktop_resolver_keeps_known_namespaces_distinct() {
        assert_eq!(AppSecretStore::resolve("ai_keys").unwrap(), AI_KEYS);
        assert_eq!(AppSecretStore::resolve("profile").unwrap(), PROFILE);
        assert_eq!(AppSecretStore::resolve("omnidisc").unwrap(), OMNIDISC);
        assert!(AppSecretStore::resolve("audit-unknown-namespace").is_err());
        assert_ne!(AI_KEYS.service, PROFILE.service);
        assert_ne!(AI_KEYS.dir_name, OMNIDISC.dir_name);
    }

    /// The MCP registry names this namespace over the seam; the desktop has to
    /// resolve it to a store of its own instead of refusing it.
    #[test]
    fn the_mcp_registry_namespace_is_supported_by_the_desktop_resolver() {
        let name = omniget_core::core::mcp::types::SECRET_NS;
        let namespace = AppSecretStore::resolve(name)
            .expect("desktop must support the namespace used by its MCP registry");
        assert_eq!(namespace.dir_name, name);
        assert_ne!(namespace.dir_name, AI_KEYS.dir_name);
        assert_ne!(namespace.service, AI_KEYS.service);
    }
}
