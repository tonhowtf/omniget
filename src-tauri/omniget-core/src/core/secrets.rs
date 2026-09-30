//! `SecretStore` seam: the core asks for secrets through this trait; the app
//! installs the keychain-backed implementation at boot (`src-tauri/src/secrets.rs`)
//! and everything else — the CLI, `cargo test -p omniget-core`, a headless run —
//! falls back to [`FileSecretStore`], an encrypted file under the app data
//! directory.
//!
//! A namespace is a plain string here instead of the app's `Namespace` struct,
//! because `omniget-core` must not depend on the app crate. The app maps the
//! string back onto its own table; [`AI_KEYS`] is the only one the core uses
//! today.
//!
//! The file default is AES-256-CBC + HMAC-SHA256 with a random key written next
//! to it at 0600 — the same floor the app's file store gives, so a secret never
//! sits in plain text in a backup or a synced folder. It is not a keyring:
//! anyone running as the same OS user can read the key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Provider API keys: the `ai-keys.json` vault and the app's own AI config.
pub const AI_KEYS: &str = "ai_keys";

/// Forces every namespace into `<dir>/<namespace>`; same variable the app's
/// store honours, so one temporary directory covers both sides in a test.
pub const SECRETS_DIR_ENV: &str = "OMNIGET_SECRETS_DIR";

const STORE_FILE: &str = "secrets.bin";
const KEY_FILE: &str = "secrets.key";
const IV_LEN: usize = 16;
const MAC_LEN: usize = 32;

type Enc = cbc::Encryptor<aes::Aes256>;
type Dec = cbc::Decryptor<aes::Aes256>;
type HmacSha256 = Hmac<Sha256>;

pub trait SecretStore: Send + Sync {
    fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String>;
    fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, namespace: &str, account: &str) -> Result<(), String>;
}

static STORE: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();

/// Installs the process-wide store. The first call wins; later calls are ignored.
pub fn install(store: Arc<dyn SecretStore>) {
    let _ = STORE.set(store);
}

pub fn installed() -> Option<Arc<dyn SecretStore>> {
    STORE.get().cloned()
}

/// Read a secret. WHY it never fails loudly on a missing store: a secret that
/// cannot be read is the same to every caller as a secret that is not there.
pub fn get(namespace: &str, account: &str) -> Result<Option<String>, String> {
    match installed() {
        Some(store) => store.get(namespace, account),
        None => FileSecretStore::default_store().get(namespace, account),
    }
}

pub fn set(namespace: &str, account: &str, value: &str) -> Result<(), String> {
    match installed() {
        Some(store) => store.set(namespace, account, value),
        None => FileSecretStore::default_store().set(namespace, account, value),
    }
}

pub fn delete(namespace: &str, account: &str) -> Result<(), String> {
    match installed() {
        Some(store) => store.delete(namespace, account),
        None => FileSecretStore::default_store().delete(namespace, account),
    }
}

/// `set` with an empty value means "forget it": an entry whose key was cleared
/// must not leave the old secret behind.
pub fn put_or_delete(namespace: &str, account: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        delete(namespace, account)
    } else {
        set(namespace, account, value)
    }
}

/// Secrets the desktop app and the CLI both need (the Microsoft 365 sign-in
/// that `omniget teams` and the Meetings screen share). The app's keychain is
/// invisible to the CLI, so these always live in the encrypted file store.
pub const SHARED: &str = "shared";

pub fn shared_get(account: &str) -> Result<Option<String>, String> {
    FileSecretStore::default_store().get(SHARED, account)
}

pub fn shared_set(account: &str, value: &str) -> Result<(), String> {
    FileSecretStore::default_store().set(SHARED, account, value)
}

pub fn shared_delete(account: &str) -> Result<(), String> {
    FileSecretStore::default_store().delete(SHARED, account)
}

// ---------------------------------------------------------------------------
// File implementation (the default outside the app)
// ---------------------------------------------------------------------------

pub struct FileSecretStore {
    root: PathBuf,
}

