use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::connector::{self, Connector, Manifest};
use super::data_dir;
use crate::core::catalog::{sha256_hex, write_atomic};
use crate::core::platform_optout::{is_opted_out, OPTED_OUT_ERROR};

pub const CATALOG_URL: &str = "https://getomniget.com/connectors/catalog.json";
pub const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVBMUREMkJDOERBOEJCOEEKUldTS3U2aU52TklkV2s5UW9LcXNyVEt3MnhLSVpwaWRjVnF0aU1yd29WeVdFSVlOMm5jRGxGMVkK";
pub const PACKAGE_EXT: &str = "omniconnector";

const MAX_CATALOG_BYTES: usize = 2 << 20;
const MAX_SIGNATURE_BYTES: usize = 8 << 10;
const MAX_PACKAGE_BYTES: usize = 8 << 20;
const MAX_PACKAGE_FILES: usize = 32;
const MAX_FILE_BYTES: u64 = 4 << 20;
const MAX_TOTAL_BYTES: u64 = 16 << 20;
const MAX_DESCRIPTION_LOCALES: usize = 32;
const MAX_DESCRIPTION_CHARS: usize = 1000;
const MAX_LOCALE_BYTES: usize = 16;
const CACHE_FRESH: Duration = Duration::from_secs(6 * 60 * 60);

const ERR_SIGNATURE: &str = "signature";
const ERR_HASH: &str = "hash";
const ERR_MISMATCH: &str = "mismatch";
const ERR_PACKAGE: &str = "package";
const ERR_NETWORK: &str = "network";
const ERR_STALE: &str = "stale";
const ERR_NOT_FOUND: &str = "not_found";

static SWAP: Mutex<()> = Mutex::new(());

