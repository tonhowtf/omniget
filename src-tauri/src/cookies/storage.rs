//! On-disk layout for the cookie manager.
//!
//! ```text
//! <app_data>/
//!   cookies/
//!     <root_domain>/
//!       _default.txt        # primary account (Netscape, yt-dlp consumable)
//!       <slug>.txt          # additional accounts (multi-account UX, CK-5)
//!     _meta.json            # registry: alias, source, captured_at, platform_kind
//!     _trash/               # 24h-retention bin for `cookies:clear`
//! ```
//!
//! The format inside each file is Netscape — same as the legacy
//! `chrome-extension-cookies.txt` so yt-dlp consumes it unchanged.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::platform::{root_domain_of, PlatformKind};
use crate::extension_storage::ExtensionCookie;

const META_FILE: &str = "_meta.json";
const DEFAULT_SLUG: &str = "_default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    pub slug: String,
    pub alias: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub source_label: Option<String>,
    pub captured_at_ms: i64,
    pub cookie_count: usize,
    #[serde(default)]
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BucketEntry {
    pub platform_kind: String,
    pub accounts: Vec<AccountEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CookieRegistry {
    #[serde(default)]
    pub buckets: BTreeMap<String, BucketEntry>,
}

pub fn cookies_root() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cookies")
}

pub fn bucket_dir(domain: &str) -> PathBuf {
    cookies_root().join(safe_domain_segment(domain))
}

pub fn account_file(domain: &str, slug: &str) -> PathBuf {
    bucket_dir(domain).join(format!("{}.txt", safe_slug_segment(slug)))
}

pub fn meta_path() -> PathBuf {
    cookies_root().join(META_FILE)
}

pub fn trash_dir() -> PathBuf {
    cookies_root().join("_trash")
}

fn safe_domain_segment(domain: &str) -> String {
    let h = domain.trim_start_matches('.').to_lowercase();
    h.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn safe_slug_segment(slug: &str) -> String {
    let lowered = slug.to_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        DEFAULT_SLUG.to_string()
    } else {
        cleaned
    }
}