impl FileSecretStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The store used when nothing was installed. Not cached: `OMNIGET_SECRETS_DIR`
    /// moves between tests in one process, and resolving a path is cheap next to
    /// the file read that follows it.
    fn default_store() -> Self {
        Self::new(default_root())
    }

    fn dir(&self, namespace: &str) -> PathBuf {
        self.root.join(namespace)
    }

    fn load(&self, namespace: &str) -> Result<HashMap<String, String>, String> {
        let dir = self.dir(namespace);
        let blob = match std::fs::read(dir.join(STORE_FILE)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(format!("secrets: could not read the secret store: {}", e)),
        };
        let Some(key) = read_key(&dir, false)? else {
            return Ok(HashMap::new());
        };
        let plain = decrypt(&key, &blob)?;
        serde_json::from_slice(&plain)
            .map_err(|e| format!("secrets: the secret store is corrupt: {}", e))
    }

    fn save(&self, namespace: &str, map: &HashMap<String, String>) -> Result<(), String> {
        let dir = self.dir(namespace);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("secrets: could not create the secret store dir: {}", e))?;
        let key = read_key(&dir, true)?
            .ok_or_else(|| "secrets: could not create the store key".to_string())?;
        let plain = serde_json::to_vec(map)
            .map_err(|e| format!("secrets: could not encode the secret store: {}", e))?;
        write_private(&dir.join(STORE_FILE), &encrypt(&key, &plain))
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, namespace: &str, account: &str) -> Result<Option<String>, String> {
        Ok(self.load(namespace)?.remove(account))
    }

    fn set(&self, namespace: &str, account: &str, value: &str) -> Result<(), String> {
        let mut map = self.load(namespace)?;
        map.insert(account.to_string(), value.to_string());
        self.save(namespace, &map)
    }

    fn delete(&self, namespace: &str, account: &str) -> Result<(), String> {
        let mut map = self.load(namespace)?;
        if map.remove(account).is_some() {
            self.save(namespace, &map)?;
        }
        Ok(())
    }
}

fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var(SECRETS_DIR_ENV)
        .ok()
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
    {
        return PathBuf::from(dir);
    }
    crate::core::paths::app_data_dir()
        .map(|d| d.join("secrets"))
        .unwrap_or_else(|| std::env::temp_dir().join("omniget-secrets"))
}

fn read_key(dir: &Path, create: bool) -> Result<Option<[u8; 32]>, String> {
    let path = dir.join(KEY_FILE);
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

/// Write a file only its owner can read. On unix the mode is part of the `open`
/// call: creating the file and chmod'ing after leaves a window in which the key
/// material — or the `.bak` the migration writes — is world-readable.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let io = |e: std::io::Error| format!("secrets: could not write {}: {}", path.display(), e);
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
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
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(path, bytes).map_err(io)
    }
}

fn derive(key: &[u8; 32], label: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(key);
    hasher.update(label);
    hasher.finalize().into()
}

fn mac_of(key: &[u8; 32]) -> HmacSha256 {
    HmacSha256::new_from_slice(&derive(key, b"mac")).unwrap_or_else(|_| {
        HmacSha256::new_from_slice(b"omniget").expect("hmac accepts any key length")
    })
}

fn encrypt(key: &[u8; 32], plain: &[u8]) -> Vec<u8> {
    let enc_key = derive(key, b"enc");
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
    let mut mac = mac_of(key);
    mac.update(&out);
    out.extend_from_slice(&mac.finalize().into_bytes());
    out
}

fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() < IV_LEN + MAC_LEN {
        return Err("secrets: the secret store is truncated".to_string());
    }
    let (body, tag) = blob.split_at(blob.len() - MAC_LEN);
    let mut mac = mac_of(key);
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
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-core-secrets-{}-{}-{}",
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
    fn a_file_store_round_trips_and_keeps_namespaces_apart() {
        let root = temp_dir("roundtrip");
        let store = FileSecretStore::new(&root);

        assert_eq!(store.get(AI_KEYS, "openai:1").unwrap(), None);
        store.set(AI_KEYS, "openai:1", "sk-secret").unwrap();
        store.set("other", "openai:1", "other-secret").unwrap();
        assert_eq!(
            store.get(AI_KEYS, "openai:1").unwrap().as_deref(),
            Some("sk-secret")
        );
        assert_eq!(
            store.get("other", "openai:1").unwrap().as_deref(),
            Some("other-secret")
        );

        // Nothing readable on disk.
        let blob = std::fs::read(root.join(AI_KEYS).join(STORE_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("sk-secret"));

        store.delete(AI_KEYS, "openai:1").unwrap();
        assert_eq!(store.get(AI_KEYS, "openai:1").unwrap(), None);
        assert_eq!(
            store.get("other", "openai:1").unwrap().as_deref(),
            Some("other-secret")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tampered_store_fails_its_integrity_check() {
        let key = [7u8; 32];
        let mut blob = encrypt(&key, b"{\"a\":\"b\"}");
        let n = blob.len();
        blob[n - 1] ^= 0xff;
        assert!(decrypt(&key, &blob).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_dir("modes");
        let path = root.join("a.bak");
        write_private(&path, b"secret").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode);
        // Rewriting an existing, loose file re-tightens it.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"secret2").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode);
        let _ = std::fs::remove_dir_all(&root);
    }
}