fn fail(code: &str, message: impl std::fmt::Display) -> String {
    format!("{code}: {message}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoginInfo {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(
        default,
        deserialize_with = "description_i18n",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub description_i18n: BTreeMap<String, String>,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    pub domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<LoginInfo>,
    #[serde(default)]
    pub min_omniget: String,
    pub sha256: String,
    pub package: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub permissions: Vec<String>,
}

fn description_i18n<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let serde_json::Value::Object(raw) = serde_json::Value::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    Ok(raw
        .into_iter()
        .filter_map(|(locale, text)| {
            let serde_json::Value::String(text) = text else {
                return None;
            };
            let locale_ok = !locale.is_empty()
                && locale.len() <= MAX_LOCALE_BYTES
                && locale
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-');
            let text_ok = !text.trim().is_empty() && text.chars().count() <= MAX_DESCRIPTION_CHARS;
            (locale_ok && text_ok).then_some((locale, text))
        })
        .take(MAX_DESCRIPTION_LOCALES)
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub generated_at: u64,
    pub connectors: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketEntry {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub state: &'static str,
    pub installed_version: Option<String>,
    pub dev: bool,
    pub in_catalog: bool,
    pub compatible: bool,
    pub has_options: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MarketView {
    pub entries: Vec<MarketEntry>,
    pub invalid: Vec<connector::InvalidConnector>,
    pub source: &'static str,
    pub fetched_at: Option<u64>,
    pub error: Option<String>,
    pub rejected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UrlLookup {
    pub opted_out: bool,
    pub installed: Option<InstalledMatch>,
    pub catalog: Option<MarketEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstalledMatch {
    pub id: String,
    pub name: String,
    pub login_url: Option<String>,
    pub dev: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub domains: Vec<String>,
    pub permissions: Vec<String>,
    pub replaces_version: Option<String>,
}

pub fn verify_signature(data: &[u8], signature_b64: &str, pubkey_b64: &str) -> Result<(), String> {
    let decode = |text: &str, what: &str| -> Result<String, String> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .map_err(|e| fail(ERR_SIGNATURE, format!("{what} is not base64: {e}")))?;
        String::from_utf8(bytes).map_err(|e| fail(ERR_SIGNATURE, format!("{what}: {e}")))
    };
    let key = minisign_verify::PublicKey::decode(&decode(pubkey_b64, "public key")?)
        .map_err(|e| fail(ERR_SIGNATURE, format!("public key: {e}")))?;
    let signature = minisign_verify::Signature::decode(&decode(signature_b64, "signature")?)
        .map_err(|e| fail(ERR_SIGNATURE, format!("signature: {e}")))?;
    key.verify(data, &signature, false).map_err(|e| {
        fail(
            ERR_SIGNATURE,
            format!("the catalog signature is not valid: {e}"),
        )
    })
}

fn entry_manifest(e: &CatalogEntry) -> Manifest {
    Manifest {
        id: e.id.clone(),
        name: e.name.clone(),
        version: e.version.clone(),
        author: e.author.clone(),
        license: String::new(),
        homepage: String::new(),
        description: e.description.clone(),
        domains: e.domains.clone(),
        matches: e.matches.clone(),
        login: e.login.as_ref().map(|l| json!({ "url": l.url })),
        entry: "connector.js".into(),
        min_omniget: e.min_omniget.clone(),
        options: Vec::new(),
        permissions: e.permissions.clone(),
        category: e.category.clone(),
    }
}

fn validate_entry(e: &CatalogEntry) -> Result<(), String> {
    connector::validate_manifest(&entry_manifest(e))?;
    let url = url::Url::parse(&e.package).map_err(|err| format!("package url: {err}"))?;
    if url.scheme() != "https" {
        return Err("the package url must be https".into());
    }
    if e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("sha256 must be 64 hex characters".into());
    }
    Ok(())
}

pub fn parse_catalog(bytes: &[u8]) -> Result<(Catalog, Vec<String>), String> {
    let mut catalog: Catalog = serde_json::from_slice(bytes)
        .map_err(|e| fail(ERR_PACKAGE, format!("catalog.json: {e}")))?;
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();
    catalog.connectors.retain(|e| match validate_entry(e) {
        Ok(()) if seen.insert(e.id.clone()) => true,
        Ok(()) => {
            rejected.push(format!("{}: listed twice", e.id));
            false
        }
        Err(err) => {
            tracing::warn!("[global-dl] catalog entry {} refused: {err}", e.id);
            rejected.push(format!("{}: {err}", e.id));
            false
        }
    });
    catalog.connectors.sort_by_key(|a| a.name.to_lowercase());
    Ok((catalog, rejected))
}

pub fn verify_catalog(
    bytes: &[u8],
    signature_b64: &str,
    pubkey_b64: &str,
) -> Result<(Catalog, Vec<String>), String> {
    verify_signature(bytes, signature_b64, pubkey_b64)?;
    parse_catalog(bytes)
}

pub fn verify_package_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(fail(
            ERR_HASH,
            format!("the package does not match the catalog (expected {expected}, got {got})"),
        ))
    }
}

pub fn manifest_matches_entry(m: &Manifest, e: &CatalogEntry) -> Result<(), String> {
    let set = |v: &[String]| -> Vec<String> {
        let mut out: Vec<String> = v.iter().map(|d| d.trim().to_ascii_lowercase()).collect();
        out.sort();
        out
    };
    let differs = |what: &str, have: &str, want: &str| {
        fail(
            ERR_MISMATCH,
            format!("the package says {what} is {have}, the catalog says {want}"),
        )
    };
    if m.id != e.id {
        return Err(differs("the id", &m.id, &e.id));
    }
    if m.version != e.version {
        return Err(differs("the version", &m.version, &e.version));
    }
    if set(&m.domains) != set(&e.domains) {
        return Err(differs(
            "the domains",
            &m.domains.join(", "),
            &e.domains.join(", "),
        ));
    }
    if set(&m.matches) != set(&e.matches) {
        return Err(differs(
            "the URL patterns",
            &m.matches.join(", "),
            &e.matches.join(", "),
        ));
    }
    Ok(())
}

pub fn extract_package(bytes: &[u8], dest: &Path) -> Result<(), String> {
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(fail(ERR_PACKAGE, "the package is too large"));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| fail(ERR_PACKAGE, format!("not a valid package: {e}")))?;
    if zip.len() > MAX_PACKAGE_FILES {
        return Err(fail(ERR_PACKAGE, "the package has too many files"));
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| fail(ERR_PACKAGE, format!("entry {i}: {e}")))?;
        let name = file
            .name()
            .map_err(|e| fail(ERR_PACKAGE, format!("entry {i}: {e}")))?
            .to_string();
        if file.is_symlink() {
            return Err(fail(ERR_PACKAGE, format!("{name} is a link")));
        }
        let rel = crate::core::catalog::fetch::safe_rel(name.trim_end_matches('/'))
            .map_err(|_| fail(ERR_PACKAGE, format!("unsafe path in the package: {name}")))?;
        if file.is_dir() {
            continue;
        }
        if !seen.insert(rel.to_string_lossy().to_lowercase()) {
            return Err(fail(ERR_PACKAGE, format!("{name} appears twice")));
        }
        let target = dest.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut body = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|e| fail(ERR_PACKAGE, format!("{name}: {e}")))?;
        if body.len() as u64 > MAX_FILE_BYTES {
            return Err(fail(ERR_PACKAGE, format!("{name} is too large")));
        }
        total += body.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(fail(ERR_PACKAGE, "the package is too large when unpacked"));
        }
        std::fs::write(&target, &body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn hidden_dir(root: &Path, prefix: &str) -> PathBuf {
    root.join(format!(".{prefix}-{}", uuid::Uuid::new_v4().simple()))
}

fn swap_into_place(root: &Path, staging: &Path, id: &str) -> Result<(), String> {
    let _guard = SWAP.lock().unwrap_or_else(|p| p.into_inner());
    let dest = root.join(id);
    let previous = std::fs::symlink_metadata(&dest).is_ok();
    let old = hidden_dir(root, "old");
    if previous {
        std::fs::rename(&dest, &old).map_err(|e| e.to_string())?;
    }
    if let Err(e) = std::fs::rename(staging, &dest) {
        if previous {
            let _ = std::fs::rename(&old, &dest);
        }
        return Err(e.to_string());
    }
    if previous && std::fs::remove_dir_all(&old).is_err() {
        let _ = std::fs::remove_file(&old);
    }
    Ok(())
}

pub fn install_package_into(
    root: &Path,
    bytes: &[u8],
    expected: Option<&CatalogEntry>,
) -> Result<Manifest, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let staging = hidden_dir(root, "staging");
    let result = (|| {
        extract_package(bytes, &staging)?;
        let m = connector::load(&staging).map_err(|e| {
            if e == OPTED_OUT_ERROR {
                e
            } else {
                fail(ERR_PACKAGE, e)
            }
        })?;
        if let Some(entry) = expected {
            manifest_matches_entry(&m, entry)?;
        }
        swap_into_place(root, &staging, &m.id)?;
        Ok(m)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

pub fn merge(catalog: &[CatalogEntry], installed: &[Connector]) -> Vec<MarketEntry> {
    let compatible =
        |min: &str| connector::check_min_omniget(min, connector::app_version()).is_ok();
    let mut out: Vec<MarketEntry> = catalog
        .iter()
        .map(|e| {
            let have = installed.iter().find(|c| c.manifest.id == e.id);
            let state = match have {
                None => "not_installed",
                Some(c) if !c.dev && connector::is_newer(&e.version, &c.manifest.version) => {
                    "update_available"
                }
                Some(_) => "installed",
            };
            MarketEntry {
                entry: e.clone(),
                state,
                installed_version: have.map(|c| c.manifest.version.clone()),
                dev: have.is_some_and(|c| c.dev),
                in_catalog: true,
                compatible: compatible(&e.min_omniget),
                has_options: have.is_some_and(|c| !c.manifest.options.is_empty()),
            }
        })
        .collect();
    for c in installed {
        if catalog.iter().any(|e| e.id == c.manifest.id) {
            continue;
        }
        let m = &c.manifest;
        out.push(MarketEntry {
            entry: CatalogEntry {
                id: m.id.clone(),
                name: m.name.clone(),
                description: m.description.clone(),
                description_i18n: BTreeMap::new(),
                version: m.version.clone(),
                author: m.author.clone(),
                matches: m.matches.clone(),
                domains: m.domains.clone(),
                login: m.login_url().map(|url| LoginInfo { url }),
                min_omniget: m.min_omniget.clone(),
                sha256: String::new(),
                package: String::new(),
                category: m.category.clone(),
                permissions: m.permissions.clone(),
            },
            state: "installed",
            installed_version: Some(m.version.clone()),
            dev: c.dev,
            in_catalog: false,
            compatible: true,
            has_options: !m.options.is_empty(),
        });
    }
    out
}

pub fn catalog_match(catalog: &[CatalogEntry], url: &str) -> Option<CatalogEntry> {
    let candidates = catalog
        .iter()
        .filter_map(|e| {
            let strength = connector::match_strength(&e.matches, &e.domains, url)?;
            Some((false, strength, e.id.clone(), e.clone()))
        })
        .collect();
    connector::pick_best(candidates)
}

fn cache_dir() -> PathBuf {
    data_dir().join("connector-catalog")
}

fn read_cache() -> Option<(Catalog, Vec<String>, u64)> {
    let dir = cache_dir();
    let bytes = std::fs::read(dir.join("catalog.json")).ok()?;
    let signature = std::fs::read_to_string(dir.join("catalog.json.sig")).ok()?;
    let (catalog, rejected) = verify_catalog(&bytes, &signature, UPDATER_PUBKEY).ok()?;
    let at = std::fs::metadata(dir.join("catalog.json"))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Some((catalog, rejected, at))
}

fn write_cache(bytes: &[u8], signature: &str) {
    let dir = cache_dir();
    let done = write_atomic(&dir.join("catalog.json.sig"), signature.as_bytes())
        .and_then(|_| write_atomic(&dir.join("catalog.json"), bytes));
    if let Err(e) = done {
        tracing::warn!("[global-dl] could not save the helper catalog: {e}");
    }
}

fn is_fresh(fetched_at: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    now.saturating_sub(fetched_at) < CACHE_FRESH.as_secs()
}

async fn get_capped(
    url: &str,
    cap: usize,
    mut on_progress: impl FnMut(usize, Option<u64>),
) -> Result<Vec<u8>, String> {
    let mut resp = crate::core::catalog::http()
        .get(url)
        .send()
        .await
        .map_err(|e| fail(ERR_NETWORK, format!("{url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(fail(ERR_NETWORK, format!("{url}: HTTP {}", resp.status())));
    }
    let total = resp.content_length();
    if total.is_some_and(|t| t > cap as u64) {
        return Err(fail(ERR_PACKAGE, format!("{url} is too large")));
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| fail(ERR_NETWORK, format!("{url}: {e}")))?
    {
        if out.len() + chunk.len() > cap {
            return Err(fail(ERR_PACKAGE, format!("{url} is too large")));
        }
        out.extend_from_slice(&chunk);
        on_progress(out.len(), total);
    }
    Ok(out)
}

pub fn pointer_target(bytes: &[u8]) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let https = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_str())
            .filter(|u| u.starts_with("https://"))
            .map(str::to_string)
    };
    Some((https("catalog")?, https("signature")?))
}

async fn fetch_remote() -> Result<(Catalog, Vec<String>, Vec<u8>, String), String> {
    let first = get_capped(CATALOG_URL, MAX_CATALOG_BYTES, |_, _| {}).await?;
    let (bytes, sig_url) = match pointer_target(&first) {
        Some((catalog_url, sig_url)) => (
            get_capped(&catalog_url, MAX_CATALOG_BYTES, |_, _| {}).await?,
            sig_url,
        ),
        None => (first, format!("{CATALOG_URL}.sig")),
    };
    let sig = get_capped(&sig_url, MAX_SIGNATURE_BYTES, |_, _| {}).await?;
    let signature = String::from_utf8(sig)
        .map_err(|e| fail(ERR_SIGNATURE, format!("the signature is not text: {e}")))?;
    let (catalog, rejected) = verify_catalog(&bytes, &signature, UPDATER_PUBKEY)?;
    Ok((catalog, rejected, bytes, signature))
}

pub fn refuses_rollback(new_generated_at: u64, cached_generated_at: u64) -> bool {
    new_generated_at < cached_generated_at
}

async fn current_catalog(
    refresh: bool,
) -> (
    Option<Catalog>,
    Vec<String>,
    &'static str,
    Option<u64>,
    Option<String>,
) {
    let cached = read_cache();
    if !refresh {
        if let Some((catalog, rejected, at)) = &cached {
            if is_fresh(*at) {
                return (
                    Some(catalog.clone()),
                    rejected.clone(),
                    "fresh",
                    Some(*at),
                    None,
                );
            }
        }
    }
    let error = match fetch_remote().await {
        Ok((catalog, rejected, bytes, signature)) => {
            let older = cached
                .as_ref()
                .is_some_and(|(c, _, _)| refuses_rollback(catalog.generated_at, c.generated_at));
            if older {
                fail(
                    ERR_STALE,
                    "the server sent a catalog older than the saved one",
                )
            } else {
                write_cache(&bytes, &signature);
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                return (Some(catalog), rejected, "network", Some(now), None);
            }
        }
        Err(e) => e,
    };
    tracing::warn!("[global-dl] helper catalog: {error}");
    match cached {
        Some((catalog, rejected, at)) => (Some(catalog), rejected, "cache", Some(at), Some(error)),
        None => (None, Vec::new(), "none", None, Some(error)),
    }
}

pub async fn view(refresh: bool) -> MarketView {
    let (catalog, rejected, source, fetched_at, error) = current_catalog(refresh).await;
    let (installed, invalid) = connector::scan();
    let entries = catalog.as_ref().map_or(&[][..], |c| &c.connectors);
    MarketView {
        entries: merge(entries, &installed),
        invalid,
        source,
        fetched_at,
        error,
        rejected,
    }
}

fn emit_progress(id: &str, stage: &str, received: usize, total: Option<u64>) {
    super::emit(json!({
        "kind": "connector_install",
        "id": id,
        "stage": stage,
        "received": received,
        "total": total,
    }));
}

pub async fn install_from_catalog(id: &str) -> Result<Manifest, String> {
    connector::validate_id(id)?;
    let (catalog, _, _, _, error) = current_catalog(false).await;
    let catalog = catalog.ok_or_else(|| {
        error.unwrap_or_else(|| fail(ERR_NETWORK, "the helper list is not available"))
    })?;
    let entry = catalog
        .connectors
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| fail(ERR_NOT_FOUND, format!("{id} is not in the helper list")))?;
    connector::check_min_omniget(&entry.min_omniget, connector::app_version())
        .map_err(|e| fail(ERR_PACKAGE, e))?;
    emit_progress(id, "download", 0, None);
    let id_owned = id.to_string();
    let bytes = get_capped(&entry.package, MAX_PACKAGE_BYTES, |received, total| {
        emit_progress(&id_owned, "download", received, total);
    })
    .await
    .inspect_err(|_| emit_progress(id, "failed", 0, None))?;
    emit_progress(id, "verify", bytes.len(), None);
    let outcome = verify_package_hash(&bytes, &entry.sha256)
        .and_then(|_| install_package_into(&connector::installed_dir(), &bytes, Some(entry)));
    emit_progress(
        id,
        if outcome.is_ok() { "done" } else { "failed" },
        bytes.len(),
        None,
    );
    outcome
}

pub async fn update_from_catalog(id: &str) -> Result<Manifest, String> {
    connector::validate_id(id)?;
    if connector::find_by_id(id).is_none() {
        return Err(fail(ERR_NOT_FOUND, format!("{id} is not installed")));
    }
    install_from_catalog(id).await
}

fn read_package_file(path: &Path) -> Result<Vec<u8>, String> {
    let is_package = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXT));
    if !is_package {
        return Err(fail(ERR_PACKAGE, format!("choose a .{PACKAGE_EXT} file")));
    }
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_PACKAGE_BYTES as u64 {
        return Err(fail(ERR_PACKAGE, "the package is too large"));
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

pub fn inspect_file(path: &Path) -> Result<PackageSummary, String> {
    let bytes = read_package_file(path)?;
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    extract_package(&bytes, tmp.path())?;
    let m = connector::load(tmp.path()).map_err(|e| {
        if e == OPTED_OUT_ERROR {
            e
        } else {
            fail(ERR_PACKAGE, e)
        }
    })?;
    let replaces_version = connector::find_by_id(&m.id).map(|c| c.manifest.version);
    Ok(PackageSummary {
        id: m.id,
        name: m.name,
        version: m.version,
        author: m.author,
        description: m.description,
        domains: m.domains,
        permissions: m.permissions,
        replaces_version,
    })
}

pub fn install_file(path: &Path) -> Result<Manifest, String> {
    let bytes = read_package_file(path)?;
    install_package_into(&connector::installed_dir(), &bytes, None)
}

pub async fn lookup(url: &str) -> UrlLookup {
    let raw = url.trim();
    let normalized = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    if is_opted_out(&normalized) {
        return UrlLookup {
            opted_out: true,
            ..Default::default()
        };
    }
    if !normalized.starts_with("http://") && !normalized.starts_with("https://") {
        return UrlLookup::default();
    }
    let installed = connector::find_for(&normalized).map(|c| InstalledMatch {
        login_url: c.manifest.login_url(),
        id: c.manifest.id,
        name: c.manifest.name,
        dev: c.dev,
    });
    let (catalog, _, _, _, _) = current_catalog(false).await;
    let hit = catalog
        .as_ref()
        .and_then(|c| catalog_match(&c.connectors, &normalized));
    let catalog = hit.and_then(|entry| {
        let list = [entry];
        merge(&list, &connector::list())
            .into_iter()
            .find(|e| e.in_catalog)
    });
    UrlLookup {
        opted_out: false,
        installed,
        catalog,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    // WHY: throwaway key from `tauri signer generate`, proves the real signer output verifies.
    const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEU1RUNBOUY0RTcyRDBCOTQKUldTVUN5M245S25zNVFDcFliWmJFODNSYVpZbUxnOXo4U0xEWHlGTHR1T29WZnRKemg4R2s1S1QK";
    const SIGNED_TEXT: &str = "{\"generated_at\":1,\"connectors\":[]}\n";
    const SIGNED_TEXT_SIG: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTVUN5M245S25zNVNjV2ZETU9hVitad1RoWk9ZaEd5cldTbjBBb0FaaU9UMDROTFhSVXM5VzZ6MzNqRS9wOFFjb0RZSVF1MXNQR0d2KzZZYkR5Rkpnc0g1SmVmelpWVUF3PQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzODg0Nzg5CWZpbGU6c2lnbmVkLmpzb24KR0JYTzYyOWErc05rb24xbXlJOWx5Lzk4UG1NRFNlWTRoVDNpNUtFWXBsM2hzWk5XR3piSUEraEZBL3RLZHozQ3RGRy9TT0ZtOWNDaUxWbWxPdTFWQkE9PQo=";

    fn manifest_json(id: &str, version: &str, domains: &[&str]) -> String {
        let host = domains.first().copied().unwrap_or("example.com");
        json!({
            "id": id,
            "name": id,
            "version": version,
            "domains": domains,
            "match": [format!("https://{host}/**")],
        })
        .to_string()
    }

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn package(id: &str, version: &str, domains: &[&str]) -> Vec<u8> {
        zip_of(&[
            (
                "manifest.json",
                manifest_json(id, version, domains).as_bytes(),
            ),
            (
                "connector.js",
                b"function resolve(){return {title:'x',children:[]}}",
            ),
            ("GUIDE.md", b"# guide"),
        ])
    }

    fn entry_for(id: &str, version: &str, domains: &[&str], bytes: &[u8]) -> CatalogEntry {
        let host = domains.first().copied().unwrap_or("example.com");
        CatalogEntry {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            description_i18n: BTreeMap::new(),
            version: version.into(),
            author: "a".into(),
            matches: vec![format!("https://{host}/**")],
            domains: domains.iter().map(|d| d.to_string()).collect(),
            login: None,
            min_omniget: String::new(),
            sha256: sha256_hex(bytes),
            package: "https://getomniget.com/connectors/x.omniconnector".into(),
            category: String::new(),
            permissions: Vec::new(),
        }
    }

    #[test]
    fn extraction_refuses_path_traversal() {
        for evil in [
            "../evil.js",
            "a/../../evil.js",
            "/etc/evil.js",
            "a\\..\\evil.js",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("in");
            let bytes = zip_of(&[(evil, b"x"), ("manifest.json", b"{}")]);
            let err = extract_package(&bytes, &dest).unwrap_err();
            assert!(err.starts_with("package:"), "{evil}: {err}");
            assert!(!tmp.path().join("evil.js").exists(), "{evil}");
        }
    }

    #[test]
    fn extraction_refuses_symlinks() {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        let bytes = w.finish().unwrap().into_inner();
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_package(&bytes, tmp.path()).unwrap_err();
        assert!(err.contains("link"), "{err}");
    }

    #[test]
    fn extraction_stops_at_the_file_size_cap() {
        let big = vec![0u8; (MAX_FILE_BYTES + 1) as usize];
        let bytes = zip_of(&[("connector.js", &big)]);
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_package(&bytes, tmp.path()).unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn extraction_refuses_too_many_files_and_duplicates() {
        let names: Vec<String> = (0..=MAX_PACKAGE_FILES).map(|i| format!("{i}.js")).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        let tmp = tempfile::tempdir().unwrap();
        assert!(extract_package(&zip_of(&files), tmp.path())
            .unwrap_err()
            .contains("too many"));
        let dup = zip_of(&[("A.js", b"1"), ("a.js", b"2")]);
        assert!(extract_package(&dup, tmp.path())
            .unwrap_err()
            .contains("twice"));
    }

    #[test]
    fn garbage_is_not_a_package() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(extract_package(b"not a zip", tmp.path())
            .unwrap_err()
            .starts_with("package:"));
    }

    #[test]
    fn sha_mismatch_is_refused() {
        let bytes = package("a", "1.0.0", &["example.com"]);
        assert!(verify_package_hash(&bytes, &sha256_hex(&bytes)).is_ok());
        let err = verify_package_hash(&bytes, &sha256_hex(b"other")).unwrap_err();
        assert!(err.starts_with("hash:"), "{err}");
    }

    #[test]
    fn signature_made_by_the_tauri_signer_is_accepted() {
        verify_signature(SIGNED_TEXT.as_bytes(), SIGNED_TEXT_SIG, TEST_PUBKEY).unwrap();
    }

    #[test]
    fn tampered_catalog_is_refused() {
        let tampered = SIGNED_TEXT.replace("[]", "[1]");
        let err = verify_signature(tampered.as_bytes(), SIGNED_TEXT_SIG, TEST_PUBKEY).unwrap_err();
        assert!(err.starts_with("signature:"), "{err}");
    }

    #[test]
    fn signature_from_another_key_is_refused() {
        let err =
            verify_signature(SIGNED_TEXT.as_bytes(), SIGNED_TEXT_SIG, UPDATER_PUBKEY).unwrap_err();
        assert!(err.starts_with("signature:"), "{err}");
    }

    #[test]
    fn garbage_signature_is_refused() {
        for bad in ["", "%%%", "bm90IGEgc2lnbmF0dXJl"] {
            let err = verify_signature(b"x", bad, TEST_PUBKEY).unwrap_err();
            assert!(err.starts_with("signature:"), "{err}");
        }
    }

    #[test]
    fn embedded_key_is_the_updater_key_of_tauri_conf() {
        let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tauri.conf.json");
        let Ok(text) = std::fs::read_to_string(conf) else {
            return;
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["plugins"]["updater"]["pubkey"], UPDATER_PUBKEY);
    }

    #[test]
    fn manifest_with_another_id_or_domains_than_the_catalog_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = package("a", "1.0.0", &["example.com"]);
        let other_id = entry_for("b", "1.0.0", &["example.com"], &bytes);
        let err = install_package_into(tmp.path(), &bytes, Some(&other_id)).unwrap_err();
        assert!(err.starts_with("mismatch:"), "{err}");
        let other_domain = {
            let mut e = entry_for("a", "1.0.0", &["example.com"], &bytes);
            e.domains = vec!["other.test".into()];
            e
        };
        let err = install_package_into(tmp.path(), &bytes, Some(&other_domain)).unwrap_err();
        assert!(err.starts_with("mismatch:"), "{err}");
        let mut other_pattern = entry_for("a", "1.0.0", &["example.com"], &bytes);
        other_pattern.matches = vec!["https://example.com/other/**".into()];
        let err = install_package_into(tmp.path(), &bytes, Some(&other_pattern)).unwrap_err();
        assert!(err.starts_with("mismatch:"), "{err}");
        assert!(!tmp.path().join("a").exists());
    }

    #[test]
    fn opted_out_package_is_refused_with_the_existing_message() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = package("k", "1.0.0", &["kiwify.com.br"]);
        let err = install_package_into(tmp.path(), &bytes, None).unwrap_err();
        assert_eq!(err, OPTED_OUT_ERROR);
        assert!(!tmp.path().join("k").exists());
    }

    #[test]
    fn opted_out_catalog_entries_are_dropped_and_reported() {
        let ok = entry_for("a", "1.0.0", &["example.com"], b"x");
        let kiwify = entry_for("k", "1.0.0", &["kiwify.com"], b"x");
        let bytes = serde_json::to_vec(&json!({ "connectors": [ok, kiwify] })).unwrap();
        let (catalog, rejected) = parse_catalog(&bytes).unwrap();
        assert_eq!(catalog.connectors.len(), 1);
        assert_eq!(catalog.connectors[0].id, "a");
        assert!(rejected[0].contains(OPTED_OUT_ERROR), "{rejected:?}");
    }

    fn catalog_with(description_i18n: Option<Value>) -> Vec<u8> {
        let mut entry = json!({
            "id": "a",
            "name": "A",
            "description": "English text.",
            "version": "1.0.0",
            "match": ["https://example.com/**"],
            "domains": ["example.com"],
            "sha256": "0".repeat(64),
            "package": "https://getomniget.com/connectors/a.omniconnector",
        });
        if let Some(map) = description_i18n {
            entry["description_i18n"] = map;
        }
        serde_json::to_vec(&json!({ "generated_at": 1, "connectors": [entry] })).unwrap()
    }

    fn only_entry(bytes: &[u8]) -> CatalogEntry {
        let (catalog, rejected) = parse_catalog(bytes).unwrap();
        assert!(rejected.is_empty(), "{rejected:?}");
        assert_eq!(catalog.connectors.len(), 1);
        catalog.connectors[0].clone()
    }

    #[test]
    fn a_catalog_without_translated_descriptions_still_loads() {
        let e = only_entry(&catalog_with(None));
        assert_eq!(e.description, "English text.");
        assert!(e.description_i18n.is_empty());
        assert!(json!(e).get("description_i18n").is_none());
    }

    #[test]
    fn translated_descriptions_reach_the_market_entry_next_to_the_english_one() {
        let e = only_entry(&catalog_with(Some(
            json!({ "ru": "Русский текст.", "zh-TW": "繁體" }),
        )));
        assert_eq!(e.description_i18n.len(), 2);
        let sent = json!(merge(&[e], &[]));
        assert_eq!(sent[0]["description"], "English text.");
        assert_eq!(sent[0]["description_i18n"]["ru"], "Русский текст.");
        assert_eq!(sent[0]["description_i18n"]["zh-TW"], "繁體");
    }

    #[test]
    fn translated_descriptions_that_are_not_text_are_dropped() {
        let e = only_entry(&catalog_with(Some(json!({
            "ru": 7,
            "pt": null,
            "es": ["x"],
            "fr": { "nested": "x" },
            "it": "   ",
            "ja": "ok",
        }))));
        assert_eq!(
            e.description_i18n,
            BTreeMap::from([("ja".to_string(), "ok".to_string())])
        );
        for not_a_map in [json!("ru"), json!(7), json!(null), json!(["ru"])] {
            let e = only_entry(&catalog_with(Some(not_a_map)));
            assert_eq!(e.description, "English text.");
            assert!(e.description_i18n.is_empty());
        }
    }

    #[test]
    fn translated_descriptions_over_the_limits_are_dropped() {
        let long = "я".repeat(MAX_DESCRIPTION_CHARS + 1);
        let fits = "я".repeat(MAX_DESCRIPTION_CHARS);
        let e = only_entry(&catalog_with(Some(json!({
            "ru": long,
            "pt": fits,
            "": "empty locale",
            "a-locale-code-far-too-long": "x",
            "../up": "x",
            "<b>": "x",
        }))));
        assert_eq!(e.description_i18n.keys().collect::<Vec<_>>(), vec!["pt"]);
        let many: serde_json::Map<String, Value> = (0..MAX_DESCRIPTION_LOCALES + 10)
            .map(|i| (format!("l{i}"), json!("x")))
            .collect();
        let e = only_entry(&catalog_with(Some(Value::Object(many))));
        assert_eq!(e.description_i18n.len(), MAX_DESCRIPTION_LOCALES);
    }

    #[test]
    fn a_pointer_names_where_the_catalog_and_its_signature_live() {
        let p = br#"{"catalog":"https://files.example/a.json","signature":"https://files.example/a.sig"}"#;
        assert_eq!(
            pointer_target(p),
            Some((
                "https://files.example/a.json".to_string(),
                "https://files.example/a.sig".to_string()
            ))
        );
    }

    #[test]
    fn a_catalog_or_a_pointer_without_https_is_not_a_pointer() {
        assert_eq!(pointer_target(SIGNED_TEXT.as_bytes()), None);
        let plain = br#"{"catalog":"http://files.example/a.json","signature":"https://files.example/a.sig"}"#;
        assert_eq!(pointer_target(plain), None);
        assert_eq!(pointer_target(b"not json"), None);
    }

    #[test]
    fn catalog_entries_without_https_package_or_real_hash_are_dropped() {
        let mut http = entry_for("a", "1.0.0", &["example.com"], b"x");
        http.package = "http://getomniget.com/a.omniconnector".into();
        let mut no_hash = entry_for("b", "1.0.0", &["example.com"], b"x");
        no_hash.sha256 = "abc".into();
        let bytes = serde_json::to_vec(&json!({ "connectors": [http, no_hash] })).unwrap();
        let (catalog, rejected) = parse_catalog(&bytes).unwrap();
        assert!(catalog.connectors.is_empty());
        assert_eq!(rejected.len(), 2);
    }

    #[test]
    fn install_copies_the_files_and_update_replaces_them_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let v1 = zip_of(&[
            (
                "manifest.json",
                manifest_json("a", "1.0.0", &["example.com"]).as_bytes(),
            ),
            ("connector.js", b"function resolve(){}"),
            ("old.txt", b"gone after the update"),
        ]);
        install_package_into(tmp.path(), &v1, None).unwrap();
        assert!(tmp.path().join("a/old.txt").is_file());
        let v2 = package("a", "2.0.0", &["example.com"]);
        let m = install_package_into(tmp.path(), &v2, None).unwrap();
        assert_eq!(m.version, "2.0.0");
        assert!(tmp.path().join("a/GUIDE.md").is_file());
        assert!(!tmp.path().join("a/old.txt").exists());
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "no staging or backup folder is left behind");
    }

    #[test]
    fn failed_update_leaves_the_installed_version_alone() {
        let tmp = tempfile::tempdir().unwrap();
        install_package_into(tmp.path(), &package("a", "1.0.0", &["example.com"]), None).unwrap();
        let broken = zip_of(&[(
            "manifest.json",
            manifest_json("a", "2.0.0", &["example.com"]).as_bytes(),
        )]);
        assert!(install_package_into(tmp.path(), &broken, None).is_err());
        let text = std::fs::read_to_string(tmp.path().join("a/manifest.json")).unwrap();
        assert!(text.contains("1.0.0"));
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn package_with_a_traversal_id_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = package("../up", "1.0.0", &["example.com"]);
        assert!(install_package_into(tmp.path(), &bytes, None).is_err());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
    }

    #[test]
    fn merge_reports_not_installed_installed_and_update_available() {
        let e = |id: &str, v: &str| entry_for(id, v, &["example.com"], b"x");
        let installed_at = |id: &str, v: &str, dev: bool| Connector {
            manifest: serde_json::from_str(&manifest_json(id, v, &["example.com"])).unwrap(),
            dir: PathBuf::from("/x"),
            dev,
        };
        let catalog = [
            e("a", "1.0.0"),
            e("b", "1.1.0"),
            e("c", "2.0.0"),
            e("d", "2.0.0"),
        ];
        let installed = [
            installed_at("b", "1.1.0", false),
            installed_at("c", "1.9.0", false),
            installed_at("d", "1.0.0", true),
            installed_at("z", "0.1.0", false),
        ];
        let got: Vec<(String, &str, bool)> = merge(&catalog, &installed)
            .into_iter()
            .map(|m| (m.entry.id, m.state, m.in_catalog))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a".to_string(), "not_installed", true),
                ("b".to_string(), "installed", true),
                ("c".to_string(), "update_available", true),
                ("d".to_string(), "installed", true),
                ("z".to_string(), "installed", false),
            ]
        );
    }

    #[test]
    fn catalog_lookup_finds_a_helper_that_is_not_installed() {
        let mut a = entry_for("a", "1.0.0", &["example.com"], b"x");
        a.matches = vec!["https://example.com/**".into()];
        let mut b = entry_for("b", "1.0.0", &["example.com"], b"x");
        b.matches = vec!["https://example.com/course/**".into()];
        let list = [a, b];
        let hit = catalog_match(&list, "https://example.com/course/1?x=2").unwrap();
        assert_eq!(hit.id, "b");
        assert!(catalog_match(&list, "https://other.test/course/1").is_none());
    }

    #[test]
    fn an_older_catalog_than_the_saved_one_is_refused() {
        assert!(refuses_rollback(5, 6));
        assert!(!refuses_rollback(6, 6));
        assert!(!refuses_rollback(7, 6));
    }

    #[test]
    #[ignore = "needs the output of scripts/connectors-build.mjs and its public key"]
    fn built_catalog_verifies_and_every_package_installs() {
        let dir = PathBuf::from(std::env::var("OMNI_CATALOG_DIR").unwrap());
        let pubkey = std::env::var("OMNI_PUBKEY").unwrap();
        let bytes = std::fs::read(dir.join("catalog.json")).unwrap();
        let sig = std::fs::read_to_string(dir.join("catalog.json.sig")).unwrap();
        let (catalog, rejected) = verify_catalog(&bytes, &sig, &pubkey).unwrap();
        assert!(rejected.is_empty(), "{rejected:?}");
        assert!(!catalog.connectors.is_empty());
        let tmp = tempfile::tempdir().unwrap();
        for e in &catalog.connectors {
            let file = format!("{}-{}.{PACKAGE_EXT}", e.id, e.version);
            let pkg = std::fs::read(dir.join("packages").join(file)).unwrap();
            verify_package_hash(&pkg, &e.sha256).unwrap();
            install_package_into(tmp.path(), &pkg, Some(e)).unwrap();
            assert!(tmp.path().join(&e.id).join("connector.js").is_file());
        }
    }
}