pub fn load_registry() -> CookieRegistry {
    let path = meta_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return CookieRegistry::default(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_registry(registry: &CookieRegistry) -> anyhow::Result<()> {
    let path = meta_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_string_pretty(registry)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serialized)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn format_cookie_line(c: &ExtensionCookie, session_ttl: u64) -> String {
    let raw_domain = sanitize_field(&c.domain);
    let path_field = sanitize_field(&c.path);
    let name = sanitize_field(&c.name);
    let value = sanitize_field(&c.value);
    let http_only_prefix = if c.http_only { "#HttpOnly_" } else { "" };
    let is_host_only = c.host_only.unwrap_or_else(|| !raw_domain.starts_with('.'));
    let (domain, include_subdomains) = if is_host_only {
        let stripped = raw_domain
            .strip_prefix('.')
            .unwrap_or(&raw_domain)
            .to_string();
        (stripped, "FALSE")
    } else if raw_domain.starts_with('.') {
        (raw_domain.clone(), "TRUE")
    } else {
        (format!(".{}", raw_domain), "TRUE")
    };
    let secure = if c.secure { "TRUE" } else { "FALSE" };
    let expires = if c.expires == 0 {
        session_ttl
    } else {
        c.expires as u64
    };
    format!(
        "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        http_only_prefix, domain, include_subdomains, path_field, secure, expires, name, value,
    )
}

fn sanitize_field(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
        .collect()
}

fn site_source_url(root: &str) -> Option<String> {
    let root = root.trim_start_matches('.').to_lowercase();
    if root.is_empty() {
        return None;
    }
    let host = if root == "bilibili.com" {
        "www.bilibili.com"
    } else {
        root.as_str()
    };
    Some(format!("https://{}/", host))
}

fn normalize_source_url(source_url: Option<&str>, root: &str) -> Option<String> {
    let root = root_domain_of(root);
    if root.is_empty() {
        return None;
    }
    if let Some(parsed) = source_url.and_then(|u| url::Url::parse(u).ok()) {
        if let Some(host) = parsed.host_str() {
            if root_domain_of(host) == root {
                return site_source_url(&root);
            }
        }
    }
    site_source_url(&root)
}

pub struct IngestSource {
    pub source_url: Option<String>,
    pub source_label: String,
    pub alias_hint: Option<String>,
}

pub fn write_account_file(
    domain: &str,
    slug: &str,
    cookies: &[ExtensionCookie],
) -> anyhow::Result<usize> {
    let path = account_file(domain, slug);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let session_ttl = now + 86400;
    let mut content = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        content.push_str(&format_cookie_line(c, session_ttl));
    }
    fs::write(&path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(cookies.len())
}

pub fn ingest_batch(
    cookies: &[ExtensionCookie],
    source: IngestSource,
) -> anyhow::Result<Vec<(String, usize)>> {
    let mut by_root: BTreeMap<String, Vec<ExtensionCookie>> = BTreeMap::new();
    for c in cookies {
        let root = root_domain_of(&c.domain);
        if root.is_empty() {
            continue;
        }
        by_root.entry(root).or_default().push(c.clone());
    }

    let mut registry = load_registry();
    let mut written: Vec<(String, usize)> = Vec::new();
    let now = current_unix_ms();

    for (root, group) in by_root.iter() {
        let count = write_account_file(root, DEFAULT_SLUG, group)?;
        let platform = PlatformKind::from_domain(root);
        let source_url = normalize_source_url(source.source_url.as_deref(), root);
        let bucket = registry
            .buckets
            .entry(root.clone())
            .or_insert_with(|| BucketEntry {
                platform_kind: platform.as_str().to_string(),
                accounts: Vec::new(),
            });
        bucket.platform_kind = platform.as_str().to_string();

        let alias_default = format!("{} · {}", platform_display(platform), human_date(now),);
        let new_alias = source.alias_hint.clone().unwrap_or(alias_default);

        if let Some(existing) = bucket.accounts.iter_mut().find(|a| a.slug == DEFAULT_SLUG) {
            existing.captured_at_ms = now;
            existing.cookie_count = count;
            if let Some(ref u) = source_url {
                existing.source_url = Some(u.clone());
            }
            existing.source_label = Some(source.source_label.clone());
            if source.alias_hint.is_some() {
                existing.alias = new_alias;
            }
        } else {
            bucket.accounts.push(AccountEntry {
                slug: DEFAULT_SLUG.to_string(),
                alias: new_alias,
                source_url,
                source_label: Some(source.source_label.clone()),
                captured_at_ms: now,
                cookie_count: count,
                last_used_at_ms: None,
            });
        }

        written.push((root.clone(), count));
    }

    save_registry(&registry)?;
    Ok(written)
}

fn platform_display(p: PlatformKind) -> &'static str {
    match p {
        PlatformKind::Youtube => "YouTube",
        PlatformKind::YoutubeMusic => "YouTube Music",
        PlatformKind::SoundCloud => "SoundCloud",
        PlatformKind::Spotify => "Spotify",
        PlatformKind::Twitch => "Twitch",
        PlatformKind::Instagram => "Instagram",
        PlatformKind::XTwitter => "X",
        PlatformKind::Vimeo => "Vimeo",
        PlatformKind::Tiktok => "TikTok",
        PlatformKind::Bilibili => "Bilibili",
        PlatformKind::Douyin => "Douyin",
        PlatformKind::Reddit => "Reddit",
        PlatformKind::Pinterest => "Pinterest",
        PlatformKind::Bluesky => "Bluesky",
        PlatformKind::Generic => "Site",
    }
}

fn human_date(ms: i64) -> String {
    let secs = (ms / 1000) as u64;
    let days = secs / 86400;
    let years = 1970 + (days / 365);
    let day_of_year = days % 365;
    let month = (day_of_year / 30).min(11);
    let day = (day_of_year % 30) + 1;
    format!("{:04}-{:02}-{:02}", years, month + 1, day)
}

pub fn read_account_file(domain: &str, slug: &str) -> anyhow::Result<String> {
    let path = account_file(domain, slug);
    Ok(fs::read_to_string(path)?)
}

pub fn move_to_trash(domain: &str, slug: &str) -> anyhow::Result<()> {
    let src = account_file(domain, slug);
    if !src.exists() {
        return Ok(());
    }
    let trash = trash_dir();
    fs::create_dir_all(&trash)?;
    let stamp = current_unix_ms();
    let dst_name = format!(
        "{}__{}__{}.txt",
        safe_domain_segment(domain),
        safe_slug_segment(slug),
        stamp
    );
    let dst = trash.join(dst_name);
    fs::rename(&src, &dst)?;

    let mut registry = load_registry();
    if let Some(bucket) = registry.buckets.get_mut(domain) {
        bucket.accounts.retain(|a| a.slug != slug);
        if bucket.accounts.is_empty() {
            registry.buckets.remove(domain);
        }
    }
    save_registry(&registry)?;

    let dir = bucket_dir(domain);
    if dir.exists() {
        if let Ok(mut entries) = fs::read_dir(&dir) {
            if entries.next().is_none() {
                let _ = fs::remove_dir(&dir);
            }
        }
    }
    Ok(())
}

pub fn slugify_alias(alias: &str) -> String {
    let lowered = alias.to_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c.is_whitespace() || c == '-' || c == '_' {
                '-'
            } else {
                '-'
            }
        })
        .collect();
    let collapsed: String = cleaned
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() || collapsed.starts_with('_') {
        format!("account-{}", current_unix_ms())
    } else {
        collapsed.chars().take(40).collect()
    }
}

