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

pub const SECRETS_DIR_ENV: &str = "OMNIGET_SECRETS_DIR";
pub const KEYRING_ENV: &str = "OMNIGET_KEYRING";
pub const KEYRING_ENV_LEGACY: &str = "OMNIGET_OMNIDISC_KEYRING";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Namespace {
    pub service: &'static str,
    pub dir_name: &'static str,
    pub dir_env: &'static str,
}

pub const OMNIDISC: Namespace = Namespace {
    service: "wtf.tonho.omniget.omnidisc",
    dir_name: "omnidisc",
    dir_env: "OMNIGET_OMNIDISC_SESSION_DIR",
};

pub const PROFILE: Namespace = Namespace {
    service: "wtf.tonho.omniget.profile",
    dir_name: "profile",
    dir_env: "OMNIGET_PROFILE_SECRET_DIR",
};

pub const AI_KEYS: Namespace = Namespace {
    service: "wtf.tonho.omniget.ai-keys",
    dir_name: "ai_keys",
    dir_env: "OMNIGET_AI_KEYS_SECRET_DIR",
};

pub const READ: Namespace = Namespace {
    service: "wtf.tonho.omniget.read",
    dir_name: "read",
    dir_env: "OMNIGET_READ_SECRET_DIR",
};

pub const MCP: Namespace = Namespace {
    service: "wtf.tonho.omniget.mcp",
    dir_name: "mcp",
    dir_env: "OMNIGET_MCP_SECRET_DIR",
};

pub const TRANSFER: Namespace = Namespace {
    service: "wtf.tonho.omniget.transfer",
    dir_name: "transfer",
    dir_env: "OMNIGET_TRANSFER_SECRET_DIR",
};

pub const RESEARCH: Namespace = Namespace {
    service: "wtf.tonho.omniget.research",
    dir_name: "research",
    dir_env: "OMNIGET_RESEARCH_SECRET_DIR",
};

pub fn namespace_by_name(name: &str) -> Option<Namespace> {
    match name {
        n if n == RESEARCH.dir_name => Some(RESEARCH),
        n if n == TRANSFER.dir_name => Some(TRANSFER),
        n if n == OMNIDISC.dir_name => Some(OMNIDISC),
        n if n == PROFILE.dir_name => Some(PROFILE),
        n if n == AI_KEYS.dir_name => Some(AI_KEYS),
        n if n == MCP.dir_name => Some(MCP),
        _ => None,
    }
}

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

pub const KEYRING_COMPILED: bool = cfg!(any(target_os = "macos", windows, target_os = "linux"));

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
        Platform::Macos => codesign_stable,
        Platform::Windows => !debug_build,
        Platform::Linux => true,
        Platform::Other => false,
    }
}

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

pub fn use_keyring() -> bool {
    keyring_policy(
        Platform::current(),
        keyring_env_override(),
        codesign_is_stable(),
        cfg!(debug_assertions),
        KEYRING_COMPILED,
    )
}

fn cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

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

pub fn forced_dir(ns: Namespace) -> Option<PathBuf> {
    env_dir(ns.dir_env).or_else(|| env_dir(SECRETS_DIR_ENV).map(|d| d.join(ns.dir_name)))
}

pub fn base_dir(ns: Namespace) -> Result<PathBuf, String> {
    if let Some(dir) = forced_dir(ns) {
        return Ok(dir);
    }
    let base = crate::core::paths::app_data_dir()
        .ok_or_else(|| "secrets: could not resolve the app data directory".to_string())?;
    Ok(base.join(ns.dir_name))
}

pub fn secret_account(url: &str, kind: &str) -> String {
    format!("{}#{}", url, kind)
}

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
        None => adopt_from_keyring(ns, account, &store)?,
    };
    cache_put(ns, account, found.clone());
    Ok(found)
}

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

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn keyring_entry(service: &str, account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, account).map_err(|e| e.to_string())
}

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

pub fn keyring_is_last_resort() -> bool {
    keyring_last_resort_policy(KEYRING_COMPILED, use_keyring(), is_release_build())
}

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

#[cfg(test)]
pub mod test_keyring {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    #[derive(Default)]
    pub struct Fake {
        entries: HashMap<String, String>,
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

/// Serializes the load -> modify -> save cycle of every FileStore.
fn file_store_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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
        let _guard = file_store_lock();
        let mut map = self.load()?;
        map.insert(account.to_string(), value.to_string());
        self.save(&map)
    }

    pub fn remove(&self, account: &str) -> Result<(), String> {
        let _guard = file_store_lock();
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
        write_private_atomic(&self.dir.join(SESSIONS_FILE), &blob)
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

/// Writes to a unique 0600 temp file in the same dir, fsyncs it and renames it
/// over `path`, so an interrupted write never leaves a truncated file behind.
fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let io = |e: std::io::Error| format!("secrets: could not write {}: {}", path.display(), e);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        write_private(&tmp, bytes)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
    #[test]
    fn file_store_concurrent_sets_keep_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    super::FileStore::new(path)
                        .set(&format!("acct-{i}"), "v")
                        .unwrap()
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let store = super::FileStore::new(path.clone());
        for i in 0..16 {
            assert_eq!(
                store.get(&format!("acct-{i}")).unwrap().as_deref(),
                Some("v")
            );
        }
        let leftovers = std::fs::read_dir(&path)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    use super::test_env::EnvGuard;
    use super::*;

    #[test]
    fn keyring_policy_covers_every_platform() {
        assert!(!keyring_policy(
            Platform::Linux,
            Some(true),
            true,
            false,
            false
        ));
        assert!(keyring_policy(Platform::Linux, None, false, true, true));
        assert!(!keyring_policy(
            Platform::Linux,
            Some(false),
            false,
            false,
            true
        ));
        assert!(!keyring_policy(Platform::Macos, None, false, false, true));
        assert!(keyring_policy(Platform::Macos, None, true, true, true));
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
        assert!(keyring_policy(Platform::Windows, None, false, false, true));
        assert!(!keyring_policy(Platform::Windows, None, false, true, true));
        assert!(!keyring_policy(Platform::Other, None, true, false, true));
    }

    #[test]
    fn the_last_resort_read_is_release_only_and_never_when_the_keyring_is_in_use() {
        assert!(keyring_last_resort_policy(true, false, true));
        assert!(!keyring_last_resort_policy(true, false, false));
        assert!(!keyring_last_resort_policy(true, true, true));
        assert!(!keyring_last_resort_policy(false, false, true));
    }

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

        assert!(file_store_accounts(OMNIDISC).is_empty());
        assert_eq!(
            load_secret(OMNIDISC, account).unwrap().as_deref(),
            Some("od1.left-in-the-keychain")
        );
        assert_eq!(file_store_accounts(OMNIDISC), vec![account.to_string()]);

        clear_cache();
        test_keyring::install(test_keyring::Fake::release());
        assert_eq!(
            load_secret(OMNIDISC, account).unwrap().as_deref(),
            Some("od1.left-in-the-keychain")
        );

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

    #[test]
    fn a_debug_build_does_not_reach_for_the_keyring_at_all() {
        let root = temp_dir("debug-build");
        let guard = EnvGuard::acquire();
        guard.set("OMNIGET_DATA_DIR", Some(root.to_str().expect("utf-8 path")));
        guard.set(SECRETS_DIR_ENV, None);
        guard.set(OMNIDISC.dir_env, None);
        guard.set(KEYRING_ENV, Some("0"));
        guard.set(KEYRING_ENV_LEGACY, None);

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

        assert_eq!(base_dir(OMNIDISC).unwrap(), own);
        drop(guard);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&own);
    }
}

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