pub fn ingest_to_account(
    domain: &str,
    requested_slug: &str,
    cookies: &[ExtensionCookie],
    source: IngestSource,
) -> anyhow::Result<(String, usize)> {
    let root = root_domain_of(domain);
    if root.is_empty() {
        anyhow::bail!("invalid domain: {domain}");
    }
    let scoped: Vec<ExtensionCookie> = cookies
        .iter()
        .filter(|c| root_domain_of(&c.domain) == root)
        .cloned()
        .collect();
    if scoped.is_empty() {
        anyhow::bail!("no cookies in payload match domain {root}");
    }

    let mut registry = load_registry();
    let bucket = registry.buckets.entry(root.clone()).or_insert_with(|| {
        let platform = PlatformKind::from_domain(&root);
        BucketEntry {
            platform_kind: platform.as_str().to_string(),
            accounts: Vec::new(),
        }
    });

    let mut slug = if requested_slug.trim().is_empty() {
        slugify_alias(source.alias_hint.as_deref().unwrap_or(""))
    } else {
        slugify_alias(requested_slug)
    };
    if slug == DEFAULT_SLUG {
        slug = format!("{}-extra", DEFAULT_SLUG.trim_start_matches('_'));
    }
    let mut candidate = slug.clone();
    let mut suffix = 2;
    while bucket.accounts.iter().any(|a| a.slug == candidate) {
        candidate = format!("{}-{}", slug, suffix);
        suffix += 1;
    }
    let final_slug = candidate;

    let count = write_account_file(&root, &final_slug, &scoped)?;
    let now = current_unix_ms();
    let source_url = normalize_source_url(source.source_url.as_deref(), &root);
    let alias = source.alias_hint.clone().unwrap_or_else(|| {
        format!(
            "{} · {}",
            platform_display(PlatformKind::from_domain(&root)),
            human_date(now)
        )
    });

    bucket.accounts.push(AccountEntry {
        slug: final_slug.clone(),
        alias,
        source_url,
        source_label: Some(source.source_label.clone()),
        captured_at_ms: now,
        cookie_count: count,
        last_used_at_ms: None,
    });

    save_registry(&registry)?;
    Ok((final_slug, count))
}

pub fn rename_account(domain: &str, slug: &str, new_alias: &str) -> anyhow::Result<()> {
    let mut registry = load_registry();
    let bucket = registry
        .buckets
        .get_mut(domain)
        .ok_or_else(|| anyhow::anyhow!("bucket not found: {domain}"))?;
    let account = bucket
        .accounts
        .iter_mut()
        .find(|a| a.slug == slug)
        .ok_or_else(|| anyhow::anyhow!("account not found: {slug}"))?;
    account.alias = new_alias.to_string();
    save_registry(&registry)?;
    Ok(())
}

pub fn account_path_for_consumer(domain: &str, slug: Option<&str>) -> Option<PathBuf> {
    let slug = slug.unwrap_or(DEFAULT_SLUG);
    let path = account_file(domain, slug);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub fn touch_last_used(domain: &str, slug: &str) {
    let mut registry = load_registry();
    let now = current_unix_ms();
    if let Some(bucket) = registry.buckets.get_mut(domain) {
        if let Some(account) = bucket.accounts.iter_mut().find(|a| a.slug == slug) {
            account.last_used_at_ms = Some(now);
            let _ = save_registry(&registry);
        }
    }
}

pub fn migrate_legacy_if_needed() -> anyhow::Result<usize> {
    let legacy = crate::extension_storage::extension_cookie_file_path();
    if !legacy.exists() {
        return Ok(0);
    }
    let raw = fs::read_to_string(&legacy)?;
    let cookies = super::parsers::parse_netscape(&raw)?;
    if cookies.is_empty() {
        return Ok(0);
    }
    let written = ingest_batch(
        &cookies,
        IngestSource {
            source_url: None,
            source_label: "Migrated from legacy single-file storage".to_string(),
            alias_hint: None,
        },
    )?;
    Ok(written.len())
}

pub fn has_been_migrated() -> bool {
    let registry = load_registry();
    !registry.buckets.is_empty()
}

pub fn migrate_path(path: &Path) -> PathBuf {
    cookies_root().join(".migrated").join(
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "legacy".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_cookie(domain: &str, name: &str, value: &str) -> ExtensionCookie {
        ExtensionCookie {
            domain: domain.to_string(),
            http_only: false,
            path: "/".to_string(),
            secure: true,
            expires: 1830000000,
            name: name.to_string(),
            value: value.to_string(),
            host_only: None,
            same_site: None,
        }
    }

    #[test]
    fn safe_domain_strips_unsafe() {
        assert_eq!(safe_domain_segment(".YouTube.Com"), "youtube.com");
        assert_eq!(safe_domain_segment("foo/bar"), "foo_bar");
    }

    #[test]
    fn safe_slug_handles_empty_and_unsafe() {
        assert_eq!(safe_slug_segment(""), DEFAULT_SLUG);
        assert_eq!(safe_slug_segment("My Account / WTF"), "my-account---wtf");
    }

    #[test]
    fn format_cookie_line_matches_netscape() {
        let c = fake_cookie(".youtube.com", "SID", "abc");
        let line = format_cookie_line(&c, 0);
        let parts: Vec<&str> = line.trim().split('\t').collect();
        assert_eq!(parts.len(), 7);
        assert_eq!(parts[0], ".youtube.com");
        assert_eq!(parts[1], "TRUE");
        assert_eq!(parts[5], "SID");
        assert_eq!(parts[6], "abc");
    }

    #[test]
    fn format_cookie_line_host_only_flag() {
        let mut c = fake_cookie("youtube.com", "k", "v");
        c.host_only = Some(true);
        let line = format_cookie_line(&c, 0);
        let parts: Vec<&str> = line.trim().split('\t').collect();
        assert_eq!(parts[0], "youtube.com");
        assert_eq!(parts[1], "FALSE");
    }

    #[test]
    fn normalize_source_url_uses_site_home_for_bilibili_video_url() {
        assert_eq!(
            normalize_source_url(
                Some("https://www.bilibili.com/bangumi/play/ss48239?t=2"),
                "bilibili.com"
            ),
            Some("https://www.bilibili.com/".to_string())
        );
    }

    #[test]
    fn normalize_source_url_falls_back_to_cookie_root() {
        assert_eq!(
            normalize_source_url(Some("not a url"), ".youtube.com"),
            Some("https://youtube.com/".to_string())
        );
    }
}
